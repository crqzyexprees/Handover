//! `.handover/config.yml`: a Project's settings, kept in the project folder
//! so they can be hand-edited and committed.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::fs;

use crate::model::{basename_from_path, ProjectConfig, ProjectConfigPatch};

pub fn config_yml_path(project_path: &str) -> PathBuf {
    Path::new(project_path).join(".handover/config.yml")
}

/// Settings present in the project's config file (absent/invalid keys are `None`).
pub async fn read_local_config(project_path: &str) -> Result<Option<ProjectConfigPatch>> {
    let path = config_yml_path(project_path);
    if !fs::try_exists(&path).await.unwrap_or(false) {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(None);
    }
    let patch = serde_norway::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(patch))
}

pub async fn write_local_config(project_path: &str, config: &ProjectConfig) -> Result<()> {
    let path = config_yml_path(project_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut doc = config.clone();
    if doc.project_name.trim().is_empty() {
        doc.project_name = basename_from_path(project_path);
    }
    let yaml = serde_norway::to_string(&doc).context("failed to serialize config.yml")?;
    fs::write(&path, yaml)
        .await
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// The config in effect: the saved settings with the project file's values
/// on top. Saving from the UI writes both, so they agree; a hand edit to the
/// file wins over the stale saved copy.
pub async fn effective_config(project_path: &str, stored: ProjectConfig) -> Result<ProjectConfig> {
    let mut config = stored;
    if let Some(patch) = read_local_config(project_path).await? {
        patch.apply_to(&mut config);
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HandoffMethod, SandboxMode};

    #[tokio::test]
    async fn hand_edited_file_wins_and_partial_file_keeps_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let mut stored = ProjectConfig::defaults_for(path);
        stored.sandbox_mode = SandboxMode::Native;
        stored.mem_limit = "3g".into();
        write_local_config(path, &stored).await.unwrap();
        std::fs::write(
            config_yml_path(path),
            "handoff_method: git\nsandbox_mode: vm\n",
        )
        .unwrap();

        let cfg = effective_config(path, stored).await.unwrap();
        assert_eq!(cfg.handoff_method, HandoffMethod::Git);
        assert_eq!(cfg.sandbox_mode, SandboxMode::Native); // invalid "vm" ignored
        assert_eq!(cfg.mem_limit, "3g"); // not in file: kept
    }

    #[tokio::test]
    async fn written_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let mut cfg = ProjectConfig::defaults_for(path);
        cfg.custom_env_vars.insert("A".into(), "1".into());
        write_local_config(path, &cfg).await.unwrap();
        let mut reread = ProjectConfig::defaults_for("/elsewhere");
        read_local_config(path)
            .await
            .unwrap()
            .unwrap()
            .apply_to(&mut reread);
        assert_eq!(reread, cfg);
    }
}
