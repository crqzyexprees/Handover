use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tauri::{Manager, RunEvent, State};
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

struct BackendState {
    port: u16,
    child: Mutex<Option<CommandChild>>,
}

#[tauri::command]
fn get_backend_port(state: State<'_, BackendState>) -> u16 {
    state.port
}

fn pick_free_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).context("failed to bind ephemeral port")?;
    Ok(listener.local_addr()?.port())
}

fn wait_for_backend(port: u16) -> Result<()> {
    let client = reqwest::blocking::Client::new();
    let url = format!("http://127.0.0.1:{port}/");
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        if let Ok(resp) = client.get(&url).send() {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    anyhow::bail!("handover-backend did not respond on {url}")
}

fn spawn_backend(app: &tauri::AppHandle, port: u16) -> Result<CommandChild> {
    let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into());
    let sidecar = app
        .shell()
        .sidecar("handover-backend")
        .context("handover-backend sidecar not found; run scripts/prepare-backend-sidecar.sh")?;
    let (_rx, child) = sidecar
        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
        .env("RUST_LOG", rust_log)
        .spawn()
        .context("failed to spawn handover-backend sidecar")?;
    wait_for_backend(port)?;
    Ok(child)
}

fn stop_backend(state: &BackendState) {
    if let Ok(mut guard) = state.child.lock() {
        if let Some(child) = guard.take() {
            let _ = child.kill();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                if let Some(icon) = app.default_window_icon().cloned() {
                    let _ = window.set_icon(icon);
                }
            }
            let port = pick_free_port()?;
            let child = spawn_backend(app.handle(), port)?;
            app.manage(BackendState {
                port,
                child: Mutex::new(Some(child)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_backend_port])
        .build(tauri::generate_context!());

    match result {
        Ok(app) => {
            app.run(|app_handle, event| {
                if matches!(event, RunEvent::Exit) {
                    if let Some(state) = app_handle.try_state::<BackendState>() {
                        stop_backend(&state);
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
