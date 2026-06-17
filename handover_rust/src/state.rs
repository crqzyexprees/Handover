use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::fs;
use tokio::sync::{mpsc, oneshot, RwLock};

pub const DEFAULT_MEM_LIMIT: &str = "2g";
pub const SANDBOX_MODES: &[&str] = &["docker", "native"];
pub const HANDOFF_METHODS: &[&str] = &["git", "summary"];
pub const HANDOFF_TEMPLATES: &[&str] = &["generic", "nextjs", "rust-cli"];

pub const GOVERNOR_INTERVAL_SECS: u64 = 15;
pub const RAM_SUSPEND_THRESHOLD: f32 = 85.0;
pub const RAM_EMERGENCY_THRESHOLD: f32 = 95.0;

#[derive(Clone)]
pub struct PtySession {
    pub input_tx: mpsc::UnboundedSender<Vec<u8>>,
    pub close_tx: Arc<std::sync::Mutex<Option<oneshot::Sender<()>>>>,
}

pub struct AppState {
    pub projects: RwLock<HashMap<String, Value>>,
    pub project_configs: RwLock<HashMap<String, Value>>,
    pub instances: RwLock<HashMap<String, Value>>,
    pub pty_sessions: RwLock<HashMap<String, PtySession>>,
    pub focused_project_id: RwLock<Option<String>>,
    pub project_last_active: RwLock<HashMap<String, f64>>,
    pub app_config: RwLock<Value>,
}

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
        Arc::new(Self {
            projects: RwLock::new(HashMap::new()),
            project_configs: RwLock::new(HashMap::new()),
            instances: RwLock::new(HashMap::new()),
            pty_sessions: RwLock::new(HashMap::new()),
            focused_project_id: RwLock::new(None),
            project_last_active: RwLock::new(HashMap::new()),
            app_config: RwLock::new(json!({
                "clis_enabled": ["claude", "codex"],
                "auto_suspend_delay": 300,
                "container_mem_limit": "2g",
            })),
        })
    }

    pub async fn load_persisted(&self) -> anyhow::Result<()> {
        self.load_persisted_from_path(state_path()).await
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

        *self.projects.write().await = persisted.projects;
        *self.project_configs.write().await = persisted.project_configs;
        *self.project_last_active.write().await = persisted.project_last_active;
        if let Some(app_config) = persisted.app_config {
            *self.app_config.write().await = app_config;
        }
        Ok(())
    }

    pub async fn save_persisted(&self) -> anyhow::Result<()> {
        self.save_persisted_to_path(state_path()).await
    }

    async fn save_persisted_to_path(&self, path: PathBuf) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let snapshot = PersistedState {
            projects: self.projects.read().await.clone(),
            project_configs: self.project_configs.read().await.clone(),
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

pub fn basename_from_path(path: &str) -> String {
    let normalized = path.replace('\\', "/").trim_end_matches('/').to_string();
    if normalized.is_empty() {
        return "Unknown".into();
    }
    normalized
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Unknown")
        .to_string()
}

pub fn default_project_config(path: &str) -> Value {
    json!({
        "project_name": basename_from_path(path),
        "sandbox_mode": "docker",
        "mem_limit": DEFAULT_MEM_LIMIT,
        "handoff_method": "summary",
        "handoff_template": "generic",
        "custom_env_vars": {},
    })
}

pub fn monotonic_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub async fn project_container_ids(state: &AppState, project_id: &str) -> Vec<String> {
    let instances = state.instances.read().await;
    instances
        .values()
        .filter_map(|inst| {
            if inst.get("project_id").and_then(|v| v.as_str()) != Some(project_id) {
                return None;
            }
            inst.get("container_id")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn persisted_state_round_trips_projects_and_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let state = AppState::new();

        state.projects.write().await.insert(
            "project-1".into(),
            json!({
                "id": "project-1",
                "path": "/tmp/project",
                "name": "project",
                "state": "active"
            }),
        );
        state
            .project_configs
            .write()
            .await
            .insert("project-1".into(), default_project_config("/tmp/project"));
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

    #[test]
    fn basename_handles_empty_and_trailing_slashes() {
        assert_eq!(basename_from_path("/tmp/example/"), "example");
        assert_eq!(basename_from_path(""), "Unknown");
    }
}
