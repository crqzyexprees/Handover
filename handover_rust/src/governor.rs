use std::sync::Arc;

use sysinfo::{MemoryRefreshKind, RefreshKind, System};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::docker::DockerRuntime;
use crate::state::{
    project_container_ids, AppState, GOVERNOR_INTERVAL_SECS, RAM_EMERGENCY_THRESHOLD,
    RAM_SUSPEND_THRESHOLD,
};

pub fn spawn_governor(state: Arc<AppState>, docker_slot: Arc<RwLock<Option<Arc<DockerRuntime>>>>) {
    tokio::spawn(async move {
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
        );
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(GOVERNOR_INTERVAL_SECS)).await;
            let docker = docker_slot.read().await.clone();
            if let Some(docker) = docker.as_ref() {
                if let Err(e) = tick(&state, docker, &mut sys).await {
                    warn!("[governor] tick error: {e}");
                }
            }
        }
    });
}

async fn suspendable_projects(state: &AppState) -> Vec<String> {
    let focused = state.focused_project_id.read().await.clone();
    let projects = state.projects.read().await;
    let mut candidates: Vec<String> = Vec::new();
    for (pid, project) in projects.iter() {
        if Some(pid.clone()) == focused {
            continue;
        }
        if project.get("state").and_then(|v| v.as_str()) == Some("suspended") {
            continue;
        }
        let containers = project_container_ids(state, pid).await;
        if !containers.is_empty() {
            candidates.push(pid.clone());
        }
    }
    let last_active = state.project_last_active.read().await;
    candidates.sort_by(|a, b| {
        let ta = last_active.get(a).copied().unwrap_or(0.0);
        let tb = last_active.get(b).copied().unwrap_or(0.0);
        ta.partial_cmp(&tb).unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates
}

async fn suspend_project(state: &AppState, docker: &DockerRuntime, project_id: &str) -> bool {
    let container_ids = project_container_ids(state, project_id).await;
    if container_ids.is_empty() {
        return false;
    }
    for id in &container_ids {
        if let Err(e) = docker.pause_container(id).await {
            warn!("[governor] failed to pause container {id}: {e}");
        }
    }
    let mut projects = state.projects.write().await;
    if let Some(project) = projects.get_mut(project_id) {
        if let Some(obj) = project.as_object_mut() {
            obj.insert("state".into(), serde_json::json!("suspended"));
        }
    }
    drop(projects);
    if let Err(e) = state.save_persisted().await {
        warn!("[governor] failed to persist suspended state: {e}");
    }
    info!(
        "[governor] suspended project {project_id} ({} container(s) paused)",
        container_ids.len()
    );
    true
}

async fn tick(state: &AppState, docker: &DockerRuntime, sys: &mut System) -> anyhow::Result<()> {
    sys.refresh_memory();
    let percent = sys.used_memory() as f32 / sys.total_memory().max(1) as f32 * 100.0;

    if percent >= RAM_EMERGENCY_THRESHOLD {
        let targets = suspendable_projects(state).await;
        if !targets.is_empty() {
            info!(
                "[governor] RAM {:.0}% >= {:.0}% — emergency pausing {} project(s)",
                percent,
                RAM_EMERGENCY_THRESHOLD,
                targets.len()
            );
        }
        for pid in targets {
            suspend_project(state, docker, &pid).await;
        }
    } else if percent >= RAM_SUSPEND_THRESHOLD {
        let targets = suspendable_projects(state).await;
        if let Some(pid) = targets.first() {
            info!(
                "[governor] RAM {:.0}% >= {:.0}% — suspending LRU project {pid}",
                percent, RAM_SUSPEND_THRESHOLD
            );
            suspend_project(state, docker, pid).await;
        }
    }
    Ok(())
}
