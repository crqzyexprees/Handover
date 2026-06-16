use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use chrono::Utc;
use git2::{Repository, Signature};
use tokio::fs;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct HandoffFileSnapshot {
    modified: Option<SystemTime>,
    len: u64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum GitHandoffOutcome {
    Committed,
    NoChanges,
}

pub async fn git_handoff(project_path: &str, summary_text: &str) -> Result<GitHandoffOutcome> {
    let path = project_path.to_string();
    let message = summary_text.to_string();
    let outcome = tokio::task::spawn_blocking(move || -> Result<GitHandoffOutcome> {
        let repo = Repository::open(&path).context("not a git repository")?;
        let mut index = repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.update_all(["*"].iter(), None)?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        if let Some(parent) = parent.as_ref() {
            if parent.tree_id() == tree_id {
                return Ok(GitHandoffOutcome::NoChanges);
            }
        }
        let sig = repo
            .signature()
            .or_else(|_| Signature::now("Handover", "handover@local"))?;
        match parent {
            Some(parent) => repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&parent])?,
            None => repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[])?,
        };
        Ok(GitHandoffOutcome::Committed)
    })
    .await??;
    Ok(outcome)
}

fn next_handoff_path(handoffs_dir: &Path) -> PathBuf {
    let mut highest = 0u32;
    if let Ok(mut entries) = std::fs::read_dir(handoffs_dir) {
        while let Some(Ok(entry)) = entries.next() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(num) = name
                .strip_prefix("handoff_")
                .and_then(|s| s.strip_suffix(".md"))
            {
                if let Ok(n) = num.parse::<u32>() {
                    highest = highest.max(n);
                }
            }
        }
    }
    handoffs_dir.join(format!("handoff_{:03}.md", highest + 1))
}

fn handoffs_dir(project_path: &str) -> PathBuf {
    Path::new(project_path).join(".handover/handoffs")
}

pub fn latest_handoff_path(project_path: &str) -> PathBuf {
    handoffs_dir(project_path).join("latest.md")
}

