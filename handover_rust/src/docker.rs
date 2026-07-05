use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, RemoveContainerOptions,
    StartContainerOptions, Stats, StatsOptions,
};
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::{ContainerStateStatusEnum, HostConfig};
use bollard::Docker;
use futures::StreamExt;
use serde_json::json;
use tempfile::TempDir;

use crate::platform::{current_gid, current_uid, current_username, home_dir};

pub const INSTANCE_LABEL: &str = "handover.instance_id";
pub const BASE_IMAGE: &str = "handover-base:latest";
pub const SANDBOX_HOME: &str = "/home/sandbox";
pub const CONFIG_STAGING_DIR: &str = "/mnt/host-config";
pub const NANO_CPUS: i64 = 1_000_000_000;

pub const NSS_DIR_PREFIX: &str = "handover-nss-";

const FORWARDED_API_KEYS: &[&str] = &["ANTHROPIC_API_KEY", "OPENAI_API_KEY"];

const CONFIG_MOUNTS: &[(&str, &str)] = &[
    ("~/.claude", ".claude"),
    ("~/.codex", ".codex"),
    ("~/.cursor", ".cursor"),
    ("~/.config/cursor", ".config/cursor"),
    ("~/.antigravity", ".antigravity"),
    ("~/.gemini", ".gemini"),
    ("~/.config/claude-code", ".config/claude-code"),
    ("~/.config/claude", ".config/claude"),
];

const CONFIG_FILE_MOUNTS: &[(&str, &str)] = &[("~/.claude.json", ".claude.json")];

fn copy_config_cmd() -> String {
    format!(
        "if [ -d {CONFIG_STAGING_DIR} ]; then \
         cp -a {CONFIG_STAGING_DIR}/. $HOME/ 2>/dev/null; \
         chmod -R u+rwX $HOME 2>/dev/null; fi; true"
    )
}

pub struct DockerRuntime {
    docker: Docker,
    _nss_dir: TempDir,
}

impl DockerRuntime {
    pub fn new() -> Result<Self> {
        let docker = Docker::connect_with_local_defaults().context("docker connect failed")?;
        let nss_dir = build_nss_files()?;
        Ok(Self {
            docker,
            _nss_dir: nss_dir,
        })
    }

    pub async fn ping(&self) -> Result<()> {
        self.docker.ping().await.context("docker ping failed")?;
        Ok(())
    }

