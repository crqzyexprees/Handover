use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Deserialize;
use tokio::sync::{mpsc, Mutex};
use tracing::warn;

use crate::state::{AppState, PtySession};

const CONTROL_PREFIX: &str = "__handover_control__:";

#[derive(Deserialize)]
#[serde(tag = "type")]
enum PtyControl {
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
}

/// Mirrors the original Python backend: one WebSocket ↔ one PTY ↔ one shell.
pub async fn handle_pty_socket(socket: WebSocket, instance_id: String, state: Arc<AppState>) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    async fn send_error(
        ws_tx: &mut futures::stream::SplitSink<WebSocket, Message>,
        message: &str,
    ) {
        let _ = ws_tx
            .send(Message::Text(Utf8Bytes::from(format!(
                "\r\n\x1b[31m[handover] {message}\x1b[0m\r\n"
            ))))
            .await;
    }

    let (sandbox_mode, container_id, cwd) = {
        let instances = state.instances.read().await;
        let instance = match instances.get(&instance_id) {
            Some(i) => i.clone(),
            None => {
                send_error(
                    &mut ws_tx,
                    "Instance not found. Close this terminal and open a new one.",
                )
                .await;
                return;
            }
        };
        let mode = instance
            .get("sandbox_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("docker")
            .to_string();
        let cid = instance
            .get("container_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let project_path = if let Some(pid) = instance.get("project_id").and_then(|v| v.as_str()) {
            let projects = state.projects.read().await;
            projects
                .get(pid)
                .and_then(|p| p.get("path").and_then(|v| v.as_str()).map(String::from))
        } else {
            None
        };
        let cwd = project_path
            .map(|p| PathBuf::from(crate::platform::normalize_storage_path(&p)))
            .filter(|p| p.is_dir())
            .or_else(|| crate::platform::home_dir());
        (mode, cid, cwd)
    };

    if sandbox_mode == "docker" && container_id.as_deref().unwrap_or("").is_empty() {
        send_error(
            &mut ws_tx,
            "Docker container is not running for this terminal. Try closing and reopening it.",
        )
        .await;
        return;
    }

    let session_lock = {
        let mut locks = state.ws_session_locks.lock().await;
        locks
            .entry(instance_id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _session_guard = session_lock.lock().await;

    shutdown_instance_pty(&state, &instance_id).await;

    let sandbox_mode_clone = sandbox_mode.clone();
    let container_id_clone = container_id.clone();
    let cwd_clone = cwd.clone();

    let handles = match tokio::task::spawn_blocking(move || {
        spawn_pty_child(
            &sandbox_mode_clone,
            container_id_clone.as_deref(),
            cwd_clone.as_deref(),
        )
    })
    .await
    {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => {
            warn!("pty spawn failed: {e}");
            send_error(&mut ws_tx, &format!("Failed to start shell: {e}")).await;
            return;
        }
        Err(e) => {
            warn!("pty spawn task failed: {e}");
            send_error(&mut ws_tx, "Failed to start shell").await;
            return;
        }
    };

    let PtyHandles {
        reader,
        writer,
        master,
        child,
    } = handles;

    let master = Arc::new(std::sync::Mutex::new(master));
    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (read_tx, mut read_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    state.pty_sessions.write().await.insert(
        instance_id.clone(),
        PtySession {
            input_tx: write_tx.clone(),
        },
    );

    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::sync_channel::<()>(1);

    let writer_handle = tokio::task::spawn_blocking(move || {
        let mut writer = writer;
        while let Some(data) = write_rx.blocking_recv() {
            if writer.write_all(&data).is_err() {
                break;
            }
        }
    });

    let reader_handle = tokio::task::spawn_blocking(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if read_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let _child_handle = tokio::task::spawn_blocking(move || {
        let mut child = child;
        let _ = shutdown_rx.recv();
        let _ = child.kill();
        let _ = child.wait();
    });

    loop {
        tokio::select! {
            msg = ws_rx.next() => {
                let Some(Ok(msg)) = msg else { break };
                match msg {
                    Message::Text(text) => {
                        if let Some(control) = text.strip_prefix(CONTROL_PREFIX) {
                            handle_control_message(control, &master);
                            continue;
                        }
                        if write_tx.send(text.as_bytes().to_vec()).is_err() {
                            break;
                        }
                    }
                    Message::Binary(data) => {
                        if write_tx.send(data.to_vec()).is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            out = read_rx.recv() => {
                let Some(data) = out else { break };
                // Python backend used send_text — match that on the wire
                let text = String::from_utf8_lossy(&data).into_owned();
                if ws_tx.send(Message::Text(Utf8Bytes::from(text))).await.is_err() {
                    break;
                }
            }
        }
    }

    drop(write_tx);
    let _ = shutdown_tx.send(());
    writer_handle.abort();
    reader_handle.abort();
    state.pty_sessions.write().await.remove(&instance_id);
}

pub async fn shutdown_instance_pty(state: &AppState, instance_id: &str) {
    state.pty_sessions.write().await.remove(instance_id);
}

fn handle_control_message(
    payload: &str,
    master: &Arc<std::sync::Mutex<Box<dyn MasterPty + Send>>>,
) {
    let Ok(control) = serde_json::from_str::<PtyControl>(payload) else {
        return;
    };
    match control {
        PtyControl::Resize { cols, rows } => {
            if cols == 0 || rows == 0 {
                return;
            }
            if let Ok(master) = master.lock() {
                let _ = master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
        }
    }
}

struct PtyHandles {
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send>,
}

fn spawn_pty_child(
    sandbox_mode: &str,
    container_id: Option<&str>,
    cwd: Option<&std::path::Path>,
) -> anyhow::Result<PtyHandles> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let cmd = if sandbox_mode == "native" {
        // Cross-platform shell selection: bash/zsh on Unix, PowerShell/cmd on Windows.
        let (shell, args) = crate::platform::native_shell();
        let mut c = CommandBuilder::new(&shell);
        for arg in &args {
            c.arg(arg);
        }
        if let Some(cwd) = cwd {
            c.cwd(cwd);
        }
        c.env("TERM", "xterm-256color");
        c
    } else {
        let cid = container_id.context("missing container_id")?;
        // Use -i only (not -t): portable-pty already provides the terminal.
        // Nesting docker's -t inside a ConPTY on Windows causes immediate disconnect.
        // `script` allocates a PTY inside the container so bash doesn't warn about
        // job control / ioctl when attached through docker exec -i on Windows.
        let mut c = CommandBuilder::new(crate::platform::docker_cli());
        c.args([
            "exec",
            "-i",
            "--workdir",
            "/workspace",
            cid,
            "script",
            "-qf",
            "/dev/null",
            "-c",
            "bash -li",
        ]);
        c.env("TERM", "xterm-256color");
        c
    };

    let child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    Ok(PtyHandles {
        reader,
        writer,
        master: pair.master,
        child,
    })
}
