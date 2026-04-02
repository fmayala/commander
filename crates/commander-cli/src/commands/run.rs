use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use commander_concurrency::key::derive_key;
use commander_concurrency::policy::ConcurrencyPolicy;
use commander_concurrency::slot::SlotManager;
use commander_coordination::{
    parse_git_status_paths, BoundaryCheckStep, RunTestsStep, ValidationContext, ValidationPipeline,
};
use commander_supervisor::singleton::SupervisorLock;
use commander_tasks::task::TaskKind;
use rusqlite::params;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use tokio::process::Child;
use tokio_util::sync::CancellationToken;

use crate::commands::agent_worker::WorkerResult;
use crate::config::CommanderConfig;

/// Active agent process tracked by the supervisor.
struct ActiveAgent {
    process: AgentProcess,
    task_id: String,
    agent_id: String,
    group_key: String,
    config_path: PathBuf,
    result_path: PathBuf,
    checkpoint_path: PathBuf,
    heartbeat_path: PathBuf,
    stderr_log_path: PathBuf,
    baseline_file_path: PathBuf,
    baseline_paths: Vec<String>,
    started_at: DateTime<Utc>,
    last_nudge_at: Option<DateTime<Utc>>,
    restart_count: u32,
}

enum AgentProcess {
    Child(Child),
    Detached { pid: u32, proc_start_time: u64 },
}

struct RunnableTask {
    id: String,
    project_id: String,
    title: String,
    description: String,
    kind: TaskKind,
    acceptance_criteria: Vec<String>,
    files: Vec<String>,
}

