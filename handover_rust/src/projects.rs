//! Projects and their Instances (see GLOSSARY.md), and the rules that tie
//! them together.
//!
//! [`Projects`] is the only owner of this state. Callers (HTTP handlers, the
//! Governor) ask for whole operations: create, configure, activate, resume,
//! suspend, unload, open and close an Instance. Lock ordering, persistence,
//! the Sandbox, and ending shells are handled here, so each rule lives in
//! one place:
//!
//! - A Project's display name is its config's `project_name`.
//! - A new Instance takes its settings from the Project's effective config.
//! - Instances live in memory only. After a backend restart nothing is
//!   paused, so no Project comes back Suspended.
//! - Closing an Instance ends its shell before tearing down its Sandbox, and
//!   a shell that was still starting can't outlive it.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use portable_pty::CommandBuilder;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};
use tracing::warn;
use uuid::Uuid;

use crate::local_config;
use crate::model::{
    basename_from_path, Instance, Project, ProjectConfig, ProjectState, ResourceStats, SandboxMode,
};
use crate::pty::Terminals;
use crate::sandbox::{ProvisionRequest, SandboxError, Sandboxes};
use crate::state::{self, monotonic_secs, Snapshot};

#[derive(Debug)]
pub enum ProjectError {
    ProjectNotFound,
    InstanceNotFound,
    Sandbox(SandboxError),
    Storage(anyhow::Error),
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProjectNotFound => write!(f, "Project not found"),
            Self::InstanceNotFound => write!(f, "Instance not found"),
            Self::Sandbox(e) => write!(f, "{e}"),
            Self::Storage(e) => write!(f, "{e:#}"),
        }
    }
}

impl From<SandboxError> for ProjectError {
    fn from(e: SandboxError) -> Self {
        Self::Sandbox(e)
    }
}

pub type ProjectResult<T> = Result<T, ProjectError>;

/// A Project as the UI sees it: its record (named after its config), its
/// effective config, and its open Instances oldest first.
#[derive(Serialize, Debug)]
pub struct ProjectView {
    #[serde(flatten)]
    pub project: Project,
    pub config: ProjectConfig,
    pub instances: Vec<Instance>,
}

/// One Instance in a Project's resource report.
pub struct InstanceResources {
    pub instance: Instance,
    pub shell_running: bool,
    pub stats: ResourceStats,
}

#[derive(Default)]
struct Inner {
    projects: HashMap<String, Project>,
    configs: HashMap<String, ProjectConfig>,
    instances: HashMap<String, Instance>,
    last_active: HashMap<String, f64>,
    focused: Option<String>,
    app_config: Value,
}

