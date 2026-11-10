//! Desktop shell: finds or starts the `handover-backend` and hands its port
//! and token to the UI.
//!
//! The backend outlives the window so running shells and containers survive
//! an app quit: on quit we ask it to stop only if nothing in it is busy.
//! Its port and token are kept in a private runtime file (`backend.json` in
//! the app's local data dir); the next launch reuses that backend, and its
//! terminals re-attach.
//! While the app is open it sends a heartbeat; a backend left behind exits
//! by itself after `IDLE_EXIT_SECS` with nothing busy and no requests.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tauri::{Manager, RunEvent, State};

const IDLE_EXIT_SECS: u64 = 600;
const HEARTBEAT: Duration = Duration::from_secs(60);
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

struct BackendState {
    port: u16,
    /// Secret the backend requires on every API/WebSocket call.
    token: String,
    info_path: PathBuf,
}

/// Contents of the runtime file.
#[derive(Serialize, Deserialize)]
struct BackendInfo {
    port: u16,
    token: String,
    pid: u32,
    version: String,
}

#[tauri::command]
fn get_backend_port(state: State<'_, BackendState>) -> u16 {
    state.port
}

#[tauri::command]
fn get_backend_token(state: State<'_, BackendState>) -> String {
    state.token.clone()
}

/// 244 random bits from two v4 UUIDs.
fn generate_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn pick_free_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).context("failed to bind ephemeral port")?;
    Ok(listener.local_addr()?.port())
}