pub async fn run(project_dir: &Path) -> Result<()> {
    let config = crate::config::load_config(project_dir)?;

    // 1. Acquire singleton lock
    let lock_path = crate::config::lock_path(project_dir);
    let _lock = SupervisorLock::acquire(&lock_path).map_err(|e| anyhow::anyhow!("{e}"))?;
    tracing::info!("supervisor lock acquired");

    // 2. Open database
    let db_path = crate::config::db_path(project_dir);
    let conn = crate::db::open_db(&db_path)?;

    // 3. Set up concurrency
    let policies = vec![ConcurrencyPolicy {
        key_expr: "project_id".into(),
        max_runs: config.supervisor.max_agents,
        strategy: Default::default(),
    }];
    let mut slot_mgr = SlotManager::new(policies);

    // 4. Resolve binary path for spawning agent-worker
    let binary_path = std::env::current_exe()?.to_string_lossy().to_string();

    // 5. Ensure directories exist
    let commander_dir = crate::config::commander_dir(project_dir);
    let agents_dir = commander_dir.join("agents");
    let results_dir = commander_dir.join("results");
    let checkpoints_dir = commander_dir.join("checkpoints");
    let heartbeats_dir = commander_dir.join("heartbeats");
    let logs_dir = commander_dir.join("logs");
    let baselines_dir = commander_dir.join("baselines");
    std::fs::create_dir_all(&agents_dir)?;
    std::fs::create_dir_all(&results_dir)?;
    std::fs::create_dir_all(&checkpoints_dir)?;
    std::fs::create_dir_all(&heartbeats_dir)?;
    std::fs::create_dir_all(&logs_dir)?;
    std::fs::create_dir_all(&baselines_dir)?;

    // 7. Reconcile stale claims and reattach to live workers
    let mut active_agents = reconcile_startup_state(
        &conn,
        &mut slot_mgr,
        &agents_dir,
        &results_dir,
        &checkpoints_dir,
        &heartbeats_dir,
        &logs_dir,
        &baselines_dir,
    )?;

    let tick_interval = std::time::Duration::from_millis(config.supervisor.tick_interval_ms);

    println!(
        "Commander supervisor running (provider={}, model={}, max_agents={}, tick={}ms)",
        config.runtime.provider(),
        config.runtime.default_model,
        config.supervisor.max_agents,
        config.supervisor.tick_interval_ms
    );
    println!("Press Ctrl+C to stop.\n");

    // Set up graceful shutdown on SIGTERM / SIGINT.
    let shutdown = CancellationToken::new();
    {
        let shutdown_clone = shutdown.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("received shutdown signal, draining active agents");
            println!("\nShutdown requested — waiting for active agents to finish...");
            shutdown_clone.cancel();
        });
    }

    // 8. Management loop
    loop {
        // --- Step A: Poll for completed agents ---
        let mut i = 0;
        while i < active_agents.len() {
            let exit_code = poll_exit_code(&mut active_agents[i])?;
            if let Some(exit_code) = exit_code {
                let agent = active_agents.remove(i);
                let disposition =
                    disposition_from_exit(exit_code, &agent.result_path, &agent.stderr_log_path);

                let attempt = count_task_attempts(&conn, &agent.task_id)?;
                let max_attempts = config.validation.max_fix_cycles.saturating_add(1);

                let resolution = match disposition {
                    Disposition::CandidateComplete(result) => {
                        if let Some(reason) =
                            check_completion_contract(&conn, &agent.task_id, &result)?
                        {
                            if attempt < max_attempts {
                                println!(
                                    "  ↻ {} completion evidence missing, retrying: {}",
                                    agent.task_id, reason
                                );
                                TaskResolution::Retry(reason)
                            } else {
                                println!(
                                    "  ✗ {} completion evidence missing, escalating: {}",
                                    agent.task_id, reason
                                );
                                TaskResolution::Escalate(reason)
                            }
                        } else {
                            let validation = run_validation(
                                &conn,
                                project_dir,
                                &config,
                                &agent.task_id,
                                &agent.baseline_paths,
                            )
                            .await?;
                            if validation.passed {
                                println!(
                                    "  ✓ {} completed: {}",
                                    agent.task_id,
                                    result.summary.chars().take(80).collect::<String>()
                                );
                                TaskResolution::Complete
                            } else {
                                let reason = summarize_validation_failures(&validation);
                                if attempt < max_attempts {
                                    println!(
                                        "  ↻ {} validation failed, retrying: {}",
                                        agent.task_id, reason
                                    );
                                    TaskResolution::Retry(reason)
                                } else {
                                    println!(
                                        "  ✗ {} validation failed, escalating: {}",
                                        agent.task_id, reason
                                    );
                                    TaskResolution::Escalate(reason)
                                }
                            }
                        }
                    }
                    Disposition::Failed(reason) => {
                        if attempt < max_attempts {
                            println!("  ↻ {} failed, retrying: {}", agent.task_id, reason);
                            TaskResolution::Retry(reason)
                        } else {
                            println!("  ✗ {} failed, escalating: {}", agent.task_id, reason);
                            TaskResolution::Escalate(reason)
                        }
                    }
                };

                // Update task and run status in DB
                match resolution {
                    TaskResolution::Complete => {
                        conn.execute(
                            "UPDATE tasks
                             SET status = 'complete', claimed_by = NULL, updated_at = datetime('now')
                             WHERE id = ?1",
                            params![agent.task_id],
                        )?;
                        conn.execute(
                            "UPDATE agent_runs SET status = 'succeeded' WHERE agent_id = ?1",
                            params![agent.agent_id],
                        )?;
                        let _ = std::fs::remove_file(&agent.baseline_file_path);
                    }
                    TaskResolution::Retry(reason) => {
                        conn.execute(
                            "UPDATE tasks
                             SET status = 'pending', claimed_by = NULL, updated_at = datetime('now')
                             WHERE id = ?1",
                            params![agent.task_id],
                        )?;
                        conn.execute(
                            "UPDATE agent_runs SET status = 'retrying' WHERE agent_id = ?1",
                            params![agent.agent_id],
                        )?;
                        tracing::warn!(
                            task = agent.task_id,
                            reason,
                            "task failed validation, queued for retry"
                        );
                        let _ = std::fs::remove_file(&agent.checkpoint_path);
                        let _ = std::fs::remove_file(&agent.heartbeat_path);
                    }
                    TaskResolution::Escalate(reason) => {
                        conn.execute(
                            "UPDATE tasks
                             SET status = 'escalated', claimed_by = NULL, updated_at = datetime('now')
                             WHERE id = ?1",
                            params![agent.task_id],
                        )?;
                        conn.execute(
                            "UPDATE agent_runs SET status = 'failed' WHERE agent_id = ?1",
                            params![agent.agent_id],
                        )?;
                        let _ = std::fs::remove_file(&agent.baseline_file_path);
                        tracing::warn!(task = agent.task_id, reason, "task failed, escalated");
                    }
                }

                // Release concurrency slot
                slot_mgr.release(&agent.group_key);

                // Clean up config file (keep result file for debugging)
                let _ = std::fs::remove_file(&agent.config_path);
            } else {
                let (stale_ms, phase) = heartbeat_staleness_ms(&active_agents[i]);

                if stale_ms >= config.supervisor.restart_after_ms {
                    let agent = active_agents.remove(i);
                    let reason = format!(
                        "heartbeat stale for {} (phase={phase})",
                        format_stale_ms(stale_ms)
                    );
                    let restart_budget = config.supervisor.max_restarts;
                    if agent.restart_count < restart_budget {
                        println!(
                            "  ↻ {} stalled, restarting ({}/{}): {}",
                            agent.task_id,
                            agent.restart_count + 1,
                            restart_budget,
                            reason
                        );
                        mark_restarted_and_requeue(&conn, &mut slot_mgr, agent, &reason)?;
                    } else {
                        println!(
                            "  ✗ {} stalled and restart budget exhausted, escalating: {}",
                            agent.task_id, reason
                        );
                        mark_failed_and_escalate(&conn, &mut slot_mgr, agent, &reason)?;
                    }
                    continue;
                }

                if stale_ms >= config.supervisor.nudge_after_ms {
                    let now = Utc::now();
                    let should_nudge = active_agents[i]
                        .last_nudge_at
                        .map(|last| {
                            now.signed_duration_since(last).num_milliseconds()
                                >= config.supervisor.nudge_after_ms as i64
                        })
                        .unwrap_or(true);
                    if should_nudge {
                        println!(
                            "  … {} appears stalled ({} since heartbeat, phase={phase})",
                            active_agents[i].task_id,
                            format_stale_ms(stale_ms)
                        );
                        active_agents[i].last_nudge_at = Some(now);
                    }
                } else {
                    active_agents[i].last_nudge_at = None;
                }

                i += 1;
            }
        }

        // --- Step A.5: Reclaim orphaned claims ---
        // Tasks stuck in 'claimed' with no active agent_runs are orphaned (e.g. supervisor
        // crashed between claim and spawn). Reset them to 'pending' after a TTL.
        let reclaimed = reclaim_orphaned_claims(&conn, 60)?;
        if reclaimed > 0 {
            tracing::warn!(count = reclaimed, "reclaimed orphaned task claims");
            println!("  ↺ reclaimed {reclaimed} orphaned task claim(s)");
        }

        // --- Step B: Spawn new agents for runnable tasks ---
        // During graceful shutdown, skip spawning and only drain in-flight agents.
        let runnable = if shutdown.is_cancelled() {
            vec![]
        } else {
            query_runnable_tasks(&conn)?
        };

        for task in &runnable {
            let check_task =
                commander_tasks::task::Task::new(&task.id, &task.project_id, &task.title);
            if !slot_mgr.can_run(&check_task) {
                continue;
            }

            let group_key = derive_key(&check_task, "project_id");
            let agent_id = format!("agent-{}", uuid::Uuid::new_v4().as_simple());
            let next_attempt = count_task_attempts(&conn, &task.id)? + 1;
            let attempt_id = format!("attempt-{next_attempt}");

            // Claim: pending -> claimed
            let changed = conn.execute(
                "UPDATE tasks SET status = 'claimed', claimed_by = ?1, updated_at = datetime('now')
                 WHERE id = ?2 AND status = 'pending'",
                params![agent_id, task.id],
            )?;

            if changed == 0 {
                continue;
            }

            tracing::info!(task = task.id, agent = agent_id, "claimed task");

            // Write agent config
            let config_path = agents_dir.join(format!("{agent_id}.json"));
            let result_path = results_dir.join(format!("{}-{agent_id}.json", task.id));
            let checkpoint_path = checkpoints_dir.join(format!("{}.json", task.id));
            let heartbeat_path = heartbeats_dir.join(format!("{agent_id}.json"));
            let baseline_file_path = baselines_dir.join(format!("{}.json", task.id));
            let restart_count = count_task_restarts(&conn, &task.id)?;
            let baseline_paths = load_or_create_task_baseline(project_dir, &baseline_file_path)
                .unwrap_or_else(|e| {
                    tracing::warn!(task = task.id, error = %e, "failed to prepare task baseline");
                    Vec::new()
                });
            let task_cwd = {
                let derived = derive_task_shell_cwd(project_dir, &task.files);
                derived.canonicalize().unwrap_or(derived)
            };
            if task_cwd == project_dir {
                tracing::debug!(task = task.id, cwd = %task_cwd.display(), "task shell cwd resolved to project root");
            } else {
                tracing::debug!(task = task.id, cwd = %task_cwd.display(), "task shell cwd resolved to subproject");
            }

            let agent_config = serde_json::json!({
                "task_id": task.id,
                "agent_id": agent_id,
                "attempt_id": attempt_id,
                "title": task.title,
                "description": task.description,
                "task_kind": task.kind.as_str(),
                "acceptance_criteria": task.acceptance_criteria,
                "allowed_files": task.files,
                "project_id": task.project_id,
                "provider": config.runtime.provider(),
                "model": config.runtime.default_model,
                "cwd": project_dir.canonicalize()?.to_string_lossy(),
                "task_cwd": task_cwd.to_string_lossy(),
                "system_prompt": "You are a software engineer. Complete the assigned task by reading relevant files, making changes, and verifying your work.",
                "max_turns": 50,
                "max_tokens": config.runtime.max_output_tokens,
                "checkpoint_path": checkpoint_path.to_string_lossy(),
                "heartbeat_path": heartbeat_path.to_string_lossy(),
            });
            std::fs::write(&config_path, serde_json::to_string_pretty(&agent_config)?)?;

            let stdout_log_path = logs_dir.join(format!("{agent_id}.stdout.log"));
            let stderr_log_path = logs_dir.join(format!("{agent_id}.stderr.log"));
            let stdout_file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&stdout_log_path)?;
            let stderr_file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&stderr_log_path)?;

            // Spawn real agent-worker process
            let child = tokio::process::Command::new(&binary_path)
                .arg("agent-worker")
                .arg("--id")
                .arg(&agent_id)
                .arg("--config")
                .arg(&config_path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::from(stdout_file))
                .stderr(std::process::Stdio::from(stderr_file))
                .spawn();

            match child {
                Ok(child) => {
                    let pid = child.id().unwrap_or(0);
                    let proc_start_time = commander_supervisor::proc_start_time(pid).unwrap_or(0);
                    slot_mgr.acquire(&group_key).ok();

                    conn.execute(
                        "INSERT OR REPLACE INTO agent_runs
                         (agent_id, task_id, pid, proc_start_time, started_at, status)
                         VALUES (?1, ?2, ?3, ?4, datetime('now'), 'running')",
                        params![agent_id, task.id, pid, proc_start_time],
                    )?;

                    active_agents.push(ActiveAgent {
                        process: AgentProcess::Child(child),
                        task_id: task.id.clone(),
                        agent_id: agent_id.clone(),
                        group_key,
                        config_path,
                        result_path,
                        checkpoint_path,
                        heartbeat_path,
                        stderr_log_path,
                        baseline_file_path,
                        baseline_paths,
                        started_at: Utc::now(),
                        last_nudge_at: None,
                        restart_count,
                    });

                    println!(
                        "  ▸ spawned {agent_id} (pid {pid}) for {}: {}",
                        task.id, task.title
                    );
                }
                Err(e) => {
                    tracing::error!(task = task.id, "spawn failed: {e}");
                    let unclaimed = conn.execute(
                        "UPDATE tasks SET status = 'pending', claimed_by = NULL, updated_at = datetime('now')
                         WHERE id = ?1 AND status = 'claimed'",
                        params![task.id],
                    )?;
                    if unclaimed == 0 {
                        tracing::error!(
                            task = task.id,
                            "unclaim failed after spawn failure, escalating"
                        );
                        conn.execute(
                            "UPDATE tasks
                             SET status = 'escalated', claimed_by = NULL, updated_at = datetime('now')
                             WHERE id = ?1",
                            params![task.id],
                        )?;
                    }
                    let _ = std::fs::remove_file(&config_path);
                    let _ = std::fs::remove_file(&baseline_file_path);
                }
            }
        }

        let blocked_escalations = escalate_blocked_pending_tasks(&conn)?;
        if blocked_escalations > 0 {
            println!("  ✗ escalated {blocked_escalations} task(s) blocked by failed dependencies");
        }

        // --- Step C: Check if done (or shutdown-drained) ---
        if shutdown.is_cancelled() && active_agents.is_empty() {
            println!("Graceful shutdown complete.");
            break;
        }

        let remaining = count_active_tasks(&conn)?;
        if remaining == 0 && active_agents.is_empty() {
            let total: u32 = conn.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))?;
            if total > 0 {
                let complete: u32 = conn.query_row(
                    "SELECT COUNT(*) FROM tasks WHERE status = 'complete'",
                    [],
                    |r| r.get(0),
                )?;
                let escalated: u32 = conn.query_row(
                    "SELECT COUNT(*) FROM tasks WHERE status = 'escalated'",
                    [],
                    |r| r.get(0),
                )?;
                println!(
                    "\nDone. {complete} completed, {escalated} escalated out of {total} tasks."
                );
                break;
            }
        }

        // Wake immediately when shutdown is requested rather than sleeping out the full tick.
        tokio::select! {
            _ = tokio::time::sleep(tick_interval) => {},
            _ = shutdown.cancelled() => {},
        }
    }

    Ok(())
}

