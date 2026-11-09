use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Context;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Deserialize;
use tokio::sync::{mpsc, Mutex, Notify};
use tracing::warn;

use crate::state::{AppState, PtySession};

const CONTROL_PREFIX: &str = "__handover_control__:";

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Deserialize)]
#[serde(tag = "type")]
enum PtyControl {
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
}

/// Mirrors the original Python backend: one WebSocket ↔ one PTY ↔ one shell.
pub async fn handle_pty_socket(socket: WebSocket, instance_id: String, state: Arc<AppState>) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    async fn send_error(ws_tx: &mut futures::stream::SplitSink<WebSocket, Message>, message: &str) {
        let _ = ws_tx
            .send(Message::Text(Utf8Bytes::from(format!(
                "\r\n\x1b[31m[handover] {message}\x1b[0m\r\n"
            ))))
            .await;
    }

    let (sandbox_mode, container_id, cwd, env_vars) = {
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
        let env_vars: HashMap<String, String> = instance
            .get("custom_env_vars")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
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
        (mode, cid, cwd, env_vars)
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
    // Held only during setup. The previous session (if any) is told to exit
    // instead of being waited on, so a half-open old socket can't block a
    // reconnect.
    let session_guard = session_lock.lock().await;

    stop_pty_session(&state, &instance_id).await;

    let sandbox_mode_clone = sandbox_mode.clone();
    let container_id_clone = container_id.clone();
    let cwd_clone = cwd.clone();

    let handles = match tokio::task::spawn_blocking(move || {
        spawn_pty_child(
            &sandbox_mode_clone,
            container_id_clone.as_deref(),
            cwd_clone.as_deref(),
            &env_vars,
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

    let shell_pid = child.process_id();
    let master = Arc::new(std::sync::Mutex::new(master));
    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (read_tx, mut read_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let kill = Arc::new(Notify::new());

    state.pty_sessions.write().await.insert(
        instance_id.clone(),
        PtySession {
            input_tx: write_tx.clone(),
            shell_pid,
            session_id,
            kill: kill.clone(),
        },
    );
    drop(session_guard);

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

    // Bytes of a UTF-8 character split across PTY reads, carried to the next read.
    let mut pending_utf8: Vec<u8> = Vec::new();

    loop {
        tokio::select! {
            _ = kill.notified() => break,
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
                pending_utf8.extend_from_slice(&data);
                let text = drain_utf8(&mut pending_utf8);
                if text.is_empty() {
                    continue;
                }
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
    let mut sessions = state.pty_sessions.write().await;
    if sessions.get(&instance_id).map(|s| s.session_id) == Some(session_id) {
        sessions.remove(&instance_id);
    }
}

/// Ends the instance's current shell (if any): its WebSocket loop exits and
/// the child process is killed.
async fn stop_pty_session(state: &AppState, instance_id: &str) {
    if let Some(session) = state.pty_sessions.write().await.remove(instance_id) {
        session.kill.notify_one();
    }
}

/// Called when an instance is deleted or its project unloaded: kills the
/// shell and forgets the per-instance setup lock.
pub async fn shutdown_instance_pty(state: &AppState, instance_id: &str) {
    stop_pty_session(state, instance_id).await;
    state.ws_session_locks.lock().await.remove(instance_id);
}

/// Decodes the longest valid UTF-8 prefix of `pending`, leaving an incomplete
/// trailing character in the buffer for the next read. Truly invalid bytes
/// become U+FFFD.
fn drain_utf8(pending: &mut Vec<u8>) -> String {
    let mut out = String::new();
    let mut start = 0;
    loop {
        match std::str::from_utf8(&pending[start..]) {
            Ok(s) => {
                out.push_str(s);
                pending.clear();
                return out;
            }
            Err(e) => {
                let valid = start + e.valid_up_to();
                // SAFETY of unwrap: from_utf8 just validated this range.
                out.push_str(std::str::from_utf8(&pending[start..valid]).unwrap());
                match e.error_len() {
                    Some(len) => {
                        out.push('\u{FFFD}');
                        start = valid + len;
                    }
                    None => {
                        pending.drain(..valid);
                        return out;
                    }
                }
            }
        }
    }
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
    env_vars: &HashMap<String, String>,
) -> anyhow::Result<PtyHandles> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let cmd = if sandbox_mode == "native" {
        // Same flags the Python pexpect backend used
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        let mut c = CommandBuilder::new(&shell);
        c.args(["-l", "-i"]);
        if let Some(cwd) = cwd {
            c.cwd(cwd);
        }
        for (k, v) in env_vars {
            c.env(k, v);
        }
        c.env("TERM", "xterm-256color");
        c
    } else {
        let cid = container_id.context("missing container_id")?;
        let mut c = CommandBuilder::new("docker");
        c.args(["exec", "-it", cid, "/bin/bash"]);
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

#[cfg(test)]
mod tests {
    use super::drain_utf8;

    #[test]
    fn drain_utf8_keeps_split_character_for_next_read() {
        let bytes = "a→b".as_bytes(); // '→' is 3 bytes
        let mut pending = bytes[..2].to_vec();
        assert_eq!(drain_utf8(&mut pending), "a");
        assert_eq!(pending, bytes[1..2]);
        pending.extend_from_slice(&bytes[2..]);
        assert_eq!(drain_utf8(&mut pending), "→b");
        assert!(pending.is_empty());
    }

    #[test]
    fn drain_utf8_replaces_invalid_bytes() {
        let mut pending = vec![b'x', 0xFF, b'y'];
        assert_eq!(drain_utf8(&mut pending), "x\u{FFFD}y");
        assert!(pending.is_empty());
    }
}
