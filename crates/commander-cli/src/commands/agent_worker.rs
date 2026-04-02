use anyhow::{Context, Result};
use commander_coordination::TaskBoundaryGuard;
use commander_hooks::NoopHookRunner;
use commander_messages::{Message, TranscriptWriter};
use commander_permissions::{PermissionEngine, PermissionMode};
use commander_runtime::{
    create_adapter, run_agent_loop, AgentLoopConfig, AutoApproveObserver, SessionOutcome,
};
use commander_tasks::task::TaskKind;
use commander_tools::builtin;
use commander_tools::registry::ToolRegistry;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Config JSON written by the supervisor before spawning a worker.
#[derive(Debug, Deserialize)]
pub struct WorkerConfig {
    pub task_id: String,
    pub agent_id: String,
    pub attempt_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_task_kind")]
    pub task_kind: TaskKind,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub allowed_files: Vec<String>,
    pub project_id: String,
    pub provider: String,
    pub model: String,
    pub cwd: String,
    #[serde(default)]
    pub task_cwd: Option<String>,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default)]
    pub checkpoint_path: Option<String>,
    #[serde(default)]
    pub heartbeat_path: Option<String>,
}

fn default_system_prompt() -> String {
    "You are a software engineer. Complete the assigned task by reading relevant files, \
     making changes, and verifying your work."
        .into()
}

fn default_max_turns() -> u32 {
    50
}

fn default_max_tokens() -> u32 {
    16384
}

fn default_task_kind() -> TaskKind {
    TaskKind::Implement
}