enum Disposition {
    CandidateComplete(WorkerResult),
    Failed(String),
}

enum TaskResolution {
    Complete,
    Retry(String),
    Escalate(String),
}

#[derive(Debug, Deserialize)]
struct HeartbeatSnapshot {
    updated_at: String,
    #[serde(default)]
    phase: String,
}

fn read_result_file(path: &Path) -> Result<WorkerResult> {
    let content = std::fs::read_to_string(path)?;
    let result: WorkerResult = serde_json::from_str(&content)?;
    Ok(result)
}

fn poll_exit_code(agent: &mut ActiveAgent) -> Result<Option<i32>> {
    match &mut agent.process {
        AgentProcess::Child(child) => {
            Ok(child.try_wait()?.map(|status| status.code().unwrap_or(-1)))
        }
        AgentProcess::Detached {
            pid,
            proc_start_time,
        } => {
            if is_expected_process_alive(*pid, *proc_start_time) {
                Ok(None)
            } else {
                // Detached process exited; we may still have a result file.
                Ok(Some(0))
            }
        }
    }
}

fn heartbeat_staleness_ms(agent: &ActiveAgent) -> (u64, String) {
    if let Ok(raw) = std::fs::read_to_string(&agent.heartbeat_path) {
        if let Ok(snapshot) = serde_json::from_str::<HeartbeatSnapshot>(&raw) {
            if let Ok(ts) = DateTime::parse_from_rfc3339(&snapshot.updated_at) {
                let age_ms =
                    non_negative_ms(Utc::now().signed_duration_since(ts.with_timezone(&Utc)));
                let phase = if snapshot.phase.is_empty() {
                    "unknown".into()
                } else {
                    snapshot.phase
                };
                return (age_ms, phase);
            }
        }
    }

    (
        non_negative_ms(Utc::now().signed_duration_since(agent.started_at)),
        "unknown".into(),
    )
}

fn non_negative_ms(delta: chrono::Duration) -> u64 {
    let ms = delta.num_milliseconds();
    if ms <= 0 {
        0
    } else {
        ms as u64
    }
}

