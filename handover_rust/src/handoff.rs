use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use git2::Repository;
use tokio::fs;

pub async fn git_handoff(project_path: &str, summary_text: &str) -> Result<()> {
    let path = project_path.to_string();
    let message = summary_text.to_string();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let repo = Repository::open(&path).context("not a git repository")?;
        let mut index = repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let sig = repo.signature()?;
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        match parent {
            Some(parent) => repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&parent])?,
            None => repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[])?,
        };
        Ok(())
    })
    .await??;
    Ok(())
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