/// Result written by the worker to the result file.
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkerResult {
    pub task_id: String,
    pub agent_id: String,
    pub attempt_id: String,
    pub status: String,
    pub summary: String,
    #[serde(default)]
    pub criteria_evidence: Vec<CriterionEvidence>,
    #[serde(default)]
    pub issues: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CriterionEvidence {
    pub criterion: String,
    pub evidence: String,
}

pub async fn run(id: &str, config_path: &Path) -> Result<()> {
    // 1. Read config
    let config_str = std::fs::read_to_string(config_path)
        .with_context(|| format!("reading worker config: {}", config_path.display()))?;
    let config: WorkerConfig =
        serde_json::from_str(&config_str).with_context(|| "parsing worker config JSON")?;
    if config.agent_id != id {
        anyhow::bail!(
            "agent id mismatch: CLI id={} but config id={}",
            id,
            config.agent_id
        );
    }

    let cwd = PathBuf::from(&config.cwd);
    let commander_dir = cwd.join(".commander");

    // 2. Create adapter (hard error if fails)
    let adapter = create_adapter(&config.provider, &config.model).with_context(|| {
        format!(
            "creating {} adapter for model {}",
            config.provider, config.model
        )
    })?;

    // 3. Set up tools
    let mut registry = ToolRegistry::new();
    builtin::register_builtins(&mut registry);

    // 4. Permissions: auto-approve for orchestrated agents
    let permissions = PermissionEngine::new(PermissionMode::AutoApprove);

    // 5. Hooks: none for v0
    let hooks = NoopHookRunner;

    // 6. Observer: auto-approve
    let observer = AutoApproveObserver;

    // 7. Transcript
    let transcripts_dir = commander_dir.join("transcripts");
    std::fs::create_dir_all(&transcripts_dir)?;
    let transcript_path = transcripts_dir.join(format!("{}.jsonl", config.agent_id));
    let mut transcript = TranscriptWriter::open(&transcript_path).await?;

    // 8. Build initial messages
    let checkpoint_path = config
        .checkpoint_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            commander_dir
                .join("checkpoints")
                .join(format!("{}.json", config.task_id))
        });
    let heartbeat_path = config
        .heartbeat_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            commander_dir
                .join("heartbeats")
                .join(format!("{}.json", config.agent_id))
        });

    let mut messages = load_or_seed_messages(&config, &checkpoint_path)?;

    let path_guard = Arc::new(TaskBoundaryGuard::new_with_workspace(
        config.allowed_files.clone(),
        cwd.clone(),
    )) as Arc<dyn commander_tools::PathGuard>;

    let hb_cancel = CancellationToken::new();
    write_heartbeat(&heartbeat_path, "starting")?;
    let hb_task = {
        let hb_path = heartbeat_path.clone();
        let hb_cancel = hb_cancel.clone();
        tokio::spawn(async move {
            loop {
                if hb_cancel.is_cancelled() {
                    break;
                }
                let _ = write_heartbeat(&hb_path, "running");
                tokio::select! {
                    _ = hb_cancel.cancelled() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {}
                }
            }
        })
    };

    // 9. Run agent loop
    let cancel = CancellationToken::new();
    let mut loop_env = HashMap::new();
    if let Some(task_cwd) = &config.task_cwd {
        loop_env.insert("COMMANDER_TASK_CWD".into(), task_cwd.clone());
    }

    let loop_config = AgentLoopConfig {
        max_turns: config.max_turns,
        cwd: cwd.clone(),
        session_id: config.agent_id.clone(),
        env: loop_env,
        system_prompt: Some(config.system_prompt.clone()),
        max_tokens: config.max_tokens,
        checkpoint_path: Some(checkpoint_path.clone()),
    };

    tracing::info!(
        task_id = config.task_id,
        agent_id = config.agent_id,
        provider = config.provider,
        model = config.model,
        "agent worker starting"
    );

    let outcome = run_agent_loop(
        loop_config,
        adapter.as_ref(),
        &registry,
        &permissions,
        &hooks,
        &observer,
        &mut transcript,
        &mut messages,
        cancel,
        Some(path_guard),
    )
    .await;

    // 10. Write result file
    let results_dir = commander_dir.join("results");
    std::fs::create_dir_all(&results_dir)?;

    let result = match outcome {
        Ok(SessionOutcome::EndTurn) => match extract_completion_signal(&messages) {
            Some(done) => WorkerResult {
                task_id: config.task_id.clone(),
                agent_id: config.agent_id.clone(),
                attempt_id: config.attempt_id.clone(),
                status: "complete".into(),
                summary: done.summary,
                criteria_evidence: done.criteria_evidence,
                issues: vec![],
            },
            None => WorkerResult {
                task_id: config.task_id.clone(),
                agent_id: config.agent_id.clone(),
                attempt_id: config.attempt_id.clone(),
                status: "incomplete".into(),
                summary: "turn ended without complete_task".into(),
                criteria_evidence: vec![],
                issues: vec!["missing complete_task".into()],
            },
        },
        Ok(SessionOutcome::MaxTurns) => WorkerResult {
            task_id: config.task_id.clone(),
            agent_id: config.agent_id.clone(),
            attempt_id: config.attempt_id.clone(),
            status: "max_turns".into(),
            summary: format!("reached max turns ({})", config.max_turns),
            criteria_evidence: vec![],
            issues: vec![],
        },
        Ok(SessionOutcome::Cancelled) => WorkerResult {
            task_id: config.task_id.clone(),
            agent_id: config.agent_id.clone(),
            attempt_id: config.attempt_id.clone(),
            status: "failed".into(),
            summary: "cancelled".into(),
            criteria_evidence: vec![],
            issues: vec![],
        },
        Err(e) => WorkerResult {
            task_id: config.task_id.clone(),
            agent_id: config.agent_id.clone(),
            attempt_id: config.attempt_id.clone(),
            status: "failed".into(),
            summary: format!("error: {e}"),
            criteria_evidence: vec![],
            issues: vec![e.to_string()],
        },
    };

    write_result_atomic(&results_dir, &config.task_id, &config.agent_id, &result)?;
    hb_cancel.cancel();
    let _ = hb_task.await;
    let phase = if result.status == "complete" {
        "completed"
    } else {
        "failed"
    };
    write_heartbeat(&heartbeat_path, phase)?;

    tracing::info!(
        task_id = config.task_id,
        status = result.status,
        "agent worker finished"
    );

    if worker_failed(&result.status) {
        anyhow::bail!("worker finished with status: {}", result.status);
    }

    Ok(())
}