fn format_stale_ms(ms: u64) -> String {
    let secs = ms / 1000;
    let minutes = secs / 60;
    let seconds = secs % 60;
    if minutes > 0 {
        format!("{minutes}m{seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

fn read_log_tail(path: &Path, max_chars: usize) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut tail: String = trimmed
        .chars()
        .rev()
        .take(max_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    tail = tail.replace('\n', " | ");
    Some(tail)
}

fn derive_task_shell_cwd(project_dir: &Path, allowed_files: &[String]) -> PathBuf {
    const MANIFEST_FILES: &[&str] = &[
        "package.json",
        "Cargo.toml",
        "pyproject.toml",
        "go.mod",
        "bunfig.toml",
        "pnpm-workspace.yaml",
        "turbo.json",
    ];

    fn has_glob(segment: &str) -> bool {
        segment.contains('*')
            || segment.contains('?')
            || segment.contains('[')
            || segment.contains('{')
    }

    fn candidate_dir_from_pattern(pattern: &str) -> Option<PathBuf> {
        let normalized = pattern.trim().trim_start_matches("./");
        if normalized.is_empty() {
            return None;
        }
        let mut rel = PathBuf::new();
        for segment in normalized.split('/') {
            if segment.is_empty() || has_glob(segment) {
                break;
            }
            rel.push(segment);
        }
        if rel.as_os_str().is_empty() {
            None
        } else {
            Some(rel)
        }
    }

    fn components(path: &Path) -> Vec<String> {
        path.components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().to_string()),
                _ => None,
            })
            .collect()
    }

    fn common_prefix(paths: &[PathBuf]) -> PathBuf {
        let mut it = paths.iter();
        let Some(first) = it.next() else {
            return PathBuf::new();
        };
        let mut prefix = components(first);
        for path in it {
            let next = components(path);
            let keep = prefix
                .iter()
                .zip(next.iter())
                .take_while(|(a, b)| a == b)
                .count();
            prefix.truncate(keep);
            if prefix.is_empty() {
                break;
            }
        }
        prefix.into_iter().collect()
    }

    fn has_manifest(dir: &Path) -> bool {
        MANIFEST_FILES.iter().any(|name| dir.join(name).is_file())
    }

    let mut candidates = Vec::new();
    for pattern in allowed_files {
        let Some(rel) = candidate_dir_from_pattern(pattern) else {
            continue;
        };
        let abs = project_dir.join(&rel);
        if abs.is_dir() {
            candidates.push(rel);
            continue;
        }
        if abs.is_file() {
            if let Some(parent) = rel.parent() {
                candidates.push(parent.to_path_buf());
            }
            continue;
        }
        if let Some(parent) = rel.parent() {
            let abs_parent = project_dir.join(parent);
            if abs_parent.is_dir() {
                candidates.push(parent.to_path_buf());
            }
        }
    }

    if candidates.is_empty() {
        return project_dir.to_path_buf();
    }

    let common = common_prefix(&candidates);
    let mut cursor = if common.as_os_str().is_empty() {
        project_dir.to_path_buf()
    } else {
        project_dir.join(common)
    };

    loop {
        if has_manifest(&cursor) {
            return cursor;
        }
        if cursor == project_dir {
            break;
        }
        let Some(parent) = cursor.parent() else {
            break;
        };
        if !parent.starts_with(project_dir) {
            break;
        }
        cursor = parent.to_path_buf();
    }

    project_dir.to_path_buf()
}

fn mark_restarted_and_requeue(
    conn: &rusqlite::Connection,
    slot_mgr: &mut SlotManager,
    mut agent: ActiveAgent,
    reason: &str,
) -> Result<()> {
    terminate_agent(&mut agent)?;
    conn.execute(
        "UPDATE tasks
         SET status = 'pending', claimed_by = NULL, updated_at = datetime('now')
         WHERE id = ?1",
        params![agent.task_id],
    )?;
    conn.execute(
        "UPDATE agent_runs SET status = 'restarted' WHERE agent_id = ?1",
        params![agent.agent_id],
    )?;
    slot_mgr.release(&agent.group_key);
    let _ = std::fs::remove_file(&agent.config_path);
    let _ = std::fs::remove_file(&agent.checkpoint_path);
    let _ = std::fs::remove_file(&agent.heartbeat_path);
    tracing::warn!(
        task = agent.task_id,
        agent = agent.agent_id,
        checkpoint = %agent.checkpoint_path.display(),
        heartbeat = %agent.heartbeat_path.display(),
        reason,
        "stalled worker terminated and task requeued"
    );
    Ok(())
}

fn mark_failed_and_escalate(
    conn: &rusqlite::Connection,
    slot_mgr: &mut SlotManager,
    mut agent: ActiveAgent,
    reason: &str,
) -> Result<()> {
    terminate_agent(&mut agent)?;
    conn.execute(
        "UPDATE tasks
         SET status = 'escalated', claimed_by = NULL, updated_at = datetime('now')
         WHERE id = ?1",
        params![agent.task_id],
    )?;
    conn.execute(
        "UPDATE agent_runs SET status = 'failed' WHERE agent_id = ?1",
        params![agent.agent_id],
    )?;
    slot_mgr.release(&agent.group_key);
    let _ = std::fs::remove_file(&agent.config_path);
    let _ = std::fs::remove_file(&agent.checkpoint_path);
    let _ = std::fs::remove_file(&agent.heartbeat_path);
    let _ = std::fs::remove_file(&agent.baseline_file_path);
    tracing::warn!(
        task = agent.task_id,
        agent = agent.agent_id,
        checkpoint = %agent.checkpoint_path.display(),
        heartbeat = %agent.heartbeat_path.display(),
        reason,
        "stalled worker escalated"
    );
    Ok(())
}

fn terminate_agent(agent: &mut ActiveAgent) -> Result<()> {
    match &mut agent.process {
        AgentProcess::Child(child) => {
            let _ = child.start_kill();
            let _ = child.try_wait();
            Ok(())
        }
        AgentProcess::Detached { pid, .. } => {
            if commander_supervisor::is_pid_alive(*pid) {
                let _ = std::process::Command::new("kill")
                    .arg("-9")
                    .arg(pid.to_string())
                    .status();
            }
            Ok(())
        }
    }
}

fn parse_sqlite_datetime(value: &str) -> Option<DateTime<Utc>> {
    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").ok()?;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
}

fn disposition_from_exit(
    exit_code: i32,
    result_path: &Path,
    stderr_log_path: &Path,
) -> Disposition {
    match (exit_code, read_result_file(result_path)) {
        (0, Ok(result)) if result.status == "complete" => Disposition::CandidateComplete(result),
        (0, Ok(result)) => Disposition::Failed(result.summary),
        (0, Err(e)) => Disposition::Failed(format!("missing/invalid result file: {e}")),
        (code, _) => {
            let stderr_hint = read_log_tail(stderr_log_path, 500);
            match stderr_hint {
                Some(tail) if !tail.is_empty() => {
                    Disposition::Failed(format!("exit code {code}: {tail}"))
                }
                _ => Disposition::Failed(format!("exit code {code}")),
            }
        }
    }
}

