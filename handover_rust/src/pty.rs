use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Context;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, Child, CommandBuilder, PtySize};
use serde::Deserialize;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tracing::warn;

use crate::state::{AppState, PtySession, SharedPty, WsAttachment};

const CONTROL_PREFIX: &str = "__handover_control__:";
const PTY_OUTPUT_CAPACITY: usize = 256;
static WS_ATTACH_ID: AtomicU64 = AtomicU64::new(1);

async fn attachment_is_active(state: &AppState, instance_id: &str, attach_id: u64) -> bool {
    state
        .ws_attachments
        .read()
        .await
        .get(instance_id)
        .map(|attachment| attachment.attach_id)
        == Some(attach_id)
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum PtyControl {
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
}

pub async fn handle_pty_socket(socket: WebSocket, instance_id: String, state: Arc<AppState>) {
    let (mut socket_tx, socket_rx) = socket.split();

    async fn send_error(
        socket_tx: &mut futures::stream::SplitSink<WebSocket, Message>,
        message: &str,
    ) {
        let _ = socket_tx
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
                    &mut socket_tx,
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
        send_error(
            &mut socket_tx,
            "Docker container is not running for this terminal. Try closing and reopening it.",
        )
        .await;
        return;
    }

    detach_ws_attachment(&state, &instance_id).await;

    let shared = match ensure_shared_pty(
        &state,
        &instance_id,
        &sandbox_mode,
        container_id.as_deref(),
        cwd.as_deref(),
    )
    .await
    {
        Ok(core) => core,
        Err(e) => {
            warn!("pty spawn failed: {e}");
            send_error(&mut socket_tx, &format!("Failed to start shell: {e}")).await;
            return;
        }
    };

    let attach_id = WS_ATTACH_ID.fetch_add(1, Ordering::SeqCst);
    let (detach_tx, mut detach_rx) = oneshot::channel::<()>();
    state.ws_attachments.write().await.insert(
        instance_id.clone(),
        WsAttachment {
            attach_id,
            detach_tx,
        },
    );

    let input_tx = shared.input_tx.clone();
    let master = shared.master.clone();
    let mut output_rx = shared.output_tx.subscribe();
    let state_for_output = state.clone();
    let instance_id_for_output = instance_id.clone();

    let (mut ws_tx, mut ws_rx) = (socket_tx, socket_rx);

    let ws_write = tokio::spawn(async move {
        loop {
            if !attachment_is_active(&state_for_output, &instance_id_for_output, attach_id).await {
                break;
            }
            match output_rx.recv().await {
                Ok(data) => {
                    if !attachment_is_active(&state_for_output, &instance_id_for_output, attach_id)
                        .await
                    {
                        break;
                    }
                    if ws_tx.send(Message::Binary(data.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    loop {
        tokio::select! {
            _ = &mut detach_rx => break,
            msg = ws_rx.next() => {
                let Some(Ok(msg)) = msg else {
                    break;
                };
                match msg {
                    Message::Text(text) => {
                        if !attachment_is_active(&state, &instance_id, attach_id).await {
                            break;
                        }
                        if let Some(control) = text.strip_prefix(CONTROL_PREFIX) {
                            handle_control_message(control, &master);
                            continue;
                        }
                        let _ = input_tx.send(text.as_bytes().to_vec());
                    }
                    Message::Binary(data) => {
                        if !attachment_is_active(&state, &instance_id, attach_id).await {
                            break;
                        }
                        let _ = input_tx.send(data.to_vec());
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }

    ws_write.abort();
    let mut attachments = state.ws_attachments.write().await;
    if attachments
        .get(&instance_id)
        .is_some_and(|attachment| attachment.attach_id == attach_id)
    {
        attachments.remove(&instance_id);
    }
}

pub async fn shutdown_instance_pty(state: &AppState, instance_id: &str) {
    detach_ws_attachment(state, instance_id).await;

    let core = state.pty_cores.write().await.remove(instance_id);
    state.pty_sessions.write().await.remove(instance_id);

    if let Some(core) = core {
        shutdown_shared_pty(core).await;
    }
}

async fn detach_ws_attachment(state: &AppState, instance_id: &str) {
    if let Some(attachment) = state.ws_attachments.write().await.remove(instance_id) {
        let _ = attachment.detach_tx.send(());
    }
}

async fn ensure_shared_pty(
    state: &AppState,
    instance_id: &str,
    sandbox_mode: &str,
    container_id: Option<&str>,
    cwd: Option<&std::path::Path>,
) -> anyhow::Result<Arc<SharedPty>> {
    if let Some(existing) = state.pty_cores.read().await.get(instance_id) {
        return Ok(existing.clone());
    }

    let create_lock = {
        let mut locks = state.pty_create_locks.lock().await;
        locks
            .entry(instance_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _create_guard = create_lock.lock().await;

    if let Some(existing) = state.pty_cores.read().await.get(instance_id) {
        return Ok(existing.clone());
    }

    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (output_tx, _) = broadcast::channel(PTY_OUTPUT_CAPACITY);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let sandbox_mode = sandbox_mode.to_string();
    let container_id = container_id.map(String::from);
    let cwd = cwd.map(|path| path.to_path_buf());

    let pty = tokio::task::spawn_blocking(move || {
        spawn_pty_child(&sandbox_mode, container_id.as_deref(), cwd.as_deref())
    })
    .await??;

    let PtyHandles {
        reader,
        writer,
        master,
        child,
    } = pty;
    let master = Arc::new(std::sync::Mutex::new(master));
    let output_tx_reader = output_tx.clone();

    tokio::task::spawn_blocking(move || {
        let mut reader = reader;
        let mut buf = [0u8; 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if output_tx_reader.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    tokio::task::spawn_blocking(move || {
        let mut writer = writer;
        while let Some(data) = input_rx.blocking_recv() {
            if writer.write_all(&data).is_err() {
                break;
            }
        }
    });

    let core = Arc::new(SharedPty {
        input_tx: input_tx.clone(),
        output_tx,
        shutdown_tx: Arc::new(std::sync::Mutex::new(Some(shutdown_tx))),
        master: master.clone(),
    });

    state
        .pty_cores
        .write()
        .await
        .insert(instance_id.to_string(), core.clone());
    state
        .pty_sessions
        .write()
        .await
        .insert(instance_id.to_string(), PtySession { input_tx });

    tokio::spawn(async move {
        let _ = shutdown_rx.await;
        tokio::task::spawn_blocking(move || {
            let mut child = child;
            let _ = child.kill();
        });
    });

    Ok(core)
}

async fn shutdown_shared_pty(core: Arc<SharedPty>) {
    if let Ok(mut shutdown_tx) = core.shutdown_tx.lock() {
        if let Some(tx) = shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

fn handle_control_message(
    payload: &str,
    master: &Arc<std::sync::Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
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