/// Returns true when the worker should cause the process to exit non-zero.
///
/// "failed" and "max_turns" are treated as errors so the supervisor can
/// detect failures from the process exit code alone, not just the result file.
fn worker_failed(status: &str) -> bool {
    matches!(status, "failed" | "max_turns")
}

/// Write result file atomically: tmp → fsync → rename.
fn write_result_atomic(
    results_dir: &Path,
    task_id: &str,
    agent_id: &str,
    result: &WorkerResult,
) -> Result<()> {
    let path = results_dir.join(format!("{task_id}-{agent_id}.json"));
    let tmp = path.with_extension("json.tmp");

    let json = serde_json::to_string_pretty(result)?;
    std::fs::write(&tmp, &json)?;

    // fsync
    let file = std::fs::File::open(&tmp)?;
    file.sync_all()?;
    drop(file);

    // atomic rename
    std::fs::rename(&tmp, &path)?;

    // Sync the parent directory so the rename is durable on crash.
    let dir = std::fs::File::open(results_dir)?;
    dir.sync_all()?;

    Ok(())
}

fn extract_last_assistant_text(messages: &[Message]) -> String {
    messages
        .iter()
        .rev()
        .find(|m| m.role == commander_messages::Role::Assistant)
        .and_then(|m| m.text())
        .unwrap_or("completed")
        .chars()
        .take(500)
        .collect()
}

fn load_or_seed_messages(config: &WorkerConfig, checkpoint_path: &Path) -> Result<Vec<Message>> {
    if checkpoint_path.exists() {
        let raw = std::fs::read_to_string(checkpoint_path)?;
        if let Ok(messages) = serde_json::from_str::<Vec<Message>>(&raw) {
            if !messages.is_empty() {
                return Ok(messages);
            }
        }
    }

    let mut task_prompt = if config.description.is_empty() {
        config.title.clone()
    } else {
        format!("{}\n\n{}", config.title, config.description)
    };

    task_prompt.push_str("\n\nWorkspace context:\n");
    task_prompt.push_str("- Working directory: ");
    task_prompt.push_str(&config.cwd);
    task_prompt.push('\n');
    if let Some(task_cwd) = &config.task_cwd {
        task_prompt.push_str("- Preferred shell working directory: ");
        task_prompt.push_str(task_cwd);
        task_prompt.push('\n');
    }
    if !config.allowed_files.is_empty() {
        task_prompt.push_str("- Allowed file paths (relative to working directory):\n");
        for path in &config.allowed_files {
            task_prompt.push_str("  - ");
            task_prompt.push_str(path);
            task_prompt.push('\n');
        }
        task_prompt.push_str(
            "- Only read/write files within allowed paths; boundary checks will reject others.\n",
        );
    }

    if !config.acceptance_criteria.is_empty() {
        task_prompt.push_str("\n\nAcceptance criteria:\n");
        for criterion in &config.acceptance_criteria {
            task_prompt.push_str("- ");
            task_prompt.push_str(criterion);
            task_prompt.push('\n');
        }
    }

    task_prompt.push_str("\nExecution requirements:\n");
    task_prompt
        .push_str("- Start by inspecting files in the working directory and allowed paths.\n");
    task_prompt.push_str(
        "- For Bash commands, run from the preferred shell working directory when provided.\n",
    );
    match config.task_kind {
        TaskKind::Implement => {
            task_prompt.push_str("- Make concrete file edits that satisfy acceptance criteria.\n");
        }
        TaskKind::Explore => {
            task_prompt.push_str(
                "- This is an exploration task: prioritize investigation and reporting. File edits are optional.\n",
            );
        }
    }
    task_prompt.push_str(
        "- Verify outcomes with finite commands when relevant (for example tests/build).\n",
    );
    task_prompt.push_str(
        "- Do NOT run long-lived dev servers (for example npm run dev, vite, npm start).\n",
    );
    task_prompt.push_str("- Call complete_task only after requirements are satisfied.\n");
    Ok(vec![Message::user(&task_prompt)])
}