fn reconcile_startup_state(
    conn: &rusqlite::Connection,
    slot_mgr: &mut SlotManager,
    agents_dir: &Path,
    results_dir: &Path,
    checkpoints_dir: &Path,
    heartbeats_dir: &Path,
    logs_dir: &Path,
    baselines_dir: &Path,
) -> Result<Vec<ActiveAgent>> {
    let mut recovered = Vec::new();
    let mut recovered_agent_ids = HashSet::new();
    let mut recovered_claims = 0_u32;
    let mut abandoned_runs = 0_u32;

    let mut stmt = conn.prepare(
        "SELECT id, project_id, claimed_by
         FROM tasks
         WHERE status = 'claimed'",
    )?;

    let claimed_tasks: Vec<(String, String, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for (task_id, project_id, claimed_by) in claimed_tasks {
        let Some(agent_id) = claimed_by else {
            conn.execute(
                "UPDATE tasks
                 SET status = 'pending', claimed_by = NULL, updated_at = datetime('now')
                 WHERE id = ?1",
                params![task_id],
            )?;
            let _ = std::fs::remove_file(checkpoints_dir.join(format!("{task_id}.json")));
            recovered_claims += 1;
            continue;
        };

        let run_row = conn.query_row(
            "SELECT pid, proc_start_time, started_at
             FROM agent_runs
             WHERE agent_id = ?1 AND status = 'running'
             ORDER BY started_at DESC
             LIMIT 1",
            params![agent_id],
            |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        );

        let (pid, proc_start_time, started_at_raw) = match run_row {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                conn.execute(
                    "UPDATE tasks
                     SET status = 'pending', claimed_by = NULL, updated_at = datetime('now')
                     WHERE id = ?1",
                    params![task_id],
                )?;
                let _ = std::fs::remove_file(checkpoints_dir.join(format!("{task_id}.json")));
                let _ = std::fs::remove_file(heartbeats_dir.join(format!("{agent_id}.json")));
                let _ = std::fs::remove_file(agents_dir.join(format!("{agent_id}.json")));
                recovered_claims += 1;
                continue;
            }
            Err(e) => return Err(e.into()),
        };

        if !is_expected_process_alive(pid, proc_start_time) {
            conn.execute(
                "UPDATE tasks
                 SET status = 'pending', claimed_by = NULL, updated_at = datetime('now')
                 WHERE id = ?1",
                params![task_id],
            )?;
            conn.execute(
                "UPDATE agent_runs SET status = 'abandoned' WHERE agent_id = ?1 AND status = 'running'",
                params![agent_id],
            )?;
            let _ = std::fs::remove_file(checkpoints_dir.join(format!("{task_id}.json")));
            let _ = std::fs::remove_file(heartbeats_dir.join(format!("{agent_id}.json")));
            let _ = std::fs::remove_file(agents_dir.join(format!("{agent_id}.json")));
            abandoned_runs += 1;
            recovered_claims += 1;
            continue;
        }

        let check_task = commander_tasks::task::Task::new(&task_id, &project_id, &task_id);
        let group_key = derive_key(&check_task, "project_id");
        slot_mgr.acquire(&group_key).ok();
        let restart_count = count_task_restarts(conn, &task_id)?;

        recovered_agent_ids.insert(agent_id.clone());
        let baseline_file_path = baselines_dir.join(format!("{task_id}.json"));
        let baseline_paths = load_task_baseline(&baseline_file_path);
        recovered.push(ActiveAgent {
            process: AgentProcess::Detached {
                pid,
                proc_start_time,
            },
            task_id: task_id.clone(),
            agent_id: agent_id.clone(),
            group_key,
            config_path: agents_dir.join(format!("{agent_id}.json")),
            result_path: results_dir.join(format!("{task_id}-{agent_id}.json")),
            checkpoint_path: checkpoints_dir.join(format!("{task_id}.json")),
            heartbeat_path: heartbeats_dir.join(format!("{agent_id}.json")),
            stderr_log_path: logs_dir.join(format!("{agent_id}.stderr.log")),
            baseline_file_path,
            baseline_paths,
            started_at: parse_sqlite_datetime(&started_at_raw).unwrap_or_else(Utc::now),
            last_nudge_at: None,
            restart_count,
        });
    }

    let mut running_stmt =
        conn.prepare("SELECT agent_id FROM agent_runs WHERE status = 'running'")?;
    let running_agent_ids: Vec<String> = running_stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;

    for agent_id in running_agent_ids {
        if !recovered_agent_ids.contains(&agent_id) {
            abandoned_runs += conn.execute(
                "UPDATE agent_runs SET status = 'abandoned' WHERE agent_id = ?1 AND status = 'running'",
                params![agent_id],
            )? as u32;
        }
    }

    if recovered_claims > 0 || abandoned_runs > 0 || !recovered.is_empty() {
        println!(
            "Recovery: {} claims reset, {} agents reattached, {} stale runs abandoned.",
            recovered_claims,
            recovered.len(),
            abandoned_runs
        );
    }

    Ok(recovered)
}

fn is_expected_process_alive(pid: u32, expected_start: u64) -> bool {
    if !commander_supervisor::is_pid_alive(pid) {
        return false;
    }
    if expected_start == 0 {
        return true;
    }
    commander_supervisor::proc_start_time(pid)
        .map(|current| current == expected_start)
        .unwrap_or(false)
}

fn check_completion_contract(
    conn: &rusqlite::Connection,
    task_id: &str,
    result: &WorkerResult,
) -> Result<Option<String>> {
    let criteria_json: String = conn.query_row(
        "SELECT acceptance_criteria FROM tasks WHERE id = ?1",
        params![task_id],
        |row| row.get(0),
    )?;
    let required: Vec<String> = serde_json::from_str(&criteria_json)
        .with_context(|| format!("failed to parse acceptance_criteria JSON for task {task_id}"))?;
    if required.is_empty() {
        return Ok(None);
    }

    if result.criteria_evidence.is_empty() {
        return Ok(Some("complete_task missing criteria evidence".into()));
    }

    let missing: Vec<String> = required
        .iter()
        .filter(|required_criterion| {
            !result.criteria_evidence.iter().any(|provided| {
                criterion_matches(required_criterion, &provided.criterion, &provided.evidence)
            })
        })
        .cloned()
        .collect();

    if missing.is_empty() {
        Ok(None)
    } else {
        Ok(Some(format!(
            "missing evidence for {} criteria",
            missing.len()
        )))
    }
}

fn criterion_matches(required: &str, provided_criterion: &str, provided_evidence: &str) -> bool {
    let required_norm = normalize_for_match(required);
    if required_norm.is_empty() {
        return true;
    }

    let provided_criterion_norm = normalize_for_match(provided_criterion);
    let provided_evidence_norm = normalize_for_match(provided_evidence);
    let combined_norm = format!("{provided_criterion_norm} {provided_evidence_norm}");

    if combined_norm.contains(&required_norm)
        || (!combined_norm.is_empty() && required_norm.contains(&combined_norm))
    {
        return true;
    }

    let required_tokens = tokenize_for_match(&required_norm);
    if required_tokens.is_empty() {
        return true;
    }
    let combined_tokens = tokenize_for_match(&combined_norm);
    if combined_tokens.is_empty() {
        return false;
    }

    let overlap = required_tokens.intersection(&combined_tokens).count() as f64;
    let required_len = required_tokens.len() as f64;
    (overlap / required_len) >= 0.6
}

