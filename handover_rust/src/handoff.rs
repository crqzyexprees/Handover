use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use git2::{Repository, Signature};
use serde::Serialize;
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

pub fn summary_template_instructions(template: &str) -> &'static str {
    match template {
        "nextjs" => "Project type: Next.js. Emphasize app router routes, API routes, React components and hooks, env vars (.env.local), and npm/pnpm scripts run or needed.",
        "rust-cli" => "Project type: Rust CLI/backend. Emphasize crates and modules changed, public APIs, cargo commands (build/test/clippy), test output, and CLI flags or config touched.",
        _ => "Include: current goal, project context, files changed, commands run, tests/results, decisions made, blockers, and exact next steps.",
    }
}

pub fn build_summary_source_prompt(task: &str, template: &str) -> String {
    let extra = summary_template_instructions(template);
    format!(
        "Create a handoff file for the next AI now. Write concise Markdown to '.handover/handoffs/latest.md' and also create the next numbered history file under '.handover/handoffs/' as 'handoff_NNN.md' with the same content. {extra} Overall goal: {task}. After writing the files, stop and wait."
    )
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

#[derive(Debug, Clone, Serialize)]
pub struct HandoffFileInfo {
    pub filename: String,
    pub modified: Option<String>,
    pub size_bytes: u64,
    pub is_latest: bool,
}

pub fn is_valid_handoff_filename(filename: &str) -> bool {
    if filename == "latest.md" {
        return true;
    }
    let Some(num) = filename
        .strip_prefix("handoff_")
        .and_then(|s| s.strip_suffix(".md"))
    else {
        return false;
    };
    !num.is_empty() && num.chars().all(|c| c.is_ascii_digit())
}

pub async fn list_handoff_files(project_path: &str) -> Result<Vec<HandoffFileInfo>> {
    let dir = handoffs_dir(project_path);
    if !fs::try_exists(&dir).await.unwrap_or(false) {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let mut read_dir = fs::read_dir(&dir)
        .await
        .context("failed to read handoffs dir")?;
    while let Some(entry) = read_dir.next_entry().await? {
        let filename = entry.file_name().to_string_lossy().into_owned();
        if !filename.ends_with(".md") || !is_valid_handoff_filename(&filename) {
            continue;
        }
        let metadata = entry.metadata().await?;
        let modified = metadata.modified().ok().and_then(system_time_to_rfc3339);
        entries.push(HandoffFileInfo {
            is_latest: filename == "latest.md",
            filename,
            modified,
            size_bytes: metadata.len(),
        });
    }

    entries.sort_by(|a, b| handoff_sort_key(&b.filename).cmp(&handoff_sort_key(&a.filename)));
    Ok(entries)
}

fn handoff_sort_key(filename: &str) -> u32 {
    if filename == "latest.md" {
        return u32::MAX;
    }
    filename
        .strip_prefix("handoff_")
        .and_then(|s| s.strip_suffix(".md"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn system_time_to_rfc3339(time: SystemTime) -> Option<String> {
    let datetime: DateTime<Utc> = time.into();
    Some(datetime.to_rfc3339())
}

pub async fn read_handoff_file(project_path: &str, filename: &str) -> Result<String> {
    if !is_valid_handoff_filename(filename) {
        anyhow::bail!("invalid handoff filename");
    }
    let path = handoffs_dir(project_path).join(filename);
    if !fs::try_exists(&path).await.unwrap_or(false) {
        anyhow::bail!("handoff file not found");
    }
    fs::read_to_string(&path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))
}

pub async fn export_handoff_log(project_path: &str) -> Result<String> {
    let files = list_handoff_files(project_path).await?;
    if files.is_empty() {
        return Ok(format!(
            "# Handover Log\n\n_No handoff files found under `{}`._\n",
            handoffs_dir(project_path).display()
        ));
    }

    let exported_at = Utc::now().to_rfc3339();
    let mut out = format!("# Handover Log\n\n- Exported: {exported_at}\n\n");

    for file in files {
        let modified = file.modified.as_deref().unwrap_or("unknown time");
        let content = read_handoff_file(project_path, &file.filename).await?;
        out.push_str(&format!(
            "---\n\n## {} ({modified})\n\n{content}\n",
            file.filename
        ));
    }
    Ok(out)
}

pub async fn diff_handoff_files(project_path: &str, from: &str, to: &str) -> Result<String> {
    if !is_valid_handoff_filename(from) || !is_valid_handoff_filename(to) {
        anyhow::bail!("invalid handoff filename");
    }
    let from_text = read_handoff_file(project_path, from).await?;
    let to_text = read_handoff_file(project_path, to).await?;
    Ok(unified_line_diff(from, to, &from_text, &to_text))
}

pub fn git_diff_range(project_path: &str, from_ref: &str, to_ref: &str) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["-C", project_path, "diff", from_ref, to_ref])
        .output()
        .context("failed to spawn git")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git diff failed: {stderr}");
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn unified_line_diff(from_label: &str, to_label: &str, from_text: &str, to_text: &str) -> String {
    use similar::TextDiff;
    let diff = TextDiff::from_lines(from_text, to_text);
    diff.unified_diff()
        .header(&format!("a/{from_label}"), &format!("b/{to_label}"))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;

    #[test]
    fn build_summary_source_prompt_includes_template_hints() {
        let generic = build_summary_source_prompt("ship feature X", "generic");
        assert!(generic.contains("files changed"));
        assert!(generic.contains("ship feature X"));

        let nextjs = build_summary_source_prompt("fix login", "nextjs");
        assert!(nextjs.contains("Next.js"));
        assert!(nextjs.contains("fix login"));

        let rust_cli = build_summary_source_prompt("add subcommand", "rust-cli");
        assert!(rust_cli.contains("Rust CLI"));
        assert!(rust_cli.contains("cargo commands"));
    }

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

        let ready = wait_for_latest_handoff(&project_path, None, Duration::from_secs(6))
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
    async fn list_and_export_handoff_files() {
        let dir = tempfile::tempdir().unwrap();
        let project_path = dir.path().to_str().unwrap();

        summary_handoff(project_path, "first").await.unwrap();
        std::fs::write(
            handoffs_dir(project_path).join("handoff_001.md"),
            "# First handoff\n",
        )
        .unwrap();

        let files = list_handoff_files(project_path).await.unwrap();
        assert!(files.iter().any(|f| f.filename == "latest.md"));
        assert!(files.iter().any(|f| f.filename == "handoff_001.md"));

        let export = export_handoff_log(project_path).await.unwrap();
        assert!(export.contains("# Handover Log"));
        assert!(export.contains("handoff_001.md"));
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
