use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio::fs;

use crate::state::{
    basename_from_path, DEFAULT_MEM_LIMIT, HANDOFF_METHODS, HANDOFF_TEMPLATES, SANDBOX_MODES,
};

pub fn config_yml_path(project_path: &str) -> PathBuf {
    Path::new(project_path).join(".handover/config.yml")
}

/// Merge `overlay` onto `base` (top-level keys; overlay wins).
pub fn merge_config_values(base: Value, overlay: Value) -> Value {
    let Some(base_obj) = base.as_object() else {
        return overlay;
    };
    let Some(overlay_obj) = overlay.as_object() else {
        return base;
    };
    let mut merged = base_obj.clone();
    for (key, value) in overlay_obj {
        merged.insert(key.clone(), value.clone());
    }
    Value::Object(merged)
}

pub async fn read_local_config(project_path: &str) -> Result<Option<Value>> {
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
    let parsed: Value = serde_norway::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(sanitize_local_config(parsed)))
}

pub async fn write_local_config(project_path: &str, config: &Value) -> Result<()> {
    let path = config_yml_path(project_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let project_name = config
        .get("project_name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| basename_from_path(project_path));

    let sandbox_mode = config
        .get("sandbox_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("docker");
    let mem_limit = config
        .get("mem_limit")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_MEM_LIMIT);
    let handoff_method = config
        .get("handoff_method")
        .and_then(|v| v.as_str())
        .unwrap_or("summary");
    let handoff_template = config
        .get("handoff_template")
        .and_then(|v| v.as_str())
        .unwrap_or("generic");

    let custom_env_vars: HashMap<String, String> = config
        .get("custom_env_vars")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let doc = json!({
        "project_name": project_name,
        "sandbox_mode": sandbox_mode,
        "mem_limit": mem_limit,
        "handoff_method": handoff_method,
        "handoff_template": handoff_template,
        "custom_env_vars": custom_env_vars,
    });

    let yaml = serde_norway::to_string(&doc).context("failed to serialize config.yml")?;
    fs::write(&path, yaml)
        .await
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Keeps only the keys actually present in the file (so a partial file
/// doesn't reset other settings to defaults) and drops values outside the
/// allowed sets.
fn sanitize_local_config(raw: Value) -> Value {
    let mut overlay = json!({});
    let Some(obj) = raw.as_object() else {
        return overlay;
    };
    for (key, value) in obj {
        let allowed: Option<&[&str]> = match key.as_str() {
            "sandbox_mode" => Some(SANDBOX_MODES),
            "handoff_method" => Some(HANDOFF_METHODS),
            "handoff_template" => Some(HANDOFF_TEMPLATES),
            _ => None,
        };
        if let Some(allowed) = allowed {
            if !value.as_str().is_some_and(|v| allowed.contains(&v)) {
                continue;
            }
        }
        if value.is_null() {
            continue;
        }
        overlay[key] = value.clone();
    }
    overlay
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_local_config_keeps_only_present_valid_keys() {
        let raw = json!({ "mem_limit": "4g", "sandbox_mode": "bogus", "handoff_method": "git" });
        let out = sanitize_local_config(raw);
        assert_eq!(out, json!({ "mem_limit": "4g", "handoff_method": "git" }));
    }

    #[test]
    fn merge_config_values_overlay_wins() {
        let base = json!({ "handoff_method": "git", "mem_limit": "2g" });
        let overlay = json!({ "handoff_method": "summary" });
        let merged = merge_config_values(base, overlay);
        assert_eq!(merged["handoff_method"], "summary");
        assert_eq!(merged["mem_limit"], "2g");
    }
}
