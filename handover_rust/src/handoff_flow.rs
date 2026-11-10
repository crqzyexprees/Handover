//! Running a Handoff (see GLOSSARY.md): move a Project's in-progress work
//! from one Instance's AI to another's.
//!
//! [`run`] owns the whole flow — prompting the source AI, waiting for its
//! handoff file, the fallback on timeout, the git checkpoint, and prompting
//! the target — and returns a typed outcome. Prompts go through a
//! [`PromptSink`], so tests can record them instead of driving terminals.

use std::future::Future;
use std::time::Duration;

use crate::handoff;
use crate::model::{HandoffMethod, HandoffTemplate};

/// Types a prompt into an Instance's terminal and submits it.
pub trait PromptSink {
    /// False if the Instance has no running shell.
    fn send_prompt(&self, instance_id: &str, prompt: &str) -> impl Future<Output = bool> + Send;
}

pub struct HandoffPlan<'a> {
    pub project_path: &'a str,
    pub from_instance_id: &'a str,
    pub to_instance_id: &'a str,
    pub method: HandoffMethod,
    pub template: HandoffTemplate,
    pub task: &'a str,
    /// How long the source AI gets to write its handoff file.
    pub summary_wait: Duration,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HandoffOutcome {
    GitCommitted,
    GitNoChanges,
    SummaryWritten,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HandoffFailure {
    NotGitRepo,
    /// The source AI wrote nothing in time; a goal-only fallback file was written.
    SummaryTimedOut,
    SourceNotRunning,
    TargetNotRunning,
    Internal(String),
}

impl HandoffOutcome {
    /// Stable identifier the frontend switches on.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::GitCommitted => "git_committed",
            Self::GitNoChanges => "git_no_changes",
            Self::SummaryWritten => "summary_written",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::GitCommitted => "Handoff complete",
            Self::GitNoChanges => "Handoff complete; no git changes needed a new commit",
            Self::SummaryWritten => {
                "Source AI wrote the handoff file; target AI was directed to read it"
            }
        }
    }
}

impl HandoffFailure {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotGitRepo => "not_git_repo",
            Self::SummaryTimedOut => "summary_timed_out",
            Self::SourceNotRunning => "source_not_running",
            Self::TargetNotRunning => "target_not_running",
            Self::Internal(_) => "internal",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotGitRepo => {
                "Project is not a Git repository. Initialize git first or use Summary handoff.".into()
            }
            Self::SummaryTimedOut => "Source AI did not create a handoff file in time. A fallback handoff file was written with only the goal you provided; review it before handing off manually.".into(),
            Self::SourceNotRunning => "from_instance does not have an active terminal session".into(),
            Self::TargetNotRunning => "to_instance does not have an active terminal session".into(),
            Self::Internal(e) => e.clone(),
        }
    }
}

fn git_target_prompt(outcome: &HandoffOutcome, task: &str) -> String {
    match outcome {
        HandoffOutcome::GitNoChanges => format!(
            "You are taking over this project. The previous AI found no uncommitted changes to checkpoint. Please inspect the current worktree and recent history with 'git status' and 'git log -1 --stat'. Your overall goal is: {task}. Continue working on this goal from where the previous AI left off."
        ),
        _ => format!(
            "You are taking over this project. The previous AI just committed their work. Please review the latest git commit using 'git log -1 -p' to understand the current state of the codebase. Your overall goal is: {task}. Continue working on this goal from where the previous AI left off."
        ),
    }
}

fn summary_target_prompt(task: &str) -> String {
    format!(
        "You are taking over this project. Read '.handover/handoffs/latest.md' for the handoff from the previous AI. Your overall goal is: {task}. Continue from that file."
    )
}