fn normalize_for_match(input: &str) -> String {
    input
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn tokenize_for_match(input: &str) -> HashSet<String> {
    input
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

fn count_task_attempts(conn: &rusqlite::Connection, task_id: &str) -> Result<u32> {
    let attempts: u32 = conn.query_row(
        "SELECT COUNT(*) FROM agent_runs
         WHERE task_id = ?1
           AND status NOT IN ('abandoned', 'restarted')",
        params![task_id],
        |row| row.get(0),
    )?;
    Ok(attempts)
}

fn count_task_restarts(conn: &rusqlite::Connection, task_id: &str) -> Result<u32> {
    let restarts: u32 = conn.query_row(
        "SELECT COUNT(*) FROM agent_runs WHERE task_id = ?1 AND status = 'restarted'",
        params![task_id],
        |row| row.get(0),
    )?;
    Ok(restarts)
}

fn load_or_create_task_baseline(
    working_dir: &Path,
    baseline_file_path: &Path,
) -> Result<Vec<String>> {
    if baseline_file_path.exists() {
        let raw = std::fs::read_to_string(baseline_file_path)?;
        if let Ok(paths) = serde_json::from_str::<Vec<String>>(&raw) {
            return Ok(paths);
        }
        tracing::warn!(
            path = %baseline_file_path.display(),
            "invalid baseline file, regenerating"
        );
    }

    let paths = capture_git_status_snapshot(working_dir)?;
    if let Some(parent) = baseline_file_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(baseline_file_path, serde_json::to_vec(&paths)?)?;
    Ok(paths)
}

fn load_task_baseline(baseline_file_path: &Path) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(baseline_file_path) else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<String>>(&raw) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(path = %baseline_file_path.display(), error = %e, "failed to parse baseline JSON, using empty baseline");
            Vec::new()
        }
    }
}

