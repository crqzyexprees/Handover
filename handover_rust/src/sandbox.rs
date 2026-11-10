//! Sandbox Mode behind one seam (see GLOSSARY.md).
//!
//! Everything that differs between a `native` and a `docker` Instance lives
//! here: preparing its environment, tearing it down, pausing it, measuring
//! it, and the command its shell runs. Callers pick the adapter once from
//! the Instance's mode and never branch on the mode themselves.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use futures::future::BoxFuture;
use futures::FutureExt;
use portable_pty::CommandBuilder;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::docker::{DockerRuntime, SANDBOX_ENV_FILE};
use crate::model::{Instance, SandboxMode};
use crate::process_stats;

#[derive(Debug)]
pub enum SandboxError {
    /// The sandbox backend can't be used at all (e.g. Docker isn't running).
    Unavailable(String),
    Failed(anyhow::Error),
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(m) => write!(f, "{m}"),
            Self::Failed(e) => write!(f, "{e:#}"),
        }
    }
}

impl From<anyhow::Error> for SandboxError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed(e)
    }
}

pub type SandboxResult<T> = Result<T, SandboxError>;

/// What an Instance needs from its environment before its shell can start.
pub struct ProvisionRequest<'a> {
    pub instance_id: &'a str,
    pub project_path: &'a str,
    pub mem_limit: &'a str,
    pub custom_env_vars: &'a BTreeMap<String, String>,
}

pub trait Sandbox: Send + Sync {
    /// Prepares the environment; returns the container id, if one was made.
    fn provision<'a>(
        &'a self,
        req: ProvisionRequest<'a>,
    ) -> BoxFuture<'a, SandboxResult<Option<String>>>;
    /// Removes whatever `provision` created.
    fn teardown<'a>(&'a self, instance: &'a Instance) -> BoxFuture<'a, SandboxResult<()>>;
    /// Freezes the Instance. Returns false if this mode can't be suspended.
    fn suspend<'a>(&'a self, instance: &'a Instance) -> BoxFuture<'a, SandboxResult<bool>>;
    fn resume<'a>(&'a self, instance: &'a Instance) -> BoxFuture<'a, SandboxResult<()>>;
    /// `{mem_used_mb, mem_limit_mb, cpu_percent}`; zeros when unknown.
    fn stats<'a>(&'a self, instance: &'a Instance, shell_pid: Option<u32>) -> BoxFuture<'a, Value>;
}

/// The command an Instance's shell runs.
pub fn shell_command(instance: &Instance, cwd: Option<&Path>) -> anyhow::Result<CommandBuilder> {
    let mut c = match instance.sandbox_mode {
        SandboxMode::Native => {
            // Same flags the original Python pexpect backend used.
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
            let mut c = CommandBuilder::new(&shell);
            c.args(["-l", "-i"]);
            if let Some(cwd) = cwd {
                c.cwd(cwd);
            }
            for (k, v) in &instance.custom_env_vars {
                c.env(k, v);
            }
            c
        }
        SandboxMode::Docker => {
            let cid = instance.container_id().context("missing container_id")?;
            let mut c = CommandBuilder::new("docker");
            // Load the instance's secrets (see docker::SANDBOX_ENV_FILE), then
            // become the interactive shell.
            let start = format!(". {SANDBOX_ENV_FILE} 2>/dev/null; exec /bin/bash");
            c.args(["exec", "-it", cid, "/bin/bash", "-c", &start]);
            c
        }
    };
    c.env("TERM", "xterm-256color");
    Ok(c)
}

// ── Native ─────────────────────────────────────────────────────────────────

/// Runs directly on the host: nothing to provision, can't be frozen.
pub struct NativeSandbox;