pub fn latest_handoff_snapshot(project_path: &str) -> Option<HandoffFileSnapshot> {
    let metadata = std::fs::metadata(latest_handoff_path(project_path)).ok()?;
    Some(HandoffFileSnapshot {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

pub const HANDOFF_FILE_WAIT_TIMEOUT: Duration = Duration::from_secs(60);
const HANDOFF_POLL_INTERVAL: Duration = Duration::from_secs(1);
const HANDOFF_STABLE_POLLS: u32 = 2;

pub async fn wait_for_latest_handoff(
    project_path: &str,
    previous: Option<HandoffFileSnapshot>,
    timeout: Duration,
) -> Result<bool> {
    let latest = latest_handoff_path(project_path);
    let deadline = tokio::time::Instant::now() + timeout;
    let mut stable_polls = 0u32;
    let mut last_seen_len: Option<u64> = None;

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Ok(false);
        }

        if let Ok(metadata) = fs::metadata(&latest).await {
            let current = HandoffFileSnapshot {
                modified: metadata.modified().ok(),
                len: metadata.len(),
            };
            let is_new_or_updated = match previous {
                None => true,
                Some(previous) => {
                    current.len != previous.len
                        || current
                            .modified
                            .zip(previous.modified)
                            .is_some_and(|(current, previous)| current > previous)
                }
            };

            if is_new_or_updated && current.len > 0 {
                match last_seen_len {
                    Some(len) if len == current.len => {
                        stable_polls += 1;
                        if stable_polls >= HANDOFF_STABLE_POLLS {
                            return Ok(true);
                        }
                    }
                    _ => {
                        stable_polls = 0;
                        last_seen_len = Some(current.len);
                    }
                }
            } else {
                stable_polls = 0;
                last_seen_len = None;
            }
        } else {
            stable_polls = 0;
            last_seen_len = None;
        }

        tokio::time::sleep(HANDOFF_POLL_INTERVAL).await;
    }
}

pub async fn summary_handoff(project_path: &str, task_description: &str) -> Result<()> {
    let handoffs_dir = handoffs_dir(project_path);
    fs::create_dir_all(&handoffs_dir).await?;

    let timestamp = Utc::now().to_rfc3339();
    let content = format!(
        "# AI Handoff\n\n- Timestamp: {timestamp}\n- Source: Handover fallback\n\n## Goal\n\n{task_description}\n\n## Status\n\nThe source AI did not create `.handover/handoffs/latest.md` before the timeout. This fallback file contains only the user-provided goal; inspect the project and terminal history before continuing.\n"
    );

    let numbered = next_handoff_path(&handoffs_dir);
    let latest = handoffs_dir.join("latest.md");
    fs::write(&numbered, &content).await?;
    fs::write(&latest, &content).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;

    #[tokio::test]
    async fn summary_handoff_writes_numbered_history_and_latest() {
        let dir = tempfile::tempdir().unwrap();

        summary_handoff(dir.path().to_str().unwrap(), "first task")
            .await
            .unwrap();
        summary_handoff(dir.path().to_str().unwrap(), "second task")
            .await
            .unwrap();

        let handoffs = dir.path().join(".handover/handoffs");
        assert!(handoffs.join("handoff_001.md").is_file());
        assert!(handoffs.join("handoff_002.md").is_file());
        let latest = std::fs::read_to_string(handoffs.join("latest.md")).unwrap();
        assert!(latest.contains("second task"));
        assert!(latest.contains("Handover fallback"));
    }

    #[tokio::test]
    async fn wait_for_latest_handoff_detects_created_file() {
        let dir = tempfile::tempdir().unwrap();
        let project_path = dir.path().to_str().unwrap().to_string();
        let writer_path = latest_handoff_path(&project_path);

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            std::fs::create_dir_all(writer_path.parent().unwrap()).unwrap();
            std::fs::write(writer_path, "ready").unwrap();
        });

        let ready = wait_for_latest_handoff(&project_path, None, Duration::from_secs(5))
            .await
            .unwrap();

        assert!(ready);
    }

    #[tokio::test]
    async fn wait_for_latest_handoff_waits_for_stable_size() {
        let dir = tempfile::tempdir().unwrap();
        let project_path = dir.path().to_str().unwrap().to_string();
        let writer_path = latest_handoff_path(&project_path);

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            std::fs::create_dir_all(writer_path.parent().unwrap()).unwrap();
            std::fs::write(&writer_path, "partial").unwrap();
            tokio::time::sleep(Duration::from_millis(1100)).await;
            std::fs::write(&writer_path, "partial and done").unwrap();
        });

        let ready = wait_for_latest_handoff(
            &project_path,
            None,
            Duration::from_secs(6),
        )
        .await
        .unwrap();

        assert!(ready);
        let content = std::fs::read_to_string(latest_handoff_path(&project_path)).unwrap();
        assert_eq!(content, "partial and done");
    }

    #[tokio::test]
    async fn wait_for_latest_handoff_times_out_when_file_is_not_updated() {
        let dir = tempfile::tempdir().unwrap();
        summary_handoff(dir.path().to_str().unwrap(), "existing")
            .await
            .unwrap();
        let previous = latest_handoff_snapshot(dir.path().to_str().unwrap());

        let ready = wait_for_latest_handoff(
            dir.path().to_str().unwrap(),
            previous,
            Duration::from_millis(50),
        )
        .await
        .unwrap();

        assert!(!ready);
    }

    #[tokio::test]
    async fn git_handoff_commits_changes_and_skips_clean_tree() {
        let dir = tempfile::tempdir().unwrap();
        Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("README.md"), "hello").unwrap();

        let first = git_handoff(dir.path().to_str().unwrap(), "first checkpoint")
            .await
            .unwrap();
        assert_eq!(first, GitHandoffOutcome::Committed);

        let second = git_handoff(dir.path().to_str().unwrap(), "second checkpoint")
            .await
            .unwrap();
        assert_eq!(second, GitHandoffOutcome::NoChanges);

        std::fs::write(dir.path().join("README.md"), "hello again").unwrap();
        let third = git_handoff(dir.path().to_str().unwrap(), "third checkpoint")
            .await
            .unwrap();
        assert_eq!(third, GitHandoffOutcome::Committed);
    }
}
