use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, Child, CommandBuilder, PtySize};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use crate::state::{AppState, PtySession};

pub async fn handle_pty_socket(socket: WebSocket, instance_id: String, state: Arc<AppState>) {
    let (sandbox_mode, container_id, cwd) = {
        let instances = state.instances.read().await;
        let instance = match instances.get(&instance_id) {
            Some(i) => i.clone(),
            None => return,
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
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(PathBuf::from)
                    .filter(|p| p.is_dir())
            });
        (mode, cid, cwd)
    };

    if sandbox_mode == "docker" && container_id.as_deref().unwrap_or("").is_empty() {
        return;
    }

    let (input_tx, input_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (close_tx, mut close_rx) = oneshot::channel::<()>();
    state.pty_sessions.write().await.insert(
        instance_id.clone(),
        PtySession {
            input_tx,
            close_tx: Arc::new(std::sync::Mutex::new(Some(close_tx))),
        },
    );

    let pty_spawn = tokio::task::spawn_blocking(move || {
        spawn_pty_child(&sandbox_mode, container_id.as_deref(), cwd.as_deref())
    })
    .await;

    let pty = match pty_spawn {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            warn!("pty spawn failed: {e}");
            state.pty_sessions.write().await.remove(&instance_id);
            return;
        }
        Err(e) => {
            warn!("pty task failed: {e}");
            state.pty_sessions.write().await.remove(&instance_id);
            return;
        }
    };

    let (pty_out_tx, mut pty_out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let PtyHandles {
        reader,
        writer,
        master,
        child,
    } = pty;

    tokio::task::spawn_blocking(move || {
        let mut reader = reader;
        let mut buf = [0u8; 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if pty_out_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    tokio::task::spawn_blocking(move || {
        let mut writer = writer;
        let mut input_rx = input_rx;
        while let Some(data) = input_rx.blocking_recv() {
            if writer.write_all(&data).is_err() {
                break;
            }
        }
    });

    let (mut ws_tx, mut ws_rx) = socket.split();

    let ws_write = tokio::spawn(async move {
        while let Some(data) = pty_out_rx.recv().await {
            let text = String::from_utf8_lossy(&data).into_owned();
            if ws_tx
                .send(Message::Text(Utf8Bytes::from(text)))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            _ = &mut close_rx => break,
            msg = ws_rx.next() => {
                let Some(Ok(msg)) = msg else {
                    break;
                };
                match msg {
                    Message::Text(text) => {
                        if let Some(session) = state.pty_sessions.read().await.get(&instance_id) {
                            let _ = session.input_tx.send(text.as_bytes().to_vec());
                        }
                    }
                    Message::Binary(data) => {
                        if let Some(session) = state.pty_sessions.read().await.get(&instance_id) {
                            let _ = session.input_tx.send(data.to_vec());
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }

    ws_write.abort();
    state.pty_sessions.write().await.remove(&instance_id);
    tokio::task::spawn_blocking(move || {
        let mut child = child;
        let _ = child.kill();
    });
    drop(master);
}

struct PtyHandles {
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
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
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        let mut c = CommandBuilder::new(&shell);
        c.arg("-l");
        c.arg("-i");
        if let Some(cwd) = cwd {
            c.cwd(cwd);
        }
        c.env("TERM", "xterm-256color");
        c
    } else {
        let cid = container_id.context("missing container_id")?;
        let mut c = CommandBuilder::new("docker");
        c.args(["exec", "-i", cid, "/bin/bash"]);
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