impl Sandbox for NativeSandbox {
    fn provision<'a>(
        &'a self,
        _req: ProvisionRequest<'a>,
    ) -> BoxFuture<'a, SandboxResult<Option<String>>> {
        async { Ok(None) }.boxed()
    }

    fn teardown<'a>(&'a self, _instance: &'a Instance) -> BoxFuture<'a, SandboxResult<()>> {
        async { Ok(()) }.boxed()
    }

    fn suspend<'a>(&'a self, _instance: &'a Instance) -> BoxFuture<'a, SandboxResult<bool>> {
        async { Ok(false) }.boxed()
    }

    fn resume<'a>(&'a self, _instance: &'a Instance) -> BoxFuture<'a, SandboxResult<()>> {
        async { Ok(()) }.boxed()
    }

    fn stats<'a>(
        &'a self,
        _instance: &'a Instance,
        shell_pid: Option<u32>,
    ) -> BoxFuture<'a, Value> {
        async move {
            match shell_pid {
                Some(pid) => {
                    tokio::task::spawn_blocking(move || process_stats::process_tree_stats(pid))
                        .await
                        .unwrap_or_else(|_| process_stats::zeros())
                }
                None => process_stats::zeros(),
            }
        }
        .boxed()
    }
}

// ── Docker ─────────────────────────────────────────────────────────────────

/// One container per Instance. Connects to Docker lazily, so the app works
/// without Docker until a docker Instance is needed.
pub struct DockerSandbox {
    slot: Arc<RwLock<Option<Arc<DockerRuntime>>>>,
}

impl DockerSandbox {
    pub fn new(slot: Arc<RwLock<Option<Arc<DockerRuntime>>>>) -> Self {
        Self { slot }
    }

    pub async fn runtime(&self) -> SandboxResult<Arc<DockerRuntime>> {
        if let Some(rt) = self.slot.read().await.clone() {
            return Ok(rt);
        }
        let mut slot = self.slot.write().await;
        if let Some(rt) = slot.clone() {
            return Ok(rt);
        }
        let rt = Arc::new(
            DockerRuntime::new()
                .map_err(|e| SandboxError::Unavailable(format!("Docker is not available: {e}")))?,
        );
        *slot = Some(rt.clone());
        Ok(rt)
    }
}

impl Sandbox for DockerSandbox {
    fn provision<'a>(
        &'a self,
        req: ProvisionRequest<'a>,
    ) -> BoxFuture<'a, SandboxResult<Option<String>>> {
        async move {
            let rt = self.runtime().await?;
            let id = rt
                .start_container(
                    req.project_path,
                    req.instance_id,
                    req.mem_limit,
                    req.custom_env_vars,
                )
                .await?;
            Ok(Some(id))
        }
        .boxed()
    }

    fn teardown<'a>(&'a self, instance: &'a Instance) -> BoxFuture<'a, SandboxResult<()>> {
        async move {
            let rt = self.runtime().await?;
            if let Some(cid) = instance.container_id() {
                rt.stop_container(cid).await?;
            }
            rt.forget_instance(&instance.id);
            Ok(())
        }
        .boxed()
    }

    fn suspend<'a>(&'a self, instance: &'a Instance) -> BoxFuture<'a, SandboxResult<bool>> {
        async move {
            let Some(cid) = instance.container_id() else {
                return Ok(false);
            };
            self.runtime().await?.pause_container(cid).await?;
            Ok(true)
        }
        .boxed()
    }

    fn resume<'a>(&'a self, instance: &'a Instance) -> BoxFuture<'a, SandboxResult<()>> {
        async move {
            if let Some(cid) = instance.container_id() {
                self.runtime().await?.resume_container(cid).await?;
            }
            Ok(())
        }
        .boxed()
    }

    fn stats<'a>(
        &'a self,
        instance: &'a Instance,
        _shell_pid: Option<u32>,
    ) -> BoxFuture<'a, Value> {
        async move {
            match (self.runtime().await, instance.container_id()) {
                (Ok(rt), Some(cid)) => rt.get_container_stats(cid).await,
                _ => process_stats::zeros(),
            }
        }
        .boxed()
    }
}

// ── Registry ───────────────────────────────────────────────────────────────

/// The adapter for each Sandbox Mode.
#[derive(Clone)]
pub struct Sandboxes {
    native: Arc<dyn Sandbox>,
    docker: Arc<dyn Sandbox>,
}