fn write_heartbeat(path: &Path, phase: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let payload = json!({
        "updated_at": chrono::Utc::now().to_rfc3339(),
        "phase": phase,
    });
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&payload)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

struct CompletionSignal {
    summary: String,
    criteria_evidence: Vec<CriterionEvidence>,
}

fn extract_completion_signal(messages: &[Message]) -> Option<CompletionSignal> {
    let tool_input = messages
        .iter()
        .rev()
        .filter(|m| m.role == commander_messages::Role::Assistant)
        .find_map(|m| {
            m.content.iter().find_map(|block| match block {
                commander_messages::ContentBlock::ToolUse { name, input, .. }
                    if name == "complete_task" =>
                {
                    Some(input.clone())
                }
                _ => None,
            })
        })?;

    let summary = tool_input
        .get("summary")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| extract_last_assistant_text(messages));

    let criteria_evidence = tool_input
        .get("criteria_met")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let criterion = item.get("criterion")?.as_str()?;
                    let evidence = item.get("evidence")?.as_str()?;
                    Some(CriterionEvidence {
                        criterion: criterion.to_string(),
                        evidence: evidence.to_string(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(CompletionSignal {
        summary,
        criteria_evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_failed_returns_true_for_error_statuses() {
        assert!(worker_failed("failed"));
        assert!(worker_failed("max_turns"));
    }

    #[test]
    fn worker_failed_returns_false_for_success_statuses() {
        assert!(!worker_failed("complete"));
        assert!(!worker_failed("incomplete"));
        assert!(!worker_failed(""));
    }

    #[test]
    fn write_result_atomic_creates_durable_file() {
        let dir = tempfile::tempdir().unwrap();
        let result = WorkerResult {
            task_id: "t1".into(),
            agent_id: "a1".into(),
            attempt_id: "att1".into(),
            status: "complete".into(),
            summary: "done".into(),
            criteria_evidence: vec![],
            issues: vec![],
        };
        write_result_atomic(dir.path(), "t1", "a1", &result).unwrap();

        let path = dir.path().join("t1-a1.json");
        assert!(path.exists(), "result file should exist");

        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists(), "tmp file should be cleaned up after rename");

        let parsed: WorkerResult =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed.status, "complete");
        assert_eq!(parsed.summary, "done");
    }

    #[test]
    fn write_result_atomic_overwrites_previous() {
        let dir = tempfile::tempdir().unwrap();
        let result1 = WorkerResult {
            task_id: "t1".into(),
            agent_id: "a1".into(),
            attempt_id: "att1".into(),
            status: "failed".into(),
            summary: "first".into(),
            criteria_evidence: vec![],
            issues: vec![],
        };
        write_result_atomic(dir.path(), "t1", "a1", &result1).unwrap();

        let result2 = WorkerResult {
            task_id: "t1".into(),
            agent_id: "a1".into(),
            attempt_id: "att2".into(),
            status: "complete".into(),
            summary: "second".into(),
            criteria_evidence: vec![],
            issues: vec![],
        };
        write_result_atomic(dir.path(), "t1", "a1", &result2).unwrap();

        let path = dir.path().join("t1-a1.json");
        let parsed: WorkerResult =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed.status, "complete");
        assert_eq!(parsed.summary, "second");
        assert_eq!(parsed.attempt_id, "att2");
    }
}
