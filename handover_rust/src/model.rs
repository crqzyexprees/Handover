//! Typed domain model: Project, Instance and ProjectConfig (see GLOSSARY.md).
//!
//! Serialized field names match the JSON the frontend and `state.json` have
//! always used, so the wire format is unchanged. Enums reject unknown values
//! at the API edge; persisted state and hand-edited config files are read
//! leniently (an unknown value falls back to the default) so one bad entry
//! can't make the whole state unloadable.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub const DEFAULT_MEM_LIMIT: &str = "2g";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SandboxMode {
    #[default]
    Docker,
    Native,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HandoffMethod {
    Git,
    #[default]
    Summary,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HandoffTemplate {
    #[default]
    Generic,
    Nextjs,
    RustCli,
}

impl HandoffTemplate {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Nextjs => "nextjs",
            Self::RustCli => "rust-cli",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProjectState {
    #[default]
    Active,
    Suspended,
}

/// Folder name used as a Project's display name.
pub fn basename_from_path(path: &str) -> String {
    let normalized = path.replace('\\', "/").trim_end_matches('/').to_string();
    normalized
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Unknown")
        .to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Project {
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "lenient")]
    pub sandbox_mode: SandboxMode,
    #[serde(default, deserialize_with = "lenient")]
    pub state: ProjectState,
}

/// One terminal of a Project. Lives in memory only (never persisted).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Instance {
    pub id: String,
    pub project_id: String,
    pub sandbox_mode: SandboxMode,
    /// Docker container; `null` for native instances.
    pub container_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_limit: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub custom_env_vars: BTreeMap<String, String>,
    /// Unix seconds when it was opened; orders a project's terminals.
    #[serde(default)]
    pub opened_at: f64,
}

impl Instance {
    pub fn container_id(&self) -> Option<&str> {
        self.container_id.as_deref().filter(|s| !s.is_empty())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProjectConfig {
    #[serde(default)]
    pub project_name: String,
    #[serde(default, deserialize_with = "lenient")]
    pub sandbox_mode: SandboxMode,
    #[serde(default = "default_mem_limit")]
    pub mem_limit: String,
    #[serde(default, deserialize_with = "lenient")]
    pub handoff_method: HandoffMethod,
    #[serde(default, deserialize_with = "lenient")]
    pub handoff_template: HandoffTemplate,
    #[serde(default)]
    pub custom_env_vars: BTreeMap<String, String>,
}

fn default_mem_limit() -> String {
    DEFAULT_MEM_LIMIT.into()
}

impl ProjectConfig {
    pub fn defaults_for(path: &str) -> Self {
        Self {
            project_name: basename_from_path(path),
            sandbox_mode: SandboxMode::default(),
            mem_limit: default_mem_limit(),
            handoff_method: HandoffMethod::default(),
            handoff_template: HandoffTemplate::default(),
            custom_env_vars: BTreeMap::new(),
        }
    }
}

/// The settings present in `.handover/config.yml`. Missing or invalid keys
/// are `None` and leave the underlying config unchanged.
#[derive(Deserialize, Debug, Default, PartialEq)]
pub struct ProjectConfigPatch {
    #[serde(default, deserialize_with = "lenient_opt")]
    pub project_name: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub sandbox_mode: Option<SandboxMode>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub mem_limit: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub handoff_method: Option<HandoffMethod>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub handoff_template: Option<HandoffTemplate>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub custom_env_vars: Option<BTreeMap<String, String>>,
}

impl ProjectConfigPatch {
    pub fn apply_to(self, config: &mut ProjectConfig) {
        if let Some(v) = self.project_name.filter(|s| !s.trim().is_empty()) {
            config.project_name = v;
        }
        if let Some(v) = self.sandbox_mode {
            config.sandbox_mode = v;
        }
        if let Some(v) = self.mem_limit.filter(|s| !s.trim().is_empty()) {
            config.mem_limit = v;
        }
        if let Some(v) = self.handoff_method {
            config.handoff_method = v;
        }
        if let Some(v) = self.handoff_template {
            config.handoff_template = v;
        }
        if let Some(v) = self.custom_env_vars {
            config.custom_env_vars = v;
        }
    }
}

/// Deserialize, falling back to the default on an invalid value.
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// Deserialize, treating an invalid value as absent.
fn lenient_opt<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enums_use_the_existing_wire_names() {
        assert_eq!(json!(SandboxMode::Native), json!("native"));
        assert_eq!(json!(HandoffTemplate::RustCli), json!("rust-cli"));
        assert_eq!(json!(ProjectState::Suspended), json!("suspended"));
    }

    #[test]
    fn strict_enums_reject_unknown_values_at_the_api_edge() {
        assert!(serde_json::from_value::<SandboxMode>(json!("vm")).is_err());
    }

    #[test]
    fn persisted_config_loads_leniently() {
        let cfg: ProjectConfig = serde_json::from_value(json!({
            "project_name": "x", "sandbox_mode": "vm", "handoff_method": "git"
        }))
        .unwrap();
        assert_eq!(cfg.sandbox_mode, SandboxMode::Docker);
        assert_eq!(cfg.handoff_method, HandoffMethod::Git);
        assert_eq!(cfg.mem_limit, "2g");
    }

    #[test]
    fn patch_applies_only_present_valid_keys() {
        let patch: ProjectConfigPatch =
            serde_json::from_value(json!({ "mem_limit": "4g", "sandbox_mode": "bogus" })).unwrap();
        let mut cfg = ProjectConfig::defaults_for("/tmp/demo");
        cfg.sandbox_mode = SandboxMode::Native;
        patch.apply_to(&mut cfg);
        assert_eq!(cfg.mem_limit, "4g");
        assert_eq!(cfg.sandbox_mode, SandboxMode::Native);
        assert_eq!(cfg.project_name, "demo");
    }

    #[test]
    fn instance_json_shape_is_unchanged() {
        let native = Instance {
            id: "i".into(),
            project_id: "p".into(),
            sandbox_mode: SandboxMode::Native,
            container_id: None,
            mem_limit: None,
            custom_env_vars: BTreeMap::new(),
            opened_at: 1.5,
        };
        assert_eq!(
            json!(native),
            json!({ "id": "i", "project_id": "p", "sandbox_mode": "native", "container_id": null, "opened_at": 1.5 })
        );
    }

    #[test]
    fn basename_handles_empty_and_trailing_slashes() {
        assert_eq!(basename_from_path("/tmp/example/"), "example");
        assert_eq!(basename_from_path(""), "Unknown");
    }
}