fn capture_git_status_snapshot(working_dir: &Path) -> Result<Vec<String>> {
    let out = std::process::Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .arg("--untracked-files=all")
        .current_dir(working_dir)
        .output()?;

    if !out.status.success() {
        anyhow::bail!(
            "git status snapshot failed (exit {}): {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(200)
                .collect::<String>()
        );
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(parse_git_status_paths(&stdout))
}

fn escalate_blocked_pending_tasks(conn: &rusqlite::Connection) -> Result<u32> {
    let mut total_escalated = 0_u32;

    loop {
        let mut stmt = conn.prepare(
            "SELECT id, depends_on
             FROM tasks
             WHERE status = 'pending'",
        )?;

        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut changed_this_pass = 0_u32;
        for (task_id, deps_json) in rows {
            let deps: Vec<String> = serde_json::from_str(&deps_json)
                .with_context(|| format!("failed to parse depends_on JSON for task {task_id}"))?;
            if deps.is_empty() {
                continue;
            }

            let blocked = deps.iter().any(|dep_id| {
                conn.query_row(
                    "SELECT status FROM tasks WHERE id = ?1",
                    params![dep_id],
                    |row| row.get::<_, String>(0),
                )
                .map(|status| matches!(status.as_str(), "escalated" | "failed"))
                .unwrap_or(true)
            });

            if blocked {
                let changed = conn.execute(
                    "UPDATE tasks
                     SET status = 'escalated', claimed_by = NULL, updated_at = datetime('now')
                     WHERE id = ?1 AND status = 'pending'",
                    params![task_id],
                )?;
                changed_this_pass += changed as u32;
            }
        }

        if changed_this_pass == 0 {
            break;
        }
        total_escalated += changed_this_pass;
    }

    Ok(total_escalated)
}

async fn run_validation(
    conn: &rusqlite::Connection,
    project_dir: &Path,
    config: &CommanderConfig,
    task_id: &str,
    baseline_paths: &[String],
) -> Result<commander_coordination::ValidationResult> {
    let (task_kind_raw, files_json): (String, String) = conn.query_row(
        "SELECT task_kind, files FROM tasks WHERE id = ?1",
        params![task_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let task_kind = parse_task_kind(task_kind_raw.as_str());
    let allowed_files: Vec<String> = serde_json::from_str(&files_json)
        .with_context(|| format!("failed to parse files JSON for task {task_id}"))?;
    let require_in_scope_changes = task_kind.requires_in_scope_changes(!allowed_files.is_empty());

    let mut pipeline = ValidationPipeline::new(config.validation.max_fix_cycles);
    pipeline.add_step(Box::new(BoundaryCheckStep));
    pipeline.add_step(Box::new(RunTestsStep));

    let context = ValidationContext {
        working_dir: project_dir.to_path_buf(),
        allowed_files,
        require_in_scope_changes,
        baseline_paths: baseline_paths.to_vec(),
        ignored_prefixes: vec![".commander/".into()],
        test_command: config.validation.test_command.clone(),
    };

    Ok(pipeline.run(task_id, &context).await)
}

fn parse_task_kind(raw: &str) -> TaskKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "implement" => TaskKind::Implement,
        "explore" => TaskKind::Explore,
        _ => TaskKind::Implement,
    }
}

fn summarize_validation_failures(validation: &commander_coordination::ValidationResult) -> String {
    if validation.issues.is_empty() {
        return "validation failed".into();
    }

    let first = &validation.issues[0].description;
    if validation.issues.len() == 1 {
        first.clone()
    } else {
        format!("{first} (+{} more issues)", validation.issues.len() - 1)
    }
}

fn query_runnable_tasks(conn: &rusqlite::Connection) -> Result<Vec<RunnableTask>> {
    let mut stmt = conn.prepare(
        "SELECT id, project_id, title, description, task_kind, acceptance_criteria, files, depends_on
         FROM tasks
         WHERE status = 'pending'
         ORDER BY
         CASE priority WHEN 'P0' THEN 0 WHEN 'P1' THEN 1 WHEN 'P2' THEN 2 ELSE 3 END",
    )?;

    let rows: Vec<(
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
    )> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut runnable = Vec::new();
    for (id, project_id, title, description, task_kind, criteria_json, files_json, deps_json) in
        rows
    {
        let deps: Vec<String> = serde_json::from_str(&deps_json)
            .with_context(|| format!("failed to parse depends_on JSON for task {id}"))?;
        let acceptance_criteria: Vec<String> = serde_json::from_str(&criteria_json)
            .with_context(|| format!("failed to parse acceptance_criteria JSON for task {id}"))?;
        let files: Vec<String> = serde_json::from_str(&files_json)
            .with_context(|| format!("failed to parse files JSON for task {id}"))?;
        if deps.is_empty() {
            runnable.push(RunnableTask {
                id,
                project_id,
                title,
                description,
                kind: parse_task_kind(&task_kind),
                acceptance_criteria,
                files,
            });
            continue;
        }
        let all_complete = deps.iter().all(|dep_id| {
            conn.query_row(
                "SELECT status FROM tasks WHERE id = ?1",
                params![dep_id],
                |row| row.get::<_, String>(0),
            )
            .map(|s| s == "complete")
            .unwrap_or(false)
        });
        if all_complete {
            runnable.push(RunnableTask {
                id,
                project_id,
                title,
                description,
                kind: parse_task_kind(&task_kind),
                acceptance_criteria,
                files,
            });
        }
    }

    Ok(runnable)
}

/// Resets tasks that have been stuck in `claimed` state for longer than `ttl_secs` seconds
/// but have no corresponding active `agent_runs` row. This covers the window between a task
/// being claimed and the supervisor successfully inserting its `agent_runs` entry (CRIT-002).
fn reclaim_orphaned_claims(conn: &rusqlite::Connection, ttl_secs: u64) -> Result<u32> {
    let threshold = Utc::now() - chrono::Duration::seconds(ttl_secs as i64);
    let threshold_str = threshold.format("%Y-%m-%d %H:%M:%S").to_string();
    let count = conn.execute(
        "UPDATE tasks
         SET status = 'pending', claimed_by = NULL, updated_at = datetime('now')
         WHERE status = 'claimed'
           AND updated_at < ?1
           AND (
             claimed_by IS NULL
             OR NOT EXISTS (
               SELECT 1 FROM agent_runs
               WHERE agent_id = tasks.claimed_by
                 AND status = 'running'
             )
           )",
        params![threshold_str],
    )?;
    Ok(count as u32)
}

fn count_active_tasks(conn: &rusqlite::Connection) -> Result<u32> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status NOT IN ('complete', 'failed', 'escalated')",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_token_wakes_sleeping_select() {
        // Verify that cancelling the token immediately unblocks a long sleep via select!,
        // which is the mechanism used by the graceful-shutdown path in run().
        let token = CancellationToken::new();
        let token_clone = token.clone();
        tokio::spawn(async move {
            token_clone.cancel();
        });
        let start = std::time::Instant::now();
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(3600)) => panic!("should not sleep"),
            _ = token.cancelled() => {},
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn parse_task_kind_defaults_to_implement_for_unknown_values() {
        assert_eq!(parse_task_kind("invalid_kind"), TaskKind::Implement);
        assert_eq!(parse_task_kind(""), TaskKind::Implement);
    }

    #[test]
    fn task_kind_drives_change_requirement() {
        assert!(TaskKind::Implement.requires_in_scope_changes(true));
        assert!(!TaskKind::Implement.requires_in_scope_changes(false));
        assert!(!TaskKind::Explore.requires_in_scope_changes(true));
    }

    #[test]
    fn derive_task_shell_cwd_prefers_manifested_subproject_dir() {
        let project = std::env::temp_dir().join(format!(
            "commander-task-cwd-test-{}",
            uuid::Uuid::new_v4().as_simple()
        ));
        std::fs::create_dir_all(project.join("webapp/src")).unwrap();
        std::fs::write(project.join("webapp/package.json"), "{}").unwrap();
        std::fs::write(project.join("package.json"), "{}").unwrap();

        let cwd = derive_task_shell_cwd(
            &project,
            &[
                "webapp/src/App.jsx".into(),
                "webapp/src/App.css".into(),
                "webapp/README.md".into(),
            ],
        );
        assert_eq!(cwd, project.join("webapp"));
        let _ = std::fs::remove_dir_all(project);
    }

    #[test]
    fn derive_task_shell_cwd_keeps_repo_root_for_src_scopes() {
        let project = std::env::temp_dir().join(format!(
            "commander-task-cwd-src-test-{}",
            uuid::Uuid::new_v4().as_simple()
        ));
        std::fs::create_dir_all(project.join("src/components/editor")).unwrap();
        std::fs::write(project.join("package.json"), "{}").unwrap();

        let cwd = derive_task_shell_cwd(
            &project,
            &[
                "src/components/editor/**".into(),
                "src/__tests__/editor/**".into(),
            ],
        );
        assert_eq!(cwd, project);
        let _ = std::fs::remove_dir_all(project);
    }

    fn open_test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn query_runnable_tasks_rejects_corrupt_files_json() {
        let conn = open_test_db();
        // Insert a task with corrupted files JSON — simulates DB corruption or injection
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, description, task_kind, acceptance_criteria, files, depends_on, status)
             VALUES ('t-corrupt', 'proj', 'Bad Task', '', 'implement', '[]', 'NOT_VALID_JSON', '[]', 'pending')",
            [],
        )
        .unwrap();

        let result = query_runnable_tasks(&conn);
        assert!(result.is_err(), "corrupt files JSON must return an error, not silently bypass file restrictions");
        let msg = format!("{}", result.err().unwrap());
        assert!(msg.contains("files"), "error should mention the 'files' field");
    }

    #[test]
    fn query_runnable_tasks_rejects_corrupt_acceptance_criteria_json() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, description, task_kind, acceptance_criteria, files, depends_on, status)
             VALUES ('t-crit', 'proj', 'Bad Criteria', '', 'implement', 'NOT_JSON', '[]', '[]', 'pending')",
            [],
        )
        .unwrap();

        let result = query_runnable_tasks(&conn);
        assert!(result.is_err());
    }

    #[test]
    fn check_completion_contract_rejects_corrupt_criteria_json() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, description, acceptance_criteria, status)
             VALUES ('t-cc', 'proj', 'Task', '', 'NOT_JSON', 'pending')",
            [],
        )
        .unwrap();

        let dummy_result = WorkerResult {
            task_id: "t-cc".into(),
            agent_id: "agent-1".into(),
            attempt_id: "attempt-1".into(),
            status: "complete".into(),
            summary: "done".into(),
            criteria_evidence: vec![],
            issues: vec![],
        };
        let result = check_completion_contract(&conn, "t-cc", &dummy_result);
        assert!(result.is_err(), "corrupt acceptance_criteria JSON must return an error");
    }

    #[test]
    fn reclaim_orphaned_claims_resets_stuck_task() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, status, claimed_by, updated_at)
             VALUES ('t1', 'proj', 'Test', 'claimed', 'agent-old', datetime('now', '-120 seconds'))",
            [],
        )
        .unwrap();

        let count = reclaim_orphaned_claims(&conn, 60).unwrap();
        assert_eq!(count, 1);

        let status: String = conn
            .query_row("SELECT status FROM tasks WHERE id = 't1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "pending");
    }

    #[test]
    fn reclaim_orphaned_claims_leaves_recently_claimed_task() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, status, claimed_by, updated_at)
             VALUES ('t2', 'proj', 'Test', 'claimed', 'agent-new', datetime('now', '-5 seconds'))",
            [],
        )
        .unwrap();

        let count = reclaim_orphaned_claims(&conn, 60).unwrap();
        assert_eq!(count, 0);

        let status: String = conn
            .query_row("SELECT status FROM tasks WHERE id = 't2'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "claimed");
    }

    // --- HIGH-001 tests: supervisor helper functions ---

    #[test]
    fn disposition_from_exit_complete_result() {
        let dir = std::env::temp_dir().join(format!("disp-test-{}", uuid::Uuid::new_v4().as_simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let result_path = dir.join("result.json");
        let stderr_path = dir.join("stderr.log");

        let result = WorkerResult {
            task_id: "t1".into(),
            agent_id: "a1".into(),
            attempt_id: "att-1".into(),
            status: "complete".into(),
            summary: "done".into(),
            criteria_evidence: vec![],
            issues: vec![],
        };
        std::fs::write(&result_path, serde_json::to_string(&result).unwrap()).unwrap();

        match disposition_from_exit(0, &result_path, &stderr_path) {
            Disposition::CandidateComplete(r) => assert_eq!(r.summary, "done"),
            Disposition::Failed(reason) => panic!("expected CandidateComplete, got Failed({reason})"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disposition_from_exit_nonzero_with_stderr() {
        let dir = std::env::temp_dir().join(format!("disp-err-{}", uuid::Uuid::new_v4().as_simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let result_path = dir.join("result.json");
        let stderr_path = dir.join("stderr.log");
        std::fs::write(&stderr_path, "thread panicked at 'index out of bounds'").unwrap();

        match disposition_from_exit(1, &result_path, &stderr_path) {
            Disposition::Failed(reason) => {
                assert!(reason.contains("exit code 1"), "reason={reason}");
                assert!(reason.contains("panicked"), "reason={reason}");
            }
            Disposition::CandidateComplete(_) => panic!("expected Failed for exit code 1"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disposition_from_exit_zero_missing_result_file() {
        let dir = std::env::temp_dir().join(format!("disp-miss-{}", uuid::Uuid::new_v4().as_simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let result_path = dir.join("result.json"); // does not exist
        let stderr_path = dir.join("stderr.log");

        match disposition_from_exit(0, &result_path, &stderr_path) {
            Disposition::Failed(reason) => {
                assert!(reason.contains("missing") || reason.contains("invalid"), "reason={reason}");
            }
            Disposition::CandidateComplete(_) => panic!("expected Failed for missing result"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disposition_from_exit_zero_failed_status() {
        let dir = std::env::temp_dir().join(format!("disp-fail-{}", uuid::Uuid::new_v4().as_simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let result_path = dir.join("result.json");
        let stderr_path = dir.join("stderr.log");

        let result = WorkerResult {
            task_id: "t1".into(),
            agent_id: "a1".into(),
            attempt_id: "att-1".into(),
            status: "failed".into(),
            summary: "LLM adapter error".into(),
            criteria_evidence: vec![],
            issues: vec![],
        };
        std::fs::write(&result_path, serde_json::to_string(&result).unwrap()).unwrap();

        match disposition_from_exit(0, &result_path, &stderr_path) {
            Disposition::Failed(reason) => assert_eq!(reason, "LLM adapter error"),
            Disposition::CandidateComplete(_) => panic!("expected Failed for failed status"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn escalate_blocked_pending_tasks_cascades_through_deps() {
        let conn = open_test_db();
        // Task A is escalated
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, status, depends_on)
             VALUES ('t-a', 'proj', 'Task A', 'escalated', '[]')",
            [],
        ).unwrap();
        // Task B depends on A -> should be escalated
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, status, depends_on)
             VALUES ('t-b', 'proj', 'Task B', 'pending', '[\"t-a\"]')",
            [],
        ).unwrap();
        // Task C depends on B -> should cascade-escalate
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, status, depends_on)
             VALUES ('t-c', 'proj', 'Task C', 'pending', '[\"t-b\"]')",
            [],
        ).unwrap();
        // Task D has no deps -> should stay pending
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, status, depends_on)
             VALUES ('t-d', 'proj', 'Task D', 'pending', '[]')",
            [],
        ).unwrap();

        let escalated = escalate_blocked_pending_tasks(&conn).unwrap();
        assert_eq!(escalated, 2, "B and C should both be escalated");

        let status_b: String = conn
            .query_row("SELECT status FROM tasks WHERE id = 't-b'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status_b, "escalated");

        let status_c: String = conn
            .query_row("SELECT status FROM tasks WHERE id = 't-c'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status_c, "escalated");

        let status_d: String = conn
            .query_row("SELECT status FROM tasks WHERE id = 't-d'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status_d, "pending");
    }

    #[test]
    fn query_runnable_tasks_filters_by_dependency_status() {
        let conn = open_test_db();
        // Dep task: complete
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, status, depends_on, acceptance_criteria, files, task_kind)
             VALUES ('dep-done', 'proj', 'Dep Done', 'complete', '[]', '[]', '[]', 'implement')",
            [],
        ).unwrap();
        // Dep task: pending
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, status, depends_on, acceptance_criteria, files, task_kind)
             VALUES ('dep-pending', 'proj', 'Dep Pending', 'pending', '[]', '[]', '[]', 'implement')",
            [],
        ).unwrap();
        // Task A: depends on complete dep -> runnable
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, status, depends_on, acceptance_criteria, files, task_kind)
             VALUES ('t-runnable', 'proj', 'Runnable', 'pending', '[\"dep-done\"]', '[]', '[]', 'implement')",
            [],
        ).unwrap();
        // Task B: depends on pending dep -> NOT runnable
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, status, depends_on, acceptance_criteria, files, task_kind)
             VALUES ('t-blocked', 'proj', 'Blocked', 'pending', '[\"dep-pending\"]', '[]', '[]', 'implement')",
            [],
        ).unwrap();

        let runnable = query_runnable_tasks(&conn).unwrap();
        let ids: Vec<&str> = runnable.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&"dep-pending"), "dep-pending has no deps so it's runnable");
        assert!(ids.contains(&"t-runnable"), "t-runnable's dep is complete");
        assert!(!ids.contains(&"t-blocked"), "t-blocked's dep is still pending");
    }

    #[test]
    fn criterion_matches_exact_substring() {
        assert!(criterion_matches(
            "all tests pass",
            "all tests pass",
            "ran cargo test, 100% pass"
        ));
    }

    #[test]
    fn criterion_matches_fuzzy_token_overlap() {
        // 60% token overlap threshold: 4/5 required tokens present = 80%
        assert!(criterion_matches(
            "error handling for file operations",
            "error handling in file",
            "added operations for safety"
        ));
    }

    #[test]
    fn criterion_matches_rejects_unrelated_evidence() {
        assert!(!criterion_matches(
            "add database migration",
            "fixed CSS styling",
            "adjusted padding and margin values"
        ));
    }

    #[test]
    fn format_stale_ms_formats_seconds_and_minutes() {
        assert_eq!(format_stale_ms(5000), "5s");
        assert_eq!(format_stale_ms(90000), "1m30s");
        assert_eq!(format_stale_ms(0), "0s");
    }

    #[test]
    fn count_task_attempts_excludes_abandoned_and_restarted() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, status)
             VALUES ('t-att', 'proj', 'Task', 'pending')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO agent_runs (agent_id, task_id, pid, proc_start_time, status)
             VALUES ('a1', 't-att', 1, 0, 'succeeded')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO agent_runs (agent_id, task_id, pid, proc_start_time, status)
             VALUES ('a2', 't-att', 2, 0, 'abandoned')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO agent_runs (agent_id, task_id, pid, proc_start_time, status)
             VALUES ('a3', 't-att', 3, 0, 'restarted')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO agent_runs (agent_id, task_id, pid, proc_start_time, status)
             VALUES ('a4', 't-att', 4, 0, 'failed')",
            [],
        ).unwrap();

        let count = count_task_attempts(&conn, "t-att").unwrap();
        assert_eq!(count, 2, "only succeeded and failed count as attempts");
    }

    #[test]
    fn reclaim_orphaned_claims_leaves_task_with_active_agent_run() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO tasks (id, project_id, title, status, claimed_by, updated_at)
             VALUES ('t3', 'proj', 'Test', 'claimed', 'agent-active', datetime('now', '-120 seconds'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO agent_runs (agent_id, task_id, pid, proc_start_time, status)
             VALUES ('agent-active', 't3', 12345, 0, 'running')",
            [],
        )
        .unwrap();

        let count = reclaim_orphaned_claims(&conn, 60).unwrap();
        assert_eq!(count, 0);

        let status: String = conn
            .query_row("SELECT status FROM tasks WHERE id = 't3'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "claimed");
    }
}
