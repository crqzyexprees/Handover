use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::fs;
use tokio::sync::{Mutex, RwLock};

use tracing::warn;

use crate::model::{Instance, Project, ProjectConfig};
use crate::pty::PtySession;

pub const GOVERNOR_INTERVAL_SECS: u64 = 15;
pub const RAM_SUSPEND_THRESHOLD: f32 = 85.0;
pub const RAM_EMERGENCY_THRESHOLD: f32 = 95.0;

pub struct AppState {
    pub projects: RwLock<HashMap<String, Project>>,
    pub project_configs: RwLock<HashMap<String, ProjectConfig>>,
    pub instances: RwLock<HashMap<String, Instance>>,
    /// Serializes shell startup per instance (held only while spawning).
    pub ws_session_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Running shells by instance id; they outlive client connections.
    pub pty_sessions: RwLock<HashMap<String, Arc<PtySession>>>,
    pub focused_project_id: RwLock<Option<String>>,
    pub project_last_active: RwLock<HashMap<String, f64>>,
    pub app_config: RwLock<Value>,
    /// Where `state.json` lives.
    persist_path: PathBuf,
    /// Unix seconds of the last authenticated request.
    last_activity: std::sync::atomic::AtomicU64,
}

/// On disk, entries stay untyped so one malformed project can be skipped
/// instead of making the whole file unreadable.
#[derive(Debug, Default, Deserialize, Serialize)]
struct PersistedState {
    #[serde(default)]
    projects: HashMap<String, Value>,
    #[serde(default)]
    project_configs: HashMap<String, Value>,
    #[serde(default)]
    project_last_active: HashMap<String, f64>,
    #[serde(default)]
    app_config: Option<Value>,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        Self::with_persist_path(state_path())
    }

    pub fn with_persist_path(persist_path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            persist_path,
            last_activity: std::sync::atomic::AtomicU64::new(monotonic_secs() as u64),
            projects: RwLock::new(HashMap::new()),
            project_configs: RwLock::new(HashMap::new()),
            instances: RwLock::new(HashMap::new()),
            ws_session_locks: Mutex::new(HashMap::new()),
            pty_sessions: RwLock::new(HashMap::new()),
            focused_project_id: RwLock::new(None),
            project_last_active: RwLock::new(HashMap::new()),
            app_config: RwLock::new(json!({
                "clis_enabled": ["claude", "codex", "cursor"],
                "auto_suspend_delay": 300,
                "container_mem_limit": "2g",
            })),
        })
    }

    pub async fn load_persisted(&self) -> anyhow::Result<()> {
        self.load_persisted_from_path(self.persist_path.clone())
            .await
    }

    async fn load_persisted_from_path(&self, path: PathBuf) -> anyhow::Result<()> {
        if !path.exists() {
            return Ok(());
        }
        let content = fs::read_to_string(&path)
            .await
            .with_context(|| format!("failed to read {}", path.display()))?;
        let persisted: PersistedState = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;

        *self.projects.write().await = typed_entries(persisted.projects, "project");
        *self.project_configs.write().await =
            typed_entries(persisted.project_configs, "project config");
        *self.project_last_active.write().await = persisted.project_last_active;
        if let Some(app_config) = persisted.app_config {
            *self.app_config.write().await = app_config;
        }
        Ok(())
    }

    pub async fn save_persisted(&self) -> anyhow::Result<()> {
        self.save_persisted_to_path(self.persist_path.clone()).await
    }

    async fn save_persisted_to_path(&self, path: PathBuf) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let snapshot = PersistedState {
            projects: untyped_entries(&*self.projects.read().await),
            project_configs: untyped_entries(&*self.project_configs.read().await),
            project_last_active: self.project_last_active.read().await.clone(),
            app_config: Some(self.app_config.read().await.clone()),
        };
        let content = serde_json::to_string_pretty(&snapshot)?;
        fs::write(&path, content)
            .await
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }
}

fn typed_entries<T: serde::de::DeserializeOwned>(
    raw: HashMap<String, Value>,
    what: &str,
) -> HashMap<String, T> {
    raw.into_iter()
        .filter_map(|(id, v)| match serde_json::from_value(v) {
            Ok(t) => Some((id, t)),
            Err(e) => {
                warn!("[state] skipping unreadable {what} {id}: {e}");
                None
            }
        })
        .collect()
}

fn untyped_entries<T: Serialize>(map: &HashMap<String, T>) -> HashMap<String, Value> {
    map.iter()
        .filter_map(|(id, t)| serde_json::to_value(t).ok().map(|v| (id.clone(), v)))
        .collect()
}

impl AppState {
    pub async fn project(&self, project_id: &str) -> Option<Project> {
        self.projects.read().await.get(project_id).cloned()
    }

    pub async fn instance(&self, instance_id: &str) -> Option<Instance> {
        self.instances.read().await.get(instance_id).cloned()
    }

    /// A project's instances, oldest first (the order its tabs show).
    pub async fn instances_of(&self, project_id: &str) -> Vec<Instance> {
        let mut list: Vec<Instance> = self
            .instances
            .read()
            .await
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

    /// Records an authenticated request (see `--idle-exit-secs`).
    pub fn touch(&self) {
        self.last_activity.store(
            monotonic_secs() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    pub fn idle_secs(&self) -> u64 {
        (monotonic_secs() as u64).saturating_sub(
            self.last_activity
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    /// Saved config for a project, or the defaults for its folder.
    pub async fn stored_config(&self, project: &Project) -> ProjectConfig {
        self.project_configs
            .read()
            .await
            .get(&project.id)
            .cloned()
            .unwrap_or_else(|| ProjectConfig::defaults_for(&project.path))
    }
}

fn state_path() -> PathBuf {
    if let Ok(path) = std::env::var("HANDOVER_STATE_PATH") {
        return PathBuf::from(path);
    }
    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(data_home).join("handover/state.json");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/handover/state.json");
    }
    PathBuf::from(".handover/state.json")
}

pub fn monotonic_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub async fn project_container_ids(state: &AppState, project_id: &str) -> Vec<String> {
    state
        .instances_of(project_id)
        .await
        .iter()
        .filter_map(|i| i.container_id().map(String::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn malformed_persisted_project_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(
            &path,
            r#"{"projects":{"good":{"id":"good","path":"/tmp/g"},"bad":{"id":3}}}"#,
        )
        .unwrap();
        let state = AppState::new();
        state.load_persisted_from_path(path).await.unwrap();
        let projects = state.projects.read().await;
        assert!(projects.contains_key("good"));
        assert!(!projects.contains_key("bad"));
    }

    #[tokio::test]
    async fn persisted_state_round_trips_projects_and_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let state = AppState::new();

        state.projects.write().await.insert(
            "project-1".into(),
            Project {
                id: "project-1".into(),
                path: "/tmp/project".into(),
                name: "project".into(),
                sandbox_mode: Default::default(),
                state: Default::default(),
            },
        );
        state.project_configs.write().await.insert(
            "project-1".into(),
            ProjectConfig::defaults_for("/tmp/project"),
        );
        state
            .project_last_active
            .write()
            .await
            .insert("project-1".into(), 42.0);
        state.save_persisted_to_path(path.clone()).await.unwrap();

        let restored = AppState::new();
        restored.load_persisted_from_path(path).await.unwrap();

        assert!(restored.projects.read().await.contains_key("project-1"));
        assert!(restored
            .project_configs
            .read()
            .await
            .contains_key("project-1"));
        assert_eq!(
            restored
                .project_last_active
                .read()
                .await
                .get("project-1")
                .copied(),
            Some(42.0)
        );
    }
}
