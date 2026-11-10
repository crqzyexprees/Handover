//! Terminal sessions: one long-lived shell per Instance.
//!
//! The shell belongs to the Instance, not to a WebSocket. A client *attaches*
//! to the running shell and *detaches* when its socket closes; the shell keeps
//! running. Recent output is kept in an [`OutputLog`] so a client that
//! reconnects can catch up on what it missed.
//!
//! Wire protocol on `/ws/pty/{id}?session=<id>&since=<offset>`:
//! - client → server text: keystrokes, or `__handover_control__:{"type":"resize",...}`
//! - server → client text: terminal output, or `__handover_control__:{json}` with
//!   `type` one of `attached {session, from}`, `exit`, `replaced`, `error {message}`.
//!
//! Offsets count UTF-16 code units of output text, which is what a browser's
//! `string.length` measures, so client and server agree on positions.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{broadcast, mpsc, Mutex, Notify, RwLock};
use tracing::warn;

const CONTROL_PREFIX: &str = "__handover_control__:";

/// Output kept for replay, in UTF-16 code units (~2 MB of text).
const OUTPUT_LOG_CAP: u64 = 1_000_000;

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Deserialize)]
#[serde(tag = "type")]
enum PtyControl {
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
}

/// Query parameters a client sends when attaching.
#[derive(Debug, Default, Deserialize)]
pub struct AttachParams {
    /// Session the client last saw output from.
    pub session: Option<u64>,
    /// Offset (UTF-16 units) of the end of the output the client already has.
    pub since: Option<u64>,
}

// ── Output log ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct Chunk {
    start: u64,
    text: Arc<str>,
}

#[derive(Clone, Debug)]
enum Event {
    Output(Chunk),
    Exited,
}

fn utf16_len(s: &str) -> u64 {
    s.encode_utf16().count() as u64
}

/// Bounded history of a session's output plus a live feed of new output.
/// Pushes and subscriptions happen under the same lock, so a subscriber's
/// replay and its live feed never overlap or leave a gap.
pub struct OutputLog {
    chunks: VecDeque<Chunk>,
    start: u64,
    end: u64,
    cap: u64,
    exited: bool,
    tx: broadcast::Sender<Event>,
}

impl OutputLog {
    fn new(cap: u64) -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            chunks: VecDeque::new(),
            start: 0,
            end: 0,
            cap,
            exited: false,
            tx,
        }
    }

    fn push(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        let chunk = Chunk {
            start: self.end,
            text: Arc::from(text),
        };
        self.end += utf16_len(&chunk.text);
        self.chunks.push_back(chunk.clone());
        while self.end - self.start > self.cap && self.chunks.len() > 1 {
            let dropped = self.chunks.pop_front().expect("len > 1");
            self.start = dropped.start + utf16_len(&dropped.text);
        }
        let _ = self.tx.send(Event::Output(chunk));
    }

    fn mark_exited(&mut self) {
        self.exited = true;
        let _ = self.tx.send(Event::Exited);
    }

    /// Chunks after `since` (or everything kept, if `since` is unknown or
    /// already trimmed away), the offset they start from, and a live feed
    /// that continues exactly where the replay ends.
    fn subscribe_since(&self, since: Option<u64>) -> (u64, Vec<Chunk>, broadcast::Receiver<Event>) {
        let from = match since {
            Some(s) if s >= self.start && s <= self.end => s,
            _ => self.start,
        };
        let replay = self
            .chunks
            .iter()
            .filter(|c| c.start >= from)
            .cloned()
            .collect();
        (from, replay, self.tx.subscribe())
    }
}

// ── Session ────────────────────────────────────────────────────────────────

/// A running shell for one Instance. Lives until the shell exits or the
/// Instance is deleted, independent of any client connection.
pub struct PtySession {
    pub session_id: u64,
    pub input_tx: mpsc::UnboundedSender<Vec<u8>>,
    /// Shell / docker-exec PID for native resource accounting.
    pub shell_pid: Option<u32>,
    master: Arc<std::sync::Mutex<Box<dyn MasterPty + Send>>>,
    log: Arc<std::sync::Mutex<OutputLog>>,
    kill_tx: std::sync::mpsc::SyncSender<()>,
    /// The attached client, signalled when another client takes over.
    client: std::sync::Mutex<Option<Arc<Notify>>>,
}

impl PtySession {
    fn kill(&self) {
        let _ = self.kill_tx.try_send(());
    }

