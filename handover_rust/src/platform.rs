use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// Strip Windows extended-length `\\?\` prefixes returned by `canonicalize`.
/// Docker, PowerShell, and many other tools reject that form.
pub fn normalize_storage_path(path: &str) -> String {
    let path = path.trim();
    #[cfg(windows)]
    {
        if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{}", rest.replace('/', "\\"));
        }
        if let Some(rest) = path.strip_prefix(r"\\?\") {
            return rest.replace('/', "\\");
        }
        path.replace('/', "\\")
    }
    #[cfg(not(windows))]
    {
        path.to_string()
    }
}

/// Host-side path formatted for a Docker bind mount.
pub fn docker_bind_source(path: &str) -> String {
    let normalized = normalize_storage_path(path);
    #[cfg(windows)]
    {
        // Docker Desktop on Windows accepts forward slashes in volume paths.
        normalized.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        normalized
    }
}

/// Bind-mount option suffix. SELinux `:z` relabeling is Linux-only.
pub fn docker_bind_options() -> &'static str {
    #[cfg(windows)]
    {
        "rw"
    }
    #[cfg(not(windows))]
    {
        "rw,z"
    }
}

pub fn docker_bind(host: &str, container: &str, mode: &str) -> String {
    format!(
        "{}:{}:{}",
        docker_bind_source(host),
        container,
        mode
    )
}

pub fn docker_bind_rw(host: &str, container: &str) -> String {
    docker_bind(host, container, docker_bind_options())
}

pub fn docker_bind_ro(host: &str, container: &str) -> String {
    #[cfg(windows)]
    {
        docker_bind(host, container, "ro")
    }
    #[cfg(not(windows))]
    {
        docker_bind(host, container, "ro,z")
    }
}

pub fn docker_cli() -> String {
    #[cfg(windows)]
    {
        if let Some(path) = which("docker.exe") {
            return path.to_string_lossy().into_owned();
        }
        for candidate in [
            r"C:\Program Files\Docker\Docker\resources\bin\docker.exe",
            r"C:\Program Files\Docker\Docker\resources\docker.exe",
        ] {
            let path = PathBuf::from(candidate);
            if path.is_file() {
                return path.to_string_lossy().into_owned();
            }
        }
        "docker.exe".to_string()
    }
    #[cfg(not(windows))]
    {
        "docker".to_string()
    }
}

pub fn home_dir() -> Option<PathBuf> {
    for key in ["HOME", "USERPROFILE"] {
        if let Ok(home) = std::env::var(key) {
            let path = PathBuf::from(home);
            if path.is_dir() {
                return Some(path);
            }
        }
    }
    None
}

pub fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        users::get_current_uid()
    }
    #[cfg(windows)]
    {
        1000
    }
}

pub fn current_gid() -> u32 {
    #[cfg(unix)]
    {
        users::get_current_gid()
    }
    #[cfg(windows)]
    {
        1000
    }
}

pub fn current_username() -> String {
    #[cfg(unix)]
    {
        users::get_current_username()
            .map(|u| u.to_string_lossy().into_owned())
            .unwrap_or_else(|| "sandbox".into())
    }
    #[cfg(windows)]
    {
        std::env::var("USERNAME").unwrap_or_else(|_| "sandbox".into())
    }
}

/// The interactive login shell used for `native` sandbox terminals.
///
/// On Unix this is the user's `$SHELL` (falling back to bash). On Windows there
/// is no `/bin/bash`, so we prefer PowerShell and fall back to `cmd.exe`.
pub fn native_shell() -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        // Prefer PowerShell for a nicer interactive experience.
        if let Some(ps) = which("powershell.exe") {
            return (ps.to_string_lossy().into_owned(), vec!["-NoLogo".into()]);
        }
        if let Ok(comspec) = std::env::var("COMSPEC") {
            if which(&comspec).is_some() {
                return (comspec, Vec::new());
            }
        }
        ("cmd.exe".into(), Vec::new())
    }
    #[cfg(unix)]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        (shell, vec!["-l".into(), "-i".into()])
    }
}

/// Returns true if `docker` responds to a lightweight `docker info` call,
/// meaning the daemon/engine is reachable.
pub fn docker_running() -> bool {
    Command::new(docker_cli())
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Attempts to start Docker if it is installed but not running, then waits for
/// the engine to become reachable. Returns `Ok(true)` if Docker is running by
/// the end (either it already was, or we started it), `Ok(false)` if we could
/// not start it, and `Err` only for unexpected failures.
///
/// This is a best-effort convenience: on Windows it launches Docker Desktop, on
/// macOS it opens the Docker app, and on Linux it tries `systemctl start docker`.
pub fn ensure_docker_running() -> anyhow::Result<bool> {
    if docker_running() {
        return Ok(true);
    }

    let launched = launch_docker();
    if !launched {
        return Ok(false);
    }

    // Docker Desktop can take a while to boot the engine on Windows/macOS.
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        if docker_running() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_secs(2));
    }

    Ok(docker_running())
}

fn launch_docker() -> bool {
    #[cfg(windows)]
    {
        for candidate in docker_desktop_paths() {
            if candidate.exists() {
                if Command::new(&candidate).spawn().is_ok() {
                    return true;
                }
            }
        }
        // Fall back to letting the shell resolve it (e.g. via PATH / start).
        Command::new("cmd")
            .args(["/C", "start", "", "Docker Desktop"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-a", "Docker"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Try starting the daemon via systemd; ignore failures (may need sudo).
        Command::new("systemctl")
            .args(["start", "docker"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

#[cfg(windows)]
fn docker_desktop_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(pf) = std::env::var("ProgramFiles") {
        paths.push(PathBuf::from(pf).join(r"Docker\Docker\Docker Desktop.exe"));
    }
    if let Ok(pf) = std::env::var("ProgramFiles(x86)") {
        paths.push(PathBuf::from(pf).join(r"Docker\Docker\Docker Desktop.exe"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        paths.push(PathBuf::from(local).join(r"Docker\Docker Desktop.exe"));
    }
    paths
}

#[cfg(windows)]
fn which(program: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(program);
    if candidate.is_absolute() && candidate.exists() {
        return Some(candidate);
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let full = dir.join(program);
        if full.exists() {
            return Some(full);
        }
    }
    None
}
