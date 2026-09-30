use std::collections::HashSet;
use std::time::Duration;

use serde_json::{json, Value};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};

/// Memory + CPU for a shell process and its descendants (native / docker-exec PTY children).
pub fn process_tree_stats(root_pid: u32) -> Value {
    process_tree_stats_batch(&[root_pid])
        .into_iter()
        .next()
        .map(|(_, stats)| stats)
        .unwrap_or_else(zeros)
}

/// Sample many process trees with a single CPU refresh cycle.
pub fn process_tree_stats_batch(root_pids: &[u32]) -> Vec<(u32, Value)> {
    if root_pids.is_empty() {
        return Vec::new();
    }

    let mut sys = System::new_with_specifics(
        RefreshKind::nothing().with_processes(
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_cmd(UpdateKind::OnlyIfNotSet),
        ),
    );

    // First sample seeds CPU counters; second sample yields usable percentages.
    sys.refresh_processes(ProcessesToUpdate::All, true);
    std::thread::sleep(Duration::from_millis(200));
    sys.refresh_processes(ProcessesToUpdate::All, true);

    root_pids
        .iter()
        .copied()
        .map(|root_pid| (root_pid, stats_for_root(&sys, root_pid)))
        .collect()
}

pub fn zeros() -> Value {
    json!({
        "mem_used_mb": 0.0,
        "mem_limit_mb": 0.0,
        "cpu_percent": 0.0,
    })
}

fn stats_for_root(sys: &System, root_pid: u32) -> Value {
    let root = Pid::from_u32(root_pid);
    if sys.process(root).is_none() {
        return zeros();
    }

    let tree = collect_tree(sys, root);
    let mut mem_bytes: u64 = 0;
    let mut cpu_percent: f64 = 0.0;
    for pid in &tree {
        if let Some(proc) = sys.process(*pid) {
            mem_bytes = mem_bytes.saturating_add(proc.memory());
            cpu_percent += f64::from(proc.cpu_usage());
        }
    }

    let mem_used_mb = ((mem_bytes as f64 / (1024.0 * 1024.0)) * 10.0).round() / 10.0;
    let cpu_percent = (cpu_percent * 10.0).round() / 10.0;

    json!({
        "mem_used_mb": mem_used_mb,
        "mem_limit_mb": 0.0,
        "cpu_percent": cpu_percent,
    })
}

fn collect_tree(sys: &System, root: Pid) -> Vec<Pid> {
    let mut ordered = vec![root];
    let mut seen = HashSet::from([root]);
    let mut i = 0;
    while i < ordered.len() {
        let parent = ordered[i];
        for (pid, proc) in sys.processes() {
            if proc.parent() == Some(parent) && seen.insert(*pid) {
                ordered.push(*pid);
            }
        }
        i += 1;
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_reports_nonzero_memory() {
        let pid = std::process::id();
        let stats = process_tree_stats(pid);
        let mem = stats
            .get("mem_used_mb")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        assert!(mem > 0.0, "expected memory for pid {pid}, got {stats}");
    }
}