    fn resize(&self, cols: u16, rows: u16) {
        if cols == 0 || rows == 0 {
            return;
        }
        if let Ok(master) = self.master.lock() {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    fn is_exited(&self) -> bool {
        self.log.lock().map(|l| l.exited).unwrap_or(true)
    }
}

// ── Terminals ──────────────────────────────────────────────────────────────

type Sessions = Arc<RwLock<HashMap<String, Arc<PtySession>>>>;

/// Every running shell, by Instance id. Shells outlive client connections;
/// one ends when it exits, or when [`Terminals::shutdown`] is called for
/// its Instance.
#[derive(Default)]
pub struct Terminals {
    sessions: Sessions,
    /// Serializes starting and shutting down a shell per Instance.
    setup_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl Terminals {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    async fn setup_lock(&self, instance_id: &str) -> Arc<Mutex<()>> {
        self.setup_locks
            .lock()
            .await
            .entry(instance_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// The Instance's running shell, starting one from `command` if there is
    /// none. `command` is only called when a shell must be started.
    pub async fn attach_or_spawn<F, Fut>(
        &self,
        instance_id: &str,
        command: F,
    ) -> Result<Arc<PtySession>, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<CommandBuilder, String>>,
    {
        let lock = self.setup_lock(instance_id).await;
        let _guard = lock.lock().await;
        if let Some(existing) = self.sessions.read().await.get(instance_id) {
            if !existing.is_exited() {
                return Ok(existing.clone());
            }
        }
        let cmd = command().await?;
        let session = spawn_session(&self.sessions, instance_id, cmd).await?;
        self.sessions
            .write()
            .await
            .insert(instance_id.to_string(), session.clone());
        Ok(session)
    }

    /// Ends the Instance's shell (attached clients receive `exit`). Waits for
    /// a shell that is still starting, so it can't slip in afterwards.
    pub async fn shutdown(&self, instance_id: &str) {
        let lock = self.setup_lock(instance_id).await;
        {
            let _guard = lock.lock().await;
            if let Some(session) = self.sessions.write().await.remove(instance_id) {
                session.kill();
            }
        }
        self.setup_locks.lock().await.remove(instance_id);
    }

    /// How many shells are running.
    pub async fn running(&self) -> usize {
        self.sessions.read().await.len()
    }

    /// For each given Instance with a running shell: its shell's pid.
    pub async fn running_shells(&self, instance_ids: &[&str]) -> HashMap<String, Option<u32>> {
        let sessions = self.sessions.read().await;
        instance_ids
            .iter()
            .filter_map(|id| sessions.get(*id).map(|s| (id.to_string(), s.shell_pid)))
            .collect()
    }

    /// Types `prompt` into the Instance's running shell and submits it with
    /// `\r` (what the Enter key sends). Raw-mode TUIs such as Claude Code
    /// treat `\n` as "insert newline", not "submit". The submit key is sent
    /// separately after a short pause so TUIs don't fold it into the pasted
    /// text. False if the Instance has no running shell.
    pub async fn type_prompt(&self, instance_id: &str, prompt: &str) -> bool {
        let Some(input_tx) = self
            .sessions
            .read()
            .await
            .get(instance_id)
            .map(|s| s.input_tx.clone())
        else {
            return false;
        };
        if input_tx
            .send(prompt_as_single_line(prompt).into_bytes())
            .is_err()
        {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        input_tx.send(b"\r".to_vec()).is_ok()
    }
}

/// Starts a shell and the tasks that pump its input and output.
async fn spawn_session(
    sessions: &Sessions,
    instance_id: &str,
    cmd: CommandBuilder,
) -> Result<Arc<PtySession>, String> {
    let handles = tokio::task::spawn_blocking(move || spawn_pty_child(cmd))
        .await
        .map_err(|e| format!("Failed to start shell: {e}"))?
        .map_err(|e| format!("Failed to start shell: {e}"))?;

    let PtyHandles {
        reader,
        writer,
        master,
        child,
    } = handles;

    let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let shell_pid = child.process_id();
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (kill_tx, kill_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let log = Arc::new(std::sync::Mutex::new(OutputLog::new(OUTPUT_LOG_CAP)));

    // Input: channel → PTY writer. Ends when every sender is dropped.
    tokio::task::spawn_blocking(move || {
        let mut writer = writer;
        while let Some(data) = input_rx.blocking_recv() {
            if writer.write_all(&data).is_err() {
                break;
            }
        }
    });

    // Child: killed on request (delete) or reaped after it exits on its own.
    tokio::task::spawn_blocking(move || {
        let mut child = child;
        let _ = kill_rx.recv();
        let _ = child.kill();
        let _ = child.wait();
    });

    // Output: PTY reader → bytes channel → decoded into the log.
    let (bytes_tx, mut bytes_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::task::spawn_blocking(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if bytes_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let session = Arc::new(PtySession {
        session_id,
        input_tx,
        shell_pid,
        master: Arc::new(std::sync::Mutex::new(master)),
        log: log.clone(),
        kill_tx: kill_tx.clone(),
        client: std::sync::Mutex::new(None),
    });

    let pump_sessions = sessions.clone();
    let pump_instance = instance_id.to_string();
    tokio::spawn(async move {
        let mut pending = Vec::new();
        while let Some(bytes) = bytes_rx.recv().await {
            pending.extend_from_slice(&bytes);
            let text = drain_utf8(&mut pending);
            if let Ok(mut log) = log.lock() {
                log.push(text);
            }
        }
        // Shell exited (or was killed): tell clients, reap, and forget it so
        // the next attach starts a fresh shell.
        if let Ok(mut log) = log.lock() {
            log.mark_exited();
        }
        let _ = kill_tx.try_send(());
        let mut sessions = pump_sessions.write().await;
        if sessions.get(&pump_instance).map(|s| s.session_id) == Some(session_id) {
            sessions.remove(&pump_instance);
        }
    });

    Ok(session)
}

/// Collapses a prompt to one line so it is submitted once: interior newlines
/// would otherwise be read as several separate submissions.
fn prompt_as_single_line(prompt: &str) -> String {
    prompt.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── Attaching a client ─────────────────────────────────────────────────────

type WsTx = SplitSink<WebSocket, Message>;

async fn send_control(ws_tx: &mut WsTx, value: serde_json::Value) -> bool {
    ws_tx
        .send(Message::Text(Utf8Bytes::from(format!(
            "{CONTROL_PREFIX}{value}"
        ))))
        .await
        .is_ok()
}

/// Sends output text. A chunk that happens to begin with the control prefix
/// is split so the client can't mistake it for a control frame; the bytes
/// and offsets are unchanged.
async fn send_output(ws_tx: &mut WsTx, text: &str) -> bool {
    if text.starts_with(CONTROL_PREFIX) {
        let (head, tail) = text.split_at(1);
        return ws_tx
            .send(Message::Text(Utf8Bytes::from(head)))
            .await
            .is_ok()
            && ws_tx
                .send(Message::Text(Utf8Bytes::from(tail)))
                .await
                .is_ok();
    }
    ws_tx
        .send(Message::Text(Utf8Bytes::from(text)))
        .await
        .is_ok()
}

/// Attaches one WebSocket client to the instance's shell until the client
/// leaves, another client takes over, or the shell exits. Never kills the
/// shell.
pub async fn handle_pty_socket<F, Fut>(
    socket: WebSocket,
    instance_id: String,
    params: AttachParams,
    terminals: Arc<Terminals>,
    command: F,
) where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<CommandBuilder, String>>,
{
    let (mut ws_tx, mut ws_rx) = socket.split();

    let session = match terminals.attach_or_spawn(&instance_id, command).await {
        Ok(s) => s,
        Err(message) => {
            warn!("pty attach failed for {instance_id}: {message}");
            let _ = send_control(&mut ws_tx, json!({ "type": "error", "message": message })).await;
            return;
        }
    };

    // Newest client wins; the previous one is told it was replaced.
    let kicked = Arc::new(Notify::new());
    if let Ok(mut client) = session.client.lock() {
        if let Some(previous) = client.replace(kicked.clone()) {
            previous.notify_one();
        }
    }

    let since = match params.session {
        Some(id) if id == session.session_id => params.since,
        _ => None,
    };
    let (from, replay, mut feed, exited) = {
        let log = session.log.lock().expect("output log poisoned");
        let (from, replay, feed) = log.subscribe_since(since);
        (from, replay, feed, log.exited)
    };

    let mut sent_to = from;
    let mut ok = send_control(
        &mut ws_tx,
        json!({ "type": "attached", "session": session.session_id, "from": from }),
    )
    .await;
    for chunk in &replay {
        if !ok {
            break;
        }
        ok = send_output(&mut ws_tx, &chunk.text).await;
        sent_to = chunk.start + utf16_len(&chunk.text);
    }
    if ok && exited {
        let _ = send_control(&mut ws_tx, json!({ "type": "exit" })).await;
        ok = false;
    }

    while ok {
        tokio::select! {
            _ = kicked.notified() => {
                let _ = send_control(&mut ws_tx, json!({ "type": "replaced" })).await;
                break;
            }
            msg = ws_rx.next() => {
                let Some(Ok(msg)) = msg else { break };
                match msg {
                    Message::Text(text) => {
                        if let Some(control) = text.strip_prefix(CONTROL_PREFIX) {
                            if let Ok(PtyControl::Resize { cols, rows }) = serde_json::from_str(control) {
                                session.resize(cols, rows);
                            }
                        } else if session.input_tx.send(text.as_bytes().to_vec()).is_err() {
                            break;
                        }
                    }
                    Message::Binary(data) => {
                        if session.input_tx.send(data.to_vec()).is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            event = feed.recv() => match event {
                Ok(Event::Output(chunk)) => {
                    if !send_output(&mut ws_tx, &chunk.text).await {
                        break;
                    }
                    sent_to = chunk.start + utf16_len(&chunk.text);
                }
                Ok(Event::Exited) | Err(broadcast::error::RecvError::Closed) => {
                    let _ = send_control(&mut ws_tx, json!({ "type": "exit" })).await;
                    break;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Client fell behind the live feed: resync from the log.
                    let (from, replay, new_feed) = {
                        let log = session.log.lock().expect("output log poisoned");
                        log.subscribe_since(Some(sent_to))
                    };
                    feed = new_feed;
                    if from != sent_to {
                        let attached = json!({ "type": "attached", "session": session.session_id, "from": from });
                        if !send_control(&mut ws_tx, attached).await {
                            break;
                        }
                    }
                    sent_to = from;
                    for chunk in &replay {
                        if !send_output(&mut ws_tx, &chunk.text).await {
                            ok = false;
                            break;
                        }
                        sent_to = chunk.start + utf16_len(&chunk.text);
                    }
                }
            }
        }
    }

    // Detach only; the shell keeps running for the next client.
    let mut client = session.client.lock().unwrap_or_else(|e| e.into_inner());
    if client.as_ref().is_some_and(|c| Arc::ptr_eq(c, &kicked)) {
        *client = None;
    }
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

struct PtyHandles {
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send>,
}

fn spawn_pty_child(cmd: CommandBuilder) -> anyhow::Result<PtyHandles> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;

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
    use super::*;

    fn sleeper() -> CommandBuilder {
        let mut c = CommandBuilder::new("sleep");
        c.arg("30");
        c
    }

    #[tokio::test]
    async fn shutdown_during_a_spawn_leaves_no_shell_behind() {
        let terminals = Terminals::new();
        let t = terminals.clone();
        let attach = tokio::spawn(async move {
            t.attach_or_spawn("i", || async {
                // Still starting when shutdown arrives.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                Ok(sleeper())
            })
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        terminals.shutdown("i").await;
        assert!(attach.await.unwrap().is_ok());
        assert_eq!(terminals.running().await, 0);
    }

    #[tokio::test]
    async fn attach_reuses_the_running_shell() {
        let terminals = Terminals::new();
        let a = terminals
            .attach_or_spawn("i", || async { Ok(sleeper()) })
            .await
            .unwrap();
        let b = terminals
            .attach_or_spawn("i", || async { Err("must not respawn".to_string()) })
            .await
            .unwrap();
        assert_eq!(a.session_id, b.session_id);
        terminals.shutdown("i").await;
        assert_eq!(terminals.running().await, 0);
    }

    #[test]
    fn prompt_as_single_line_collapses_newlines() {
        assert_eq!(
            prompt_as_single_line("  fix\nthe\r\n  bug \n"),
            "fix the bug"
        );
    }

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

    fn texts(chunks: &[Chunk]) -> String {
        chunks.iter().map(|c| c.text.as_ref()).collect()
    }

    #[test]
    fn output_log_replays_only_what_the_client_missed() {
        let mut log = OutputLog::new(1_000);
        log.push("hello ".into());
        log.push("world".into());
        let (from, replay, _) = log.subscribe_since(Some(6));
        assert_eq!(from, 6);
        assert_eq!(texts(&replay), "world");
        let (from, replay, _) = log.subscribe_since(None);
        assert_eq!(from, 0);
        assert_eq!(texts(&replay), "hello world");
    }

    #[test]
    fn output_log_offsets_count_utf16_units() {
        let mut log = OutputLog::new(1_000);
        log.push("→😀".into()); // 1 + 2 UTF-16 units
        log.push("x".into());
        let (from, replay, _) = log.subscribe_since(Some(3));
        assert_eq!(from, 3);
        assert_eq!(texts(&replay), "x");
    }

    #[test]
    fn output_log_trims_to_cap_and_replays_from_oldest_kept() {
        let mut log = OutputLog::new(10);
        log.push("aaaaaa".into());
        log.push("bbbbbb".into());
        log.push("cc".into());
        let (from, replay, _) = log.subscribe_since(Some(0)); // trimmed away
        assert_eq!(from, 6);
        assert_eq!(texts(&replay), "bbbbbbcc");
    }

    #[test]
    fn output_log_feed_continues_after_replay() {
        let mut log = OutputLog::new(1_000);
        log.push("old".into());
        let (_, replay, mut feed) = log.subscribe_since(None);
        log.push("new".into());
        log.mark_exited();
        assert_eq!(texts(&replay), "old");
        match feed.try_recv().unwrap() {
            Event::Output(c) => assert_eq!((c.start, c.text.as_ref()), (3, "new")),
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(feed.try_recv().unwrap(), Event::Exited));
    }
}
