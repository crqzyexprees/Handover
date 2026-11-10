//! `state.json`: what the backend keeps between runs (Projects, their
//! config and recency, app settings). Instances are never saved.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::fs;
use tracing::warn;

use crate::model::{Project, ProjectConfig};

pub const GOVERNOR_INTERVAL_SECS: u64 = 15;
pub const RAM_SUSPEND_THRESHOLD: f32 = 85.0;
pub const RAM_EMERGENCY_THRESHOLD: f32 = 95.0;

/// The saved state, typed.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub projects: HashMap<String, Project>,
    pub configs: HashMap<String, ProjectConfig>,
    pub last_active: HashMap<String, f64>,
    pub app_config: Value,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            projects: HashMap::new(),
            configs: HashMap::new(),
            last_active: HashMap::new(),
            app_config: default_app_config(),
        }
    }
}

fn default_app_config() -> Value {
    json!({
        "clis_enabled": ["claude", "codex", "cursor"],
        "auto_suspend_delay": 300,
        "container_mem_limit": "2g",
    })
}

/// On disk, entries stay untyped so one malformed project can be skipped
/// instead of making the whole file unreadable.
#[derive(Debug, Default, Deserialize, Serialize)]
struct OnDisk {
    #[serde(default)]
    projects: HashMap<String, Value>,
    #[serde(default)]
    project_configs: HashMap<String, Value>,
    #[serde(default)]
    project_last_active: HashMap<String, f64>,
    #[serde(default)]
    app_config: Option<Value>,
}

/// Where `state.json` lives by default.
pub fn default_path() -> PathBuf {
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

/// Reads saved state; a missing file is an empty state.
pub async fn load(path: &Path) -> anyhow::Result<Snapshot> {
    if !fs::try_exists(path).await.unwrap_or(false) {
        return Ok(Snapshot::default());
    }
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    let disk: OnDisk = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Snapshot {
        projects: typed_entries(disk.projects, "project"),
        configs: typed_entries(disk.project_configs, "project config"),
        last_active: disk.project_last_active,
        app_config: disk.app_config.unwrap_or_else(default_app_config),
    })
}

pub async fn save(path: &Path, snapshot: &Snapshot) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let disk = OnDisk {
        projects: untyped_entries(&snapshot.projects),
        project_configs: untyped_entries(&snapshot.configs),
        project_last_active: snapshot.last_active.clone(),
        app_config: Some(snapshot.app_config.clone()),
    };
    // Write then rename, so a crash mid-write never truncates the file.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(&disk)?)
        .await
        .with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .await
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

pub fn monotonic_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn malformed_project_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(
            &path,
            r#"{"projects":{"good":{"id":"good","path":"/tmp/g"},"bad":{"id":3}}}"#,
        )
        .unwrap();
        let snapshot = load(&path).await.unwrap();
        assert!(snapshot.projects.contains_key("good"));
        assert!(!snapshot.projects.contains_key("bad"));
    }

    #[tokio::test]
    async fn snapshot_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut snapshot = Snapshot::default();
        snapshot.projects.insert(
            "project-1".into(),
            Project {
                id: "project-1".into(),
                path: "/tmp/project".into(),
                name: "project".into(),
                sandbox_mode: Default::default(),
                state: Default::default(),
            },
        );
        snapshot.configs.insert(
            "project-1".into(),
            ProjectConfig::defaults_for("/tmp/project"),
        );
        snapshot.last_active.insert("project-1".into(), 42.0);
        save(&path, &snapshot).await.unwrap();
        assert_eq!(load(&path).await.unwrap(), snapshot);
    }

    #[tokio::test]
    async fn missing_file_is_empty_state() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot = load(&dir.path().join("none.json")).await.unwrap();
        assert!(snapshot.projects.is_empty());
        assert_eq!(snapshot.app_config["container_mem_limit"], "2g");
    }
}