pub async fn run(
    plan: HandoffPlan<'_>,
    sink: &impl PromptSink,
) -> Result<HandoffOutcome, HandoffFailure> {
    let (outcome, target_prompt) = match plan.method {
        HandoffMethod::Git => {
            let checkpoint = format!("Handoff checkpoint: {}", plan.task);
            let outcome = match handoff::git_handoff(plan.project_path, &checkpoint).await {
                Ok(handoff::GitHandoffOutcome::Committed) => HandoffOutcome::GitCommitted,
                Ok(handoff::GitHandoffOutcome::NoChanges) => HandoffOutcome::GitNoChanges,
                Err(_) => return Err(HandoffFailure::NotGitRepo),
            };
            let prompt = git_target_prompt(&outcome, plan.task);
            (outcome, prompt)
        }
        HandoffMethod::Summary => {
            let previous = handoff::latest_handoff_snapshot(plan.project_path);
            let source_prompt =
                handoff::build_summary_source_prompt(plan.task, plan.template.as_str());
            if !sink
                .send_prompt(plan.from_instance_id, &source_prompt)
                .await
            {
                return Err(HandoffFailure::SourceNotRunning);
            }
            let ready =
                handoff::wait_for_latest_handoff(plan.project_path, previous, plan.summary_wait)
                    .await
                    .map_err(|e| {
                        HandoffFailure::Internal(format!(
                            "failed while waiting for handoff file: {e}"
                        ))
                    })?;
            if !ready {
                handoff::summary_handoff(plan.project_path, plan.task)
                    .await
                    .map_err(|e| {
                        HandoffFailure::Internal(format!(
                            "failed to write fallback handoff file: {e}"
                        ))
                    })?;
                return Err(HandoffFailure::SummaryTimedOut);
            }
            (
                HandoffOutcome::SummaryWritten,
                summary_target_prompt(plan.task),
            )
        }
    };

    if !sink.send_prompt(plan.to_instance_id, &target_prompt).await {
        return Err(HandoffFailure::TargetNotRunning);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records prompts; optionally plays a source AI that writes the handoff file.
    struct FakeTerminals {
        running: Vec<&'static str>,
        writes_handoff_in: Option<String>,
        sent: Mutex<Vec<(String, String)>>,
    }

    impl FakeTerminals {
        fn new(running: Vec<&'static str>) -> Self {
            Self {
                running,
                writes_handoff_in: None,
                sent: Mutex::new(Vec::new()),
            }
        }
        fn sent_to(&self) -> Vec<String> {
            self.sent
                .lock()
                .unwrap()
                .iter()
                .map(|(i, _)| i.clone())
                .collect()
        }
        fn prompt_for(&self, id: &str) -> String {
            self.sent
                .lock()
                .unwrap()
                .iter()
                .find(|(i, _)| i == id)
                .map(|(_, p)| p.clone())
                .unwrap_or_default()
        }
    }

    impl PromptSink for FakeTerminals {
        async fn send_prompt(&self, instance_id: &str, prompt: &str) -> bool {
            if !self.running.contains(&instance_id) {
                return false;
            }
            self.sent
                .lock()
                .unwrap()
                .push((instance_id.into(), prompt.into()));
            if let Some(path) = &self.writes_handoff_in {
                if instance_id == "src" {
                    let dir = std::path::Path::new(path).join(".handover/handoffs");
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(dir.join("latest.md"), "# handoff\nnext: ship it\n").unwrap();
                }
            }
            true
        }
    }

    fn plan<'a>(path: &'a str, method: HandoffMethod, wait_ms: u64) -> HandoffPlan<'a> {
        HandoffPlan {
            project_path: path,
            from_instance_id: "src",
            to_instance_id: "dst",
            method,
            template: HandoffTemplate::Generic,
            task: "ship it",
            summary_wait: Duration::from_millis(wait_ms),
        }
    }

    #[tokio::test]
    async fn summary_prompts_source_then_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let mut terms = FakeTerminals::new(vec!["src", "dst"]);
        terms.writes_handoff_in = Some(path.into());
        let out = run(plan(path, HandoffMethod::Summary, 10_000), &terms).await;
        assert_eq!(out, Ok(HandoffOutcome::SummaryWritten));
        assert_eq!(terms.sent_to(), ["src", "dst"]);
        assert!(terms.prompt_for("dst").contains("latest.md"));
    }

    #[tokio::test]
    async fn summary_timeout_writes_fallback_and_never_prompts_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let terms = FakeTerminals::new(vec!["src", "dst"]);
        let out = run(plan(path, HandoffMethod::Summary, 50), &terms).await;
        assert_eq!(out, Err(HandoffFailure::SummaryTimedOut));
        assert_eq!(terms.sent_to(), ["src"]);
        let fallback =
            std::fs::read_to_string(dir.path().join(".handover/handoffs/latest.md")).unwrap();
        assert!(fallback.contains("ship it"));
    }

    #[tokio::test]
    async fn summary_needs_a_running_source() {
        let dir = tempfile::tempdir().unwrap();
        let terms = FakeTerminals::new(vec!["dst"]);
        let out = run(
            plan(dir.path().to_str().unwrap(), HandoffMethod::Summary, 50),
            &terms,
        )
        .await;
        assert_eq!(out, Err(HandoffFailure::SourceNotRunning));
    }

    #[tokio::test]
    async fn git_commits_and_points_target_at_the_commit() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.txt"), "hi").unwrap();
        let terms = FakeTerminals::new(vec!["src", "dst"]);
        let out = run(
            plan(dir.path().to_str().unwrap(), HandoffMethod::Git, 0),
            &terms,
        )
        .await;
        assert_eq!(out, Ok(HandoffOutcome::GitCommitted));
        assert_eq!(terms.sent_to(), ["dst"]);
        assert!(terms.prompt_for("dst").contains("git log -1 -p"));
    }

    #[tokio::test]
    async fn git_outside_a_repo_fails_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let terms = FakeTerminals::new(vec!["src", "dst"]);
        let out = run(
            plan(dir.path().to_str().unwrap(), HandoffMethod::Git, 0),
            &terms,
        )
        .await;
        assert_eq!(out, Err(HandoffFailure::NotGitRepo));
        assert!(terms.sent_to().is_empty());
    }

    #[tokio::test]
    async fn git_reports_target_not_running() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let terms = FakeTerminals::new(vec!["src"]);
        let out = run(
            plan(dir.path().to_str().unwrap(), HandoffMethod::Git, 0),
            &terms,
        )
        .await;
        assert_eq!(out, Err(HandoffFailure::TargetNotRunning));
    }
}