    pub async fn start_container(
        &self,
        project_path: &str,
        instance_id: &str,
        mem_limit: &str,
        custom_env_vars: Option<&HashMap<String, String>>,
    ) -> Result<String> {
        let uid = current_uid();
        let gid = current_gid();
        let mem_bytes = parse_mem_bytes(mem_limit);
        let nss_path = self._nss_dir.path();

        let mut env = vec![format!("HOME={SANDBOX_HOME}")];
        for key in FORWARDED_API_KEYS {
            if let Ok(val) = std::env::var(key) {
                env.push(format!("{key}={val}"));
            }
        }
        if let Some(custom) = custom_env_vars {
            for (k, v) in custom {
                env.push(format!("{k}={v}"));
            }
        }

        let mut binds = vec![
            crate::platform::docker_bind_rw(project_path, "/workspace"),
            crate::platform::docker_bind_ro(
                &nss_path.join("passwd").to_string_lossy(),
                "/etc/passwd",
            ),
            crate::platform::docker_bind_ro(
                &nss_path.join("group").to_string_lossy(),
                "/etc/group",
            ),
        ];

        for (host, rel) in CONFIG_MOUNTS {
            let expanded = expand_home(host);
            if expanded.is_dir() {
                binds.push(crate::platform::docker_bind_ro(
                    &expanded.to_string_lossy(),
                    &format!("{CONFIG_STAGING_DIR}/{rel}"),
                ));
            }
        }
        for (host, rel) in CONFIG_FILE_MOUNTS {
            let expanded = expand_home(host);
            if expanded.is_file() {
                binds.push(crate::platform::docker_bind_ro(
                    &expanded.to_string_lossy(),
                    &format!("{CONFIG_STAGING_DIR}/{rel}"),
                ));
            }
        }

        let mut labels = HashMap::new();
        labels.insert(INSTANCE_LABEL.to_string(), instance_id.to_string());

        let host_config = HostConfig {
            binds: Some(binds),
            memory: Some(mem_bytes),
            memory_swap: Some(mem_bytes),
            nano_cpus: Some(NANO_CPUS),
            extra_hosts: Some(vec!["host.docker.internal:host-gateway".into()]),
            ..Default::default()
        };

        let config = Config {
            image: Some(BASE_IMAGE.to_string()),
            cmd: Some(vec!["/bin/bash".into()]),
            tty: Some(true),
            open_stdin: Some(true),
            user: Some(format!("{uid}:{gid}")),
            working_dir: Some("/workspace".into()),
            env: Some(env),
            labels: Some(labels),
            host_config: Some(host_config),
            ..Default::default()
        };

        let name = format!("handover-{instance_id}");
        let create = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name,
                    platform: None,
                }),
                config,
            )
            .await?;

        let id = create.id;
        self.docker
            .start_container(&id, None::<StartContainerOptions<String>>)
            .await?;

        let exec = self
            .docker
            .create_exec(
                &id,
                CreateExecOptions {
                    cmd: Some(vec!["/bin/bash".into(), "-c".into(), copy_config_cmd()]),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    user: Some(format!("{uid}:{gid}")),
                    env: Some(vec![format!("HOME={SANDBOX_HOME}")]),
                    ..Default::default()
                },
            )
            .await?;

        if let StartExecResults::Attached { mut output, .. } =
            self.docker.start_exec(&exec.id, None).await?
        {
            while output.next().await.is_some() {}
        }

        Ok(id)
    }

    pub async fn stop_container(&self, container_id: &str) -> Result<()> {
        self.docker
            .remove_container(
                container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .with_context(|| format!("failed to remove container {container_id}"))?;
        Ok(())
    }

    pub async fn pause_container(&self, container_id: &str) -> Result<()> {
        self.docker
            .pause_container(container_id)
            .await
            .with_context(|| format!("failed to pause container {container_id}"))?;
        Ok(())
    }

    pub async fn resume_container(&self, container_id: &str) -> Result<()> {
        self.docker
            .unpause_container(container_id)
            .await
            .with_context(|| format!("failed to resume container {container_id}"))?;
        Ok(())
    }

    pub async fn get_container_stats(&self, container_id: &str) -> serde_json::Value {
        let zeros = json!({
            "mem_used_mb": 0.0_f64,
            "mem_limit_mb": 0.0_f64,
            "cpu_percent": 0.0_f64,
        });

        let inspect = match self.docker.inspect_container(container_id, None).await {
            Ok(i) => i,
            Err(_) => return zeros,
        };
        if inspect.state.and_then(|s| s.status) != Some(ContainerStateStatusEnum::RUNNING) {
            return zeros;
        }

        let mut stream = self.docker.stats(
            container_id,
            Some(StatsOptions {
                stream: false,
                one_shot: true,
            }),
        );
        let stats = match stream.next().await {
            Some(Ok(s)) => s,
            _ => return zeros,
        };

        json!({
            "mem_used_mb": mem_used_mb(&stats),
            "mem_limit_mb": mem_limit_mb(&stats),
            "cpu_percent": cpu_percent(&stats),
        })
    }

    pub async fn cleanup_orphans(&self) -> usize {
        let mut filters: HashMap<String, Vec<String>> = HashMap::new();
        filters.insert("label".to_string(), vec![INSTANCE_LABEL.to_string()]);
        let list = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters,
                ..Default::default()
            }))
            .await
            .unwrap_or_default();
        let count = list.len();
        for c in list {
            if let Some(id) = c.id {
                let _ = self
                    .docker
                    .remove_container(
                        &id,
                        Some(RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }),
                    )
                    .await;
            }
        }
        count
    }

    pub async fn cleanup_nss_temp_dirs(&self) -> usize {
        let current = self._nss_dir.path().to_path_buf();
        let temp_dir = std::env::temp_dir();
        let entries = match std::fs::read_dir(temp_dir) {
            Ok(entries) => entries,
            Err(_) => return 0,
        };

        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.starts_with(NSS_DIR_PREFIX) || path == current || !path.is_dir() {
                continue;
            }
            if std::fs::remove_dir_all(path).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn parse_mem_bytes(mem_limit: &str) -> i64 {
    let s = mem_limit.trim().to_lowercase();
    if let Some(num) = s.strip_suffix('g') {
        return (num.parse::<f64>().unwrap_or(2.0) * 1024.0 * 1024.0 * 1024.0) as i64;
    }
    if let Some(num) = s.strip_suffix('m') {
        return (num.parse::<f64>().unwrap_or(512.0) * 1024.0 * 1024.0) as i64;
    }
    2 * 1024 * 1024 * 1024
}

fn build_nss_files() -> Result<TempDir> {
    let dir = tempfile::Builder::new()
        .prefix(NSS_DIR_PREFIX)
        .tempdir()
        .context("tempdir failed")?;
    let uid = current_uid();
    let gid = current_gid();
    let user = current_username();

    let passwd = format!(
        "root:x:0:0:root:/root:/bin/bash\n{user}:x:{uid}:{gid}:{user}:{SANDBOX_HOME}:/bin/bash\n"
    );
    let group = if gid != 0 {
        format!("root:x:0:\n{user}:x:{gid}:\n")
    } else {
        "root:x:0:\n".to_string()
    };

    std::fs::write(dir.path().join("passwd"), passwd)?;
    std::fs::write(dir.path().join("group"), group)?;
    Ok(dir)
}

fn cpu_percent(stats: &Stats) -> f64 {
    let cpu = &stats.cpu_stats;
    let precpu = &stats.precpu_stats;
    let usage = &cpu.cpu_usage;
    let pre_usage = &precpu.cpu_usage;
    let cpu_delta = usage.total_usage.saturating_sub(pre_usage.total_usage);
    let system_delta = cpu
        .system_cpu_usage
        .unwrap_or(0)
        .saturating_sub(precpu.system_cpu_usage.unwrap_or(0));
    if cpu_delta <= 0 || system_delta <= 0 {
        return 0.0;
    }
    let online_cpus = cpu
        .online_cpus
        .or_else(|| usage.percpu_usage.as_ref().map(|p| p.len() as u64))
        .unwrap_or(1) as f64;
    ((cpu_delta as f64 / system_delta as f64) * online_cpus * 100.0 * 10.0).round() / 10.0
}

fn mem_used_mb(stats: &Stats) -> f64 {
    let mem = &stats.memory_stats;
    let usage = mem.usage.unwrap_or(0) as i64;
    let cache = match mem.stats {
        Some(bollard::container::MemoryStatsStats::V1(s)) => s.inactive_file,
        Some(bollard::container::MemoryStatsStats::V2(s)) => s.inactive_file,
        None => 0,
    } as i64;
    ((usage - cache).max(0) as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0
}

fn mem_limit_mb(stats: &Stats) -> f64 {
    let limit = stats.memory_stats.limit.unwrap_or(0) as f64;
    (limit / (1024.0 * 1024.0) * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mem_bytes_supports_gigabytes_and_megabytes() {
        assert_eq!(parse_mem_bytes("2g"), 2 * 1024 * 1024 * 1024);
        assert_eq!(parse_mem_bytes("512m"), 512 * 1024 * 1024);
        assert_eq!(
            parse_mem_bytes("1.5g"),
            (1.5 * 1024.0 * 1024.0 * 1024.0) as i64
        );
    }

    #[test]
    fn parse_mem_bytes_falls_back_for_invalid_values() {
        assert_eq!(parse_mem_bytes("bad"), 2 * 1024 * 1024 * 1024);
        assert_eq!(parse_mem_bytes(""), 2 * 1024 * 1024 * 1024);
    }
}
