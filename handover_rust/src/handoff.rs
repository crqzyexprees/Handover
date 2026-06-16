#[cfg(test)]
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
#[cfg(test)]
use chrono::Utc;
use git2::{Repository, Signature};
#[cfg(test)]
use tokio::fs;

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

#[cfg(test)]
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

#[cfg(test)]
pub async fn summary_handoff(project_path: &str, task_description: &str) -> Result<()> {
    let handoffs_dir = Path::new(project_path).join(".handover/handoffs");
    fs::create_dir_all(&handoffs_dir).await?;

    let timestamp = Utc::now().to_rfc3339();
    let content =
        format!("# AI Handoff\n\n- Timestamp: {timestamp}\n\n## Task\n\n{task_description}\n");

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
