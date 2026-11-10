use std::sync::Arc;

use sysinfo::{MemoryRefreshKind, RefreshKind, System};
use tracing::{info, warn};

use crate::model::ProjectState;
use crate::sandbox::Sandboxes;
use crate::state::{
    project_container_ids, AppState, GOVERNOR_INTERVAL_SECS, RAM_EMERGENCY_THRESHOLD,
    RAM_SUSPEND_THRESHOLD,
};

pub fn spawn_governor(state: Arc<AppState>, sandboxes: Sandboxes) {
    tokio::spawn(async move {
        let mut sys = match System::new_with_specifics(
            RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
        ) {
            Ok(sys) => sys,
            Err(e) => {
                warn!("[governor] disabled: cannot read host memory: {e}");
                return;
            }
        };
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(GOVERNOR_INTERVAL_SECS)).await;
            if let Err(e) = tick(&state, &sandboxes, &mut sys).await {
                warn!("[governor] tick error: {e}");
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
        if project.state == ProjectState::Suspended {
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

async fn suspend_project(state: &AppState, sandboxes: &Sandboxes, project_id: &str) -> bool {
    let mut paused = 0;
    for instance in state.instances_of(project_id).await {
        match sandboxes.for_instance(&instance).suspend(&instance).await {
            Ok(true) => paused += 1,
            Ok(false) => {}
            Err(e) => warn!("[governor] failed to suspend instance {}: {e}", instance.id),
        }
    }
    if paused == 0 {
        return false;
    }
    if let Some(project) = state.projects.write().await.get_mut(project_id) {
        project.state = ProjectState::Suspended;
    }
    if let Err(e) = state.save_persisted().await {
        warn!("[governor] failed to persist suspended state: {e}");
    }
    info!("[governor] suspended project {project_id} ({paused} instance(s) paused)");
    true
}

async fn tick(state: &AppState, sandboxes: &Sandboxes, sys: &mut System) -> anyhow::Result<()> {
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
            suspend_project(state, sandboxes, &pid).await;
        }
    } else if percent >= RAM_SUSPEND_THRESHOLD {
        let targets = suspendable_projects(state).await;
        if let Some(pid) = targets.first() {
            info!(
                "[governor] RAM {:.0}% >= {:.0}% — suspending LRU project {pid}",
                percent, RAM_SUSPEND_THRESHOLD
            );
            suspend_project(state, sandboxes, pid).await;
        }
    }
    Ok(())
}
