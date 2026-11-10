//! The Governor (see GLOSSARY.md): when host RAM runs high, suspend the
//! least recently active unfocused Projects that have containers.

use std::sync::Arc;

use sysinfo::{MemoryRefreshKind, RefreshKind, System};
use tracing::{info, warn};

use crate::projects::Projects;
use crate::state::{GOVERNOR_INTERVAL_SECS, RAM_EMERGENCY_THRESHOLD, RAM_SUSPEND_THRESHOLD};

pub fn spawn_governor(projects: Arc<Projects>) {
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
            sys.refresh_memory();
            let percent = sys.used_memory() as f32 / sys.total_memory().max(1) as f32 * 100.0;
            tick(&projects, percent).await;
        }
    });
}

/// One decision at `ram_percent` host memory use.
async fn tick(projects: &Projects, ram_percent: f32) {
    if ram_percent >= RAM_EMERGENCY_THRESHOLD {
        let targets = projects.suspend_candidates().await;
        if !targets.is_empty() {
            info!(
                "[governor] RAM {ram_percent:.0}% >= {RAM_EMERGENCY_THRESHOLD:.0}% — suspending {} project(s)",
                targets.len()
            );
        }
        for id in targets {
            projects.suspend(&id).await;
        }
    } else if ram_percent >= RAM_SUSPEND_THRESHOLD {
        if let Some(id) = projects.suspend_candidates().await.first() {
            info!(
                "[governor] RAM {ram_percent:.0}% >= {RAM_SUSPEND_THRESHOLD:.0}% — suspending LRU project {id}"
            );
            projects.suspend(id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProjectState, SandboxMode};
    use crate::pty::Terminals;
    use crate::sandbox::fake::FakeSandbox;
    use crate::sandbox::{NativeSandbox, Sandboxes};

    async fn suspended(projects: &Projects, ids: &[String]) -> usize {
        let mut n = 0;
        for id in ids {
            if projects.project(id).await.unwrap().state == ProjectState::Suspended {
                n += 1;
            }
        }
        n
    }

    #[tokio::test]
    async fn suspends_one_project_when_high_and_all_when_critical() {
        let dir = tempfile::tempdir().unwrap();
        let projects = Projects::load(
            dir.path().join("state.json"),
            Sandboxes::new(
                Arc::new(NativeSandbox),
                Arc::new(FakeSandbox {
                    container: Some("c".into()),
                    ..Default::default()
                }),
            ),
            Terminals::new(),
        )
        .await;
        let mut ids = Vec::new();
        for n in 0..3 {
            let folder = dir.path().join(format!("p{n}"));
            std::fs::create_dir(&folder).unwrap();
            let p = projects
                .create(folder.to_str().unwrap().into(), SandboxMode::Docker)
                .await
                .unwrap();
            projects.open_instance(&p.project.id, None).await.unwrap();
            ids.push(p.project.id);
        }
        tick(&projects, 50.0).await;
        assert_eq!(suspended(&projects, &ids).await, 0);
        tick(&projects, RAM_SUSPEND_THRESHOLD).await;
        assert_eq!(suspended(&projects, &ids).await, 1);
        tick(&projects, RAM_EMERGENCY_THRESHOLD).await;
        assert_eq!(suspended(&projects, &ids).await, 3);
    }
}
