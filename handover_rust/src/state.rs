use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, RwLock};

pub const DEFAULT_MEM_LIMIT: &str = "2g";
pub const SANDBOX_MODES: &[&str] = &["docker", "native"];
pub const HANDOFF_METHODS: &[&str] = &["git", "summary"];

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
        "handoff_method": "git",
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