impl Inner {
    fn instances_of(&self, project_id: &str) -> Vec<Instance> {
        let mut list: Vec<Instance> = self
            .instances
            .values()
            .filter(|i| i.project_id == project_id)
            .cloned()
            .collect();
        list.sort_by(|a, b| {
            a.opened_at
                .total_cmp(&b.opened_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        list
    }

    fn stored_config(&self, project: &Project) -> ProjectConfig {
        self.configs
            .get(&project.id)
            .cloned()
            .unwrap_or_else(|| ProjectConfig::defaults_for(&project.path))
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            projects: self.projects.clone(),
            configs: self.configs.clone(),
            last_active: self.last_active.clone(),
            app_config: self.app_config.clone(),
        }
    }
}

pub struct Projects {
    inner: RwLock<Inner>,
    persist_path: PathBuf,
    /// Held across snapshot + write so saves land in order.
    save_lock: Mutex<()>,
    sandboxes: Sandboxes,
    terminals: Arc<Terminals>,
}

impl Projects {
    /// Loads saved Projects (a missing or unreadable file starts empty).
    pub async fn load(
        persist_path: PathBuf,
        sandboxes: Sandboxes,
        terminals: Arc<Terminals>,
    ) -> Arc<Self> {
        let snapshot = match state::load(&persist_path).await {
            Ok(s) => s,
            Err(e) => {
                warn!("[startup] failed to load saved state: {e:#}");
                Snapshot::default()
            }
        };
        let mut projects = snapshot.projects;
        // Instances aren't saved, so nothing is paused after a restart.
        for p in projects.values_mut() {
            p.state = ProjectState::Active;
        }
        Arc::new(Self {
            inner: RwLock::new(Inner {
                projects,
                configs: snapshot.configs,
                instances: HashMap::new(),
                last_active: snapshot.last_active,
                focused: None,
                app_config: snapshot.app_config,
            }),
            persist_path,
            save_lock: Mutex::new(()),
            sandboxes,
            terminals,
        })
    }

    async fn persist(&self) -> ProjectResult<()> {
        let _order = self.save_lock.lock().await;
        let snapshot = self.inner.read().await.snapshot();
        state::save(&self.persist_path, &snapshot)
            .await
            .map_err(ProjectError::Storage)
    }

    async fn persist_or_warn(&self, what: &str) {
        if let Err(e) = self.persist().await {
            warn!("failed to save state after {what}: {e}");
        }
    }

    // ── Reading ────────────────────────────────────────────────────────────

    pub async fn project(&self, project_id: &str) -> ProjectResult<Project> {
        self.inner
            .read()
            .await
            .projects
            .get(project_id)
            .cloned()
            .ok_or(ProjectError::ProjectNotFound)
    }

    pub async fn instance(&self, instance_id: &str) -> ProjectResult<Instance> {
        self.inner
            .read()
            .await
            .instances
            .get(instance_id)
            .cloned()
            .ok_or(ProjectError::InstanceNotFound)
    }

    /// A Project's Instances, oldest first (the order its tabs show).
    pub async fn instances_of(&self, project_id: &str) -> Vec<Instance> {
        self.inner.read().await.instances_of(project_id)
    }

    /// Whether any Instance exists (running shells or not).
    #[cfg(test)]
    pub async fn has_instances(&self) -> bool {
        !self.inner.read().await.instances.is_empty()
    }

    /// The saved config with the project folder's `config.yml` on top.
    pub async fn effective_config(&self, project_id: &str) -> ProjectResult<ProjectConfig> {
        let (project, stored) = {
            let inner = self.inner.read().await;
            let project = inner
                .projects
                .get(project_id)
                .cloned()
                .ok_or(ProjectError::ProjectNotFound)?;
            let stored = inner.stored_config(&project);
            (project, stored)
        };
        local_config::effective_config(&project.path, stored)
            .await
            .map_err(ProjectError::Storage)
    }

    fn view(mut project: Project, config: ProjectConfig, instances: Vec<Instance>) -> ProjectView {
        project.name = display_name(&config, &project);
        ProjectView {
            project,
            config,
            instances,
        }
    }

    pub async fn list(&self) -> Vec<ProjectView> {
        let entries: Vec<(Project, ProjectConfig, Vec<Instance>)> = {
            let inner = self.inner.read().await;
            inner
                .projects
                .values()
                .map(|p| (p.clone(), inner.stored_config(p), inner.instances_of(&p.id)))
                .collect()
        };
        let mut out = Vec::with_capacity(entries.len());
        for (project, stored, instances) in entries {
            let config = match local_config::effective_config(&project.path, stored.clone()).await {
                Ok(c) => c,
                Err(e) => {
                    warn!("ignoring unreadable config.yml for {}: {e:#}", project.path);
                    stored
                }
            };
            out.push(Self::view(project, config, instances));
        }
        out
    }

    // ── Project lifecycle ──────────────────────────────────────────────────

    /// `path` must already be validated (an existing directory).
    pub async fn create(
        &self,
        path: String,
        sandbox_mode: SandboxMode,
    ) -> ProjectResult<ProjectView> {
        let project = Project {
            id: Uuid::new_v4().to_string(),
            name: basename_from_path(&path),
            path,
            sandbox_mode,
            state: ProjectState::Active,
        };
        let mut config = ProjectConfig::defaults_for(&project.path);
        config.sandbox_mode = sandbox_mode;
        {
            let mut inner = self.inner.write().await;
            inner.projects.insert(project.id.clone(), project.clone());
            inner.configs.insert(project.id.clone(), config.clone());
            inner
                .last_active
                .insert(project.id.clone(), monotonic_secs());
        }
        if let Err(e) = local_config::write_local_config(&project.path, &config).await {
            warn!("failed to write .handover/config.yml on create: {e}");
        }
        self.persist().await?;
        Ok(Self::view(project, config, Vec::new()))
    }

    /// Saves new settings to the app and to the project's `config.yml`.
    pub async fn configure(
        &self,
        project_id: &str,
        mut config: ProjectConfig,
    ) -> ProjectResult<ProjectConfig> {
        let path = {
            let mut inner = self.inner.write().await;
            let project = inner
                .projects
                .get_mut(project_id)
                .ok_or(ProjectError::ProjectNotFound)?;
            if config.project_name.trim().is_empty() {
                config.project_name = basename_from_path(&project.path);
            }
            project.name = config.project_name.clone();
            let path = project.path.clone();
            inner.configs.insert(project_id.to_string(), config.clone());
            path
        };
        local_config::write_local_config(&path, &config)
            .await
            .map_err(ProjectError::Storage)?;
        self.persist().await?;
        Ok(config)
    }

    /// Focuses the Project (the Governor never suspends it), resuming it if
    /// it was suspended.
    pub async fn activate(&self, project_id: &str) -> ProjectResult<()> {
        let suspended = {
            let mut inner = self.inner.write().await;
            let state = inner
                .projects
                .get(project_id)
                .ok_or(ProjectError::ProjectNotFound)?
                .state;
            inner.focused = Some(project_id.to_string());
            inner
                .last_active
                .insert(project_id.to_string(), monotonic_secs());
            state == ProjectState::Suspended
        };
        if suspended {
            if let Err(e) = self.resume_instances(project_id).await {
                warn!("failed to resume instances while activating project: {e}");
            }
        }
        self.persist().await
    }

    /// Unfreezes every Instance; fails if any can't be resumed.
    pub async fn resume(&self, project_id: &str) -> ProjectResult<()> {
        self.project(project_id).await?;
        self.resume_instances(project_id).await?;
        self.inner
            .write()
            .await
            .last_active
            .insert(project_id.to_string(), monotonic_secs());
        self.persist().await
    }

    async fn resume_instances(&self, project_id: &str) -> Result<(), SandboxError> {
        for instance in self.instances_of(project_id).await {
            self.sandboxes
                .for_instance(&instance)
                .resume(&instance)
                .await?;
        }
        if let Some(p) = self.inner.write().await.projects.get_mut(project_id) {
            p.state = ProjectState::Active;
        }
        Ok(())
    }

    /// Resumes the Project if it is suspended, so prompts can be delivered.
    pub async fn ensure_active(&self, project_id: &str) -> ProjectResult<()> {
        if self.project(project_id).await?.state == ProjectState::Suspended {
            self.resume_instances(project_id).await?;
            self.persist_or_warn("resuming a project").await;
        }
        Ok(())
    }

    /// Freezes the Project's Instances. False if nothing could be paused.
    pub async fn suspend(&self, project_id: &str) -> bool {
        let mut paused = 0;
        for instance in self.instances_of(project_id).await {
            match self
                .sandboxes
                .for_instance(&instance)
                .suspend(&instance)
                .await
            {
                Ok(true) => paused += 1,
                Ok(false) => {}
                Err(e) => warn!("failed to suspend instance {}: {e}", instance.id),
            }
        }
        if paused == 0 {
            return false;
        }
        if let Some(p) = self.inner.write().await.projects.get_mut(project_id) {
            p.state = ProjectState::Suspended;
        }
        self.persist_or_warn("suspending a project").await;
        true
    }

    /// Projects the Governor may suspend, least recently active first: not
    /// focused, not already suspended, and with at least one container.
    pub async fn suspend_candidates(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        let mut candidates: Vec<(f64, String)> = inner
            .projects
            .values()
            .filter(|p| inner.focused.as_deref() != Some(p.id.as_str()))
            .filter(|p| p.state != ProjectState::Suspended)
            .filter(|p| {
                inner
                    .instances
                    .values()
                    .any(|i| i.project_id == p.id && i.container_id().is_some())
            })
            .map(|p| {
                let t = inner.last_active.get(&p.id).copied().unwrap_or(0.0);
                (t, p.id.clone())
            })
            .collect();
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
        candidates.into_iter().map(|(_, id)| id).collect()
    }

    /// Closes every Instance and forgets the Project. False if it was unknown.
    pub async fn unload(&self, project_id: &str) -> bool {
        let removed: Vec<Instance> = {
            let mut inner = self.inner.write().await;
            if inner.projects.remove(project_id).is_none() {
                return false;
            }
            inner.configs.remove(project_id);
            inner.last_active.remove(project_id);
            if inner.focused.as_deref() == Some(project_id) {
                inner.focused = None;
            }
            let ids: Vec<String> = inner
                .instances
                .values()
                .filter(|i| i.project_id == project_id)
                .map(|i| i.id.clone())
                .collect();
            ids.iter()
                .filter_map(|id| inner.instances.remove(id))
                .collect()
        };
        for instance in removed {
            self.release(&instance).await;
        }
        self.persist_or_warn("unloading a project").await;
        true
    }

    // ── Instances ──────────────────────────────────────────────────────────

    /// Opens an Instance with the Project's effective config; `mode`
    /// overrides the config's Sandbox Mode.
    pub async fn open_instance(
        &self,
        project_id: &str,
        mode: Option<SandboxMode>,
    ) -> ProjectResult<Instance> {
        let project = self.project(project_id).await?;
        let config = self.effective_config(project_id).await?;
        let sandbox_mode = mode.unwrap_or(config.sandbox_mode);
        let id = Uuid::new_v4().to_string();
        let container_id = self
            .sandboxes
            .for_mode(sandbox_mode)
            .provision(ProvisionRequest {
                instance_id: &id,
                project_path: &project.path,
                mem_limit: &config.mem_limit,
                custom_env_vars: &config.custom_env_vars,
            })
            .await?;
        let instance = Instance {
            id,
            project_id: project.id,
            sandbox_mode,
            // Only a container enforces the limit; don't report one otherwise.
            mem_limit: container_id.as_ref().map(|_| config.mem_limit),
            container_id,
            custom_env_vars: config.custom_env_vars,
            opened_at: monotonic_secs(),
        };
        let kept = {
            let mut inner = self.inner.write().await;
            // The Project may have been unloaded while provisioning.
            let exists = inner.projects.contains_key(project_id);
            if exists {
                inner
                    .instances
                    .insert(instance.id.clone(), instance.clone());
            }
            exists
        };
        if !kept {
            self.release(&instance).await;
            return Err(ProjectError::ProjectNotFound);
        }
        Ok(instance)
    }

    /// Ends the Instance's shell and removes its Sandbox. False if unknown.
    pub async fn close_instance(self: &Arc<Self>, instance_id: &str) -> bool {
        let Some(instance) = self.inner.write().await.instances.remove(instance_id) else {
            return false;
        };
        self.terminals.shutdown(instance_id).await;
        let this = self.clone();
        // Removing a container can take seconds; the UI needn't wait.
        tokio::spawn(async move {
            if let Err(e) = this
                .sandboxes
                .for_instance(&instance)
                .teardown(&instance)
                .await
            {
                warn!("failed to tear down closed instance: {e}");
            }
        });
        true
    }

    async fn release(&self, instance: &Instance) {
        self.terminals.shutdown(&instance.id).await;
        if let Err(e) = self
            .sandboxes
            .for_instance(instance)
            .teardown(instance)
            .await
        {
            warn!("failed to tear down instance {}: {e}", instance.id);
        }
    }

    /// The command that starts the Instance's shell, in its Project folder.
    pub async fn shell_command(&self, instance_id: &str) -> Result<CommandBuilder, String> {
        let (instance, project_path) = {
            let inner = self.inner.read().await;
            let instance = inner
                .instances
                .get(instance_id)
                .cloned()
                .ok_or("Instance not found. Close this terminal and open a new one.")?;
            let path = inner
                .projects
                .get(&instance.project_id)
                .map(|p| p.path.clone());
            (instance, path)
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
        self.sandboxes
            .for_instance(&instance)
            .shell_command(&instance, cwd.as_deref())
            .map_err(|e| e.to_string())
    }

    /// What the Instance is using now.
    pub async fn instance_stats(&self, instance_id: &str) -> ProjectResult<ResourceStats> {
        let instance = self.instance(instance_id).await?;
        let pid = self
            .terminals
            .running_shells(&[instance_id])
            .await
            .remove(instance_id)
            .flatten();
        Ok(self
            .sandboxes
            .for_instance(&instance)
            .stats(&instance, pid)
            .await)
    }

    /// Each of the Project's Instances (oldest first), whether its shell is
    /// running, and what it is using.
    pub async fn resources(&self, project_id: &str) -> ProjectResult<Vec<InstanceResources>> {
        self.project(project_id).await?;
        let instances = self.instances_of(project_id).await;
        let ids: Vec<&str> = instances.iter().map(|i| i.id.as_str()).collect();
        let shells = self.terminals.running_shells(&ids).await;
        let stats = futures::future::join_all(instances.iter().map(|i| {
            let pid = shells.get(&i.id).copied().flatten();
            self.sandboxes.for_instance(i).stats(i, pid)
        }))
        .await;
        Ok(instances
            .into_iter()
            .zip(stats)
            .map(|(instance, stats)| InstanceResources {
                shell_running: shells.contains_key(&instance.id),
                instance,
                stats,
            })
            .collect())
    }

    /// Whether `mode` can be used right now.
    pub async fn sandbox_health(&self, mode: SandboxMode) -> Result<(), SandboxError> {
        self.sandboxes.for_mode(mode).health().await
    }

    /// Closes every Instance (on backend exit) and saves.
    pub async fn shutdown_all(&self) {
        let all: Vec<Instance> = self
            .inner
            .write()
            .await
            .instances
            .drain()
            .map(|(_, i)| i)
            .collect();
        for instance in all {
            self.release(&instance).await;
        }
        self.persist_or_warn("shutting down").await;
    }

    // ── App settings ───────────────────────────────────────────────────────

    pub async fn app_config(&self) -> Value {
        self.inner.read().await.app_config.clone()
    }

    pub async fn update_app_config(&self, patch: BTreeMap<String, Value>) {
        {
            let mut inner = self.inner.write().await;
            if let Some(base) = inner.app_config.as_object_mut() {
                base.extend(patch);
            }
        }
        self.persist_or_warn("updating app settings").await;
    }
}

fn display_name(config: &ProjectConfig, project: &Project) -> String {
    [config.project_name.as_str(), project.name.as_str()]
        .into_iter()
        .find(|s| !s.trim().is_empty())
        .map(String::from)
        .unwrap_or_else(|| basename_from_path(&project.path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::fake::FakeSandbox;
    use crate::sandbox::NativeSandbox;

    struct Fixture {
        dir: tempfile::TempDir,
        folder: String,
        state_file: PathBuf,
        docker: Arc<FakeSandbox>,
        projects: Arc<Projects>,
    }

    impl Fixture {
        /// Another project folder (each has its own config.yml).
        fn folder(&self, name: &str) -> String {
            let path = self.dir.path().join(name);
            std::fs::create_dir_all(&path).unwrap();
            path.to_str().unwrap().into()
        }
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("proj");
        std::fs::create_dir(&folder).unwrap();
        let docker = Arc::new(FakeSandbox {
            container: Some("c1".into()),
            ..Default::default()
        });
        let state_file = dir.path().join("state.json");
        let projects = Projects::load(
            state_file.clone(),
            Sandboxes::new(Arc::new(NativeSandbox), docker.clone()),
            Terminals::new(),
        )
        .await;
        Fixture {
            folder: folder.to_str().unwrap().into(),
            dir,
            state_file,
            docker,
            projects,
        }
    }

    #[tokio::test]
    async fn governor_suspends_least_recent_unfocused_project_with_containers() {
        let f = fixture().await;
        let a = f
            .projects
            .create(f.folder("a"), SandboxMode::Docker)
            .await
            .unwrap();
        let b = f
            .projects
            .create(f.folder("b"), SandboxMode::Docker)
            .await
            .unwrap();
        let native = f
            .projects
            .create(f.folder("n"), SandboxMode::Native)
            .await
            .unwrap();
        for p in [&a, &b] {
            f.projects.open_instance(&p.project.id, None).await.unwrap();
        }
        f.projects
            .open_instance(&native.project.id, None)
            .await
            .unwrap();
        f.projects.activate(&b.project.id).await.unwrap();

        let candidates = f.projects.suspend_candidates().await;
        assert_eq!(candidates, std::slice::from_ref(&a.project.id));
        assert!(f.projects.suspend(&a.project.id).await);
        assert_eq!(
            f.projects.project(&a.project.id).await.unwrap().state,
            ProjectState::Suspended
        );
        assert!(f.projects.suspend_candidates().await.is_empty());
    }

    #[tokio::test]
    async fn suspended_state_does_not_survive_a_restart() {
        let f = fixture().await;
        let p = f
            .projects
            .create(f.folder.clone(), SandboxMode::Docker)
            .await
            .unwrap();
        f.projects.open_instance(&p.project.id, None).await.unwrap();
        assert!(f.projects.suspend(&p.project.id).await);

        let reloaded = Projects::load(
            f.state_file.clone(),
            Sandboxes::new(Arc::new(NativeSandbox), Arc::new(FakeSandbox::default())),
            Terminals::new(),
        )
        .await;
        let project = reloaded.project(&p.project.id).await.unwrap();
        assert_eq!(project.state, ProjectState::Active);
    }

    #[tokio::test]
    async fn unload_tears_down_every_instance_and_forgets_the_project() {
        let f = fixture().await;
        let p = f
            .projects
            .create(f.folder.clone(), SandboxMode::Docker)
            .await
            .unwrap();
        let i = f.projects.open_instance(&p.project.id, None).await.unwrap();
        assert!(f.projects.unload(&p.project.id).await);
        assert!(f.docker.calls().contains(&format!("teardown {}", i.id)));
        assert!(matches!(
            f.projects.instance(&i.id).await,
            Err(ProjectError::InstanceNotFound)
        ));
        assert!(f.projects.list().await.is_empty());
        assert!(!f.projects.unload(&p.project.id).await);
    }

    #[tokio::test]
    async fn configure_renames_the_project() {
        let f = fixture().await;
        let p = f
            .projects
            .create(f.folder.clone(), SandboxMode::Docker)
            .await
            .unwrap();
        let mut cfg = p.config.clone();
        cfg.project_name = "Renamed".into();
        f.projects.configure(&p.project.id, cfg).await.unwrap();
        assert_eq!(f.projects.list().await[0].project.name, "Renamed");
        cfg = ProjectConfig::defaults_for("/x");
        cfg.project_name = "  ".into();
        let saved = f.projects.configure(&p.project.id, cfg).await.unwrap();
        assert_eq!(saved.project_name, "proj");
    }

    #[tokio::test]
    async fn ensure_active_resumes_a_suspended_project() {
        let f = fixture().await;
        let p = f
            .projects
            .create(f.folder.clone(), SandboxMode::Docker)
            .await
            .unwrap();
        f.projects.open_instance(&p.project.id, None).await.unwrap();
        f.projects.suspend(&p.project.id).await;
        f.projects.ensure_active(&p.project.id).await.unwrap();
        assert!(f.docker.calls().iter().any(|c| c.starts_with("resume ")));
        assert_eq!(
            f.projects.project(&p.project.id).await.unwrap().state,
            ProjectState::Active
        );
    }
}