impl Sandboxes {
    pub fn new(native: Arc<dyn Sandbox>, docker: Arc<dyn Sandbox>) -> Self {
        Self { native, docker }
    }

    pub fn for_mode(&self, mode: SandboxMode) -> &dyn Sandbox {
        match mode {
            SandboxMode::Native => self.native.as_ref(),
            SandboxMode::Docker => self.docker.as_ref(),
        }
    }

    pub fn for_instance(&self, instance: &Instance) -> &dyn Sandbox {
        self.for_mode(instance.sandbox_mode)
    }
}

#[cfg(test)]
pub mod fake {
    //! A recording sandbox for tests that don't need Docker.
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct FakeSandbox {
        pub calls: Mutex<Vec<String>>,
        pub container: Option<String>,
        pub fail_provision: bool,
    }

    impl FakeSandbox {
        fn record(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }
        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Sandbox for FakeSandbox {
        fn provision<'a>(
            &'a self,
            req: ProvisionRequest<'a>,
        ) -> BoxFuture<'a, SandboxResult<Option<String>>> {
            async move {
                self.record(format!("provision {}", req.instance_id));
                if self.fail_provision {
                    return Err(SandboxError::Unavailable("fake unavailable".into()));
                }
                Ok(self.container.clone())
            }
            .boxed()
        }
        fn teardown<'a>(&'a self, i: &'a Instance) -> BoxFuture<'a, SandboxResult<()>> {
            async move {
                self.record(format!("teardown {}", i.id));
                Ok(())
            }
            .boxed()
        }
        fn suspend<'a>(&'a self, i: &'a Instance) -> BoxFuture<'a, SandboxResult<bool>> {
            async move {
                self.record(format!("suspend {}", i.id));
                Ok(true)
            }
            .boxed()
        }
        fn resume<'a>(&'a self, i: &'a Instance) -> BoxFuture<'a, SandboxResult<()>> {
            async move {
                self.record(format!("resume {}", i.id));
                Ok(())
            }
            .boxed()
        }
        fn stats<'a>(&'a self, i: &'a Instance, _pid: Option<u32>) -> BoxFuture<'a, Value> {
            async move {
                self.record(format!("stats {}", i.id));
                serde_json::json!({ "mem_used_mb": 1.0, "mem_limit_mb": 2.0, "cpu_percent": 3.0 })
            }
            .boxed()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(mode: SandboxMode, container: Option<&str>) -> Instance {
        Instance {
            id: "i".into(),
            project_id: "p".into(),
            sandbox_mode: mode,
            container_id: container.map(String::from),
            mem_limit: None,
            custom_env_vars: BTreeMap::from([("K".into(), "v".into())]),
            opened_at: 0.0,
        }
    }

    #[test]
    fn docker_shell_execs_into_its_container() {
        let cmd = shell_command(&instance(SandboxMode::Docker, Some("abc")), None).unwrap();
        let argv: Vec<String> = cmd
            .get_argv()
            .iter()
            .map(|a| a.to_string_lossy().into())
            .collect();
        assert_eq!(&argv[..5], ["docker", "exec", "-it", "abc", "/bin/bash"]);
        assert!(argv[6].contains(SANDBOX_ENV_FILE));
    }

    #[test]
    fn docker_shell_without_container_is_an_error() {
        assert!(shell_command(&instance(SandboxMode::Docker, None), None).is_err());
    }

    #[test]
    fn native_shell_gets_custom_env() {
        let cmd = shell_command(&instance(SandboxMode::Native, None), None).unwrap();
        assert_eq!(
            cmd.get_env("K").map(|v| v.to_string_lossy().into_owned()),
            Some("v".into())
        );
    }

    #[tokio::test]
    async fn native_cannot_be_suspended() {
        let suspended = NativeSandbox
            .suspend(&instance(SandboxMode::Native, None))
            .await
            .unwrap();
        assert!(!suspended);
    }
}