/// One plain HTTP/1.0 request to the local backend: `(status, body)`.
fn http(
    port: u16,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> Option<(u16, String)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    let mut req = format!("{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n");
    if let Some(t) = token {
        req.push_str(&format!("X-Handover-Token: {t}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    req.push_str("\r\n");
    req.push_str(body.unwrap_or(""));
    stream.write_all(req.as_bytes()).ok()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok()?;
    // "HTTP/1.1 200 OK\r\n...\r\n\r\nbody"
    let status = raw.split_whitespace().nth(1)?.parse().ok()?;
    let body = raw.split_once("\r\n\r\n").map(|(_, b)| b.to_string())?;
    Some((status, body))
}

#[derive(Deserialize)]
struct Ping {
    version: String,
    shells: usize,
    /// Instances stopping the backend would lose. Older backends don't
    /// send it; their running shells are the closest measure.
    #[serde(default)]
    busy: Option<usize>,
}

impl Ping {
    fn busy(&self) -> usize {
        self.busy.unwrap_or(self.shells)
    }
}

/// What to do with the backend a previous launch left behind.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// It is up and usable: connect to it.
    Reuse,
    /// It is an older build with nothing busy: stop it, then start ours.
    ReplaceIdle,
    /// Nothing usable is there: start a new one.
    Spawn,
}

/// `ping` is the saved backend's answer, `None` if there was no saved
/// backend or it didn't answer. A backend from another app version is
/// still reused while it is busy, so its shells and containers aren't lost.
fn plan_backend(ping: Option<&Ping>, app_version: &str) -> Plan {
    match ping {
        None => Plan::Spawn,
        Some(p) if p.version == app_version || p.busy() > 0 => Plan::Reuse,
        Some(_) => Plan::ReplaceIdle,
    }
}

/// The backend's status, if it is up and accepts this token.
fn ping(port: u16, token: &str) -> Option<Ping> {
    match http(port, "GET", "/api/ping", Some(token), None)? {
        (200, body) => serde_json::from_str(&body).ok(),
        _ => None,
    }
}

/// Asks the backend to exit if nothing is busy. True if it is stopping.
fn stop_if_idle(port: u16, token: &str) -> bool {
    #[derive(Deserialize)]
    struct Reply {
        stopping: bool,
    }
    match http(
        port,
        "POST",
        "/api/shutdown",
        Some(token),
        Some(r#"{"only_if_idle":true}"#),
    ) {
        Some((200, body)) => serde_json::from_str::<Reply>(&body).is_ok_and(|r| r.stopping),
        _ => false,
    }
}

fn wait_until(timeout: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

fn read_info(path: &Path) -> Option<BackendInfo> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Writes a file only the current user can read (it holds the token).
fn write_private(path: &Path, contents: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)?.write_all(contents.as_bytes())?;
    Ok(())
}

/// The bundled backend binary (Tauri places sidecars next to the app binary).
fn bundled_backend() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("cannot locate the app binary")?;
    let name = if cfg!(windows) {
        "handover-backend.exe"
    } else {
        "handover-backend"
    };
    let path = exe.parent().context("app binary has no folder")?.join(name);
    anyhow::ensure!(
        path.is_file(),
        "handover-backend not found at {}; run scripts/prepare-backend-sidecar.sh",
        path.display()
    );
    Ok(path)
}

/// Copies the backend out of the app bundle before running it: an AppImage
/// is a mount that disappears when the app quits, and the backend may keep
/// running after that. Falls back to the bundled path if copying fails.
fn runnable_copy(bundled: &Path, data_dir: &Path) -> PathBuf {
    let dest = data_dir
        .join("bin")
        .join(bundled.file_name().unwrap_or_default());
    let tmp = dest.with_extension("tmp");
    let copied = std::fs::create_dir_all(data_dir.join("bin"))
        .and_then(|_| std::fs::copy(bundled, &tmp))
        // Rename, not overwrite: an old backend may still be running the file.
        .and_then(|_| std::fs::rename(&tmp, &dest));
    match copied {
        Ok(()) => dest,
        Err(e) => {
            eprintln!("could not copy backend to {}: {e}", dest.display());
            bundled.to_path_buf()
        }
    }
}

/// Environment changes that undo an AppImage's launcher: every variable
/// that points into the mount (`PATH`, `LD_LIBRARY_PATH`, `PYTHONHOME`, ...)
/// loses those entries, or is removed if nothing else is left. Without this
/// the backend, and every native terminal it starts, would run the bundle's
/// libraries and interpreters (and break once the mount is gone).
fn scrub_appimage_env(
    vars: impl IntoIterator<Item = (String, String)>,
    appdir: &str,
) -> Vec<(String, Option<String>)> {
    let appdir = appdir.trim_end_matches('/');
    let mut changes = Vec::new();
    for (key, value) in vars {
        if matches!(key.as_str(), "APPDIR" | "APPIMAGE" | "ARGV0") {
            changes.push((key, None));
            continue;
        }
        if appdir.is_empty() || !value.contains(appdir) {
            continue;
        }
        let kept: Vec<&str> = value
            .split(':')
            .filter(|part| !part.is_empty() && !part.starts_with(appdir))
            .collect();
        let new = (!kept.is_empty()).then(|| kept.join(":"));
        changes.push((key, new));
    }
    changes
}

/// In the forked child, just before exec: close every inherited descriptor
/// above stderr on exec. The AppImage runtime stays alive while anyone holds
/// its keep-alive pipe, so a backend that inherited it would keep a stale
/// runtime (and mount) around for as long as it runs.
#[cfg(target_os = "linux")]
fn close_inherited_fds_in_child(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the hook only makes async-signal-safe syscalls (close_range /
    // fcntl) and touches no memory.
    unsafe {
        cmd.pre_exec(|| {
            let marked = libc::syscall(
                libc::SYS_close_range,
                3u32,
                u32::MAX,
                libc::CLOSE_RANGE_CLOEXEC,
            ) == 0;
            if !marked {
                // Kernels before 5.11: mark the usual range one by one.
                for fd in 3..4096 {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags >= 0 {
                        libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
                    }
                }
            }
            Ok(())
        });
    }
}

/// Starts a backend that isn't tied to this process, so it can outlive it.
fn spawn_backend(data_dir: &Path, port: u16, token: &str) -> Result<u32> {
    let exe = runnable_copy(&bundled_backend()?, data_dir);
    let log = std::fs::File::create(data_dir.join("backend.log"))
        .context("failed to create backend.log")?;
    let mut cmd = Command::new(exe);
    cmd.args([
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
        "--idle-exit-secs",
        &IDLE_EXIT_SECS.to_string(),
    ])
    .env(
        "RUST_LOG",
        std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()),
    )
    // Env, not argv, so other local users can't read it from the process list.
    .env("HANDOVER_TOKEN", token)
    .stdin(Stdio::null())
    .stdout(log.try_clone()?)
    .stderr(log);
    // Not the app's folder, which may be inside the AppImage mount.
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        cmd.current_dir(home);
    }
    if let Ok(appdir) = std::env::var("APPDIR") {
        for (key, value) in scrub_appimage_env(std::env::vars(), &appdir) {
            match value {
                Some(v) => cmd.env(key, v),
                None => cmd.env_remove(key),
            };
        }
    }
    #[cfg(target_os = "linux")]
    close_inherited_fds_in_child(&mut cmd);
    // Own process group: a Ctrl-C or hangup aimed at the app doesn't reach it.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    #[cfg(windows)]
    {
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::os::windows::process::CommandExt::creation_flags(
            &mut cmd,
            CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW,
        );
    }
    let mut child = cmd.spawn().context("failed to start handover-backend")?;
    let pid = child.id();
    // Reap it if it exits while the app is open; never kill it.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    anyhow::ensure!(
        wait_until(Duration::from_secs(120), || ping(port, token).is_some()),
        "handover-backend did not respond on http://127.0.0.1:{port}/"
    );
    Ok(pid)
}

/// Reuses the backend from a previous launch when it is still up (keeping
/// its shells), otherwise starts a new one. Returns `(port, token)`.
fn connect_backend(data_dir: &Path, info_path: &Path) -> Result<(u16, String)> {
    if let Some(info) = read_info(info_path) {
        let status = ping(info.port, &info.token);
        match plan_backend(status.as_ref(), APP_VERSION) {
            Plan::Reuse => return Ok((info.port, info.token)),
            Plan::ReplaceIdle => {
                if stop_if_idle(info.port, &info.token) {
                    wait_until(Duration::from_secs(5), || {
                        ping(info.port, &info.token).is_none()
                    });
                }
            }
            Plan::Spawn => {}
        }
    }
    let port = pick_free_port()?;
    let token = generate_token();
    let pid = spawn_backend(data_dir, port, &token)?;
    let info = BackendInfo {
        port,
        token: token.clone(),
        pid,
        version: APP_VERSION.into(),
    };
    write_private(info_path, &serde_json::to_string(&info)?)
        .context("failed to write backend.json")?;
    Ok((port, token))
}

fn start_heartbeat(port: u16, token: String) {
    std::thread::spawn(move || loop {
        std::thread::sleep(HEARTBEAT);
        let _ = ping(port, &token);
    });
}

/// On quit: stop the backend unless something in it is busy.
fn release_backend(state: &BackendState) {
    if stop_if_idle(state.port, &state.token) {
        let _ = std::fs::remove_file(&state.info_path);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                if let Some(icon) = app.default_window_icon().cloned() {
                    let _ = window.set_icon(icon);
                }
            }
            let data_dir = app
                .path()
                .app_local_data_dir()
                .context("no local data dir")?;
            std::fs::create_dir_all(&data_dir)?;
            let info_path = data_dir.join("backend.json");
            let (port, token) = connect_backend(&data_dir, &info_path)?;
            start_heartbeat(port, token.clone());
            app.manage(BackendState {
                port,
                token,
                info_path,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_backend_port,
            get_backend_token
        ])
        .build(tauri::generate_context!());

    match result {
        Ok(app) => {
            app.run(|app_handle, event| {
                if matches!(event, RunEvent::Exit) {
                    if let Some(state) = app_handle.try_state::<BackendState>() {
                        release_backend(&state);
                    }
                }
            });
        }
        Err(error) => {
            eprintln!("failed to start Handover desktop: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pinged(version: &str, shells: usize, busy: Option<usize>) -> Ping {
        Ping {
            version: version.into(),
            shells,
            busy,
        }
    }

    #[test]
    fn backend_plan_table() {
        let cases = [
            (None, Plan::Spawn),
            (Some(pinged(APP_VERSION, 0, Some(0))), Plan::Reuse),
            (Some(pinged(APP_VERSION, 2, Some(2))), Plan::Reuse),
            (Some(pinged("0.9.0", 0, Some(0))), Plan::ReplaceIdle),
            // A container with no shell attached still counts as busy.
            (Some(pinged("0.9.0", 0, Some(1))), Plan::Reuse),
            // Older backends report only shells.
            (Some(pinged("0.9.0", 1, None)), Plan::Reuse),
            (Some(pinged("0.9.0", 0, None)), Plan::ReplaceIdle),
        ];
        for (ping, want) in cases {
            assert_eq!(plan_backend(ping.as_ref(), APP_VERSION), want);
        }
    }

    fn scrub(vars: &[(&str, &str)]) -> Vec<(String, Option<String>)> {
        scrub_appimage_env(
            vars.iter().map(|(k, v)| (k.to_string(), v.to_string())),
            "/tmp/.mount_X",
        )
    }

    #[test]
    fn appimage_paths_are_removed_and_the_rest_kept() {
        let changes = scrub(&[
            ("PATH", "/tmp/.mount_X/usr/bin/:/usr/bin:/bin"),
            ("PYTHONHOME", "/tmp/.mount_X/usr/"),
            ("LD_LIBRARY_PATH", "/tmp/.mount_X/usr/lib/:"),
            ("APPIMAGE", "/home/u/Handover.AppImage"),
            ("HOME", "/home/u"),
        ]);
        assert_eq!(
            changes,
            [
                ("PATH".into(), Some("/usr/bin:/bin".into())),
                ("PYTHONHOME".into(), None),
                ("LD_LIBRARY_PATH".into(), None),
                ("APPIMAGE".into(), None),
            ]
        );
    }
}
