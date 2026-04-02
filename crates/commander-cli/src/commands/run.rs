use anyhow::Result;
use commander_concurrency::policy::ConcurrencyPolicy;
use commander_concurrency::slot::SlotManager;
use commander_concurrency::key::derive_key;
use commander_supervisor::singleton::SupervisorLock;
use commander_supervisor::spawner::{MockSpawner, ProcessSpawner};
use rusqlite::params;
use std::collections::HashMap;
use std::path::Path;

pub async fn run(project_dir: &Path) -> Result<()> {
    let config = crate::config::load_config(project_dir)?;

    // 1. Acquire singleton lock
    let lock_path = crate::config::lock_path(project_dir);
    let _lock = SupervisorLock::acquire(&lock_path)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
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

    // 4. Set up spawner
    // TODO: OsProcessSpawner for real agents
    let spawner = MockSpawner::new();

    // 5. Track active task_id -> group_key
    let mut active_keys: HashMap<String, String> = HashMap::new();

    let tick_interval = std::time::Duration::from_millis(config.supervisor.tick_interval_ms);

    println!("Commander supervisor running (max_agents={}, tick={}ms)",
        config.supervisor.max_agents, config.supervisor.tick_interval_ms);
    println!("Press Ctrl+C to stop.\n");

    // 6. Management loop
    loop {
        // Query for pending tasks with deps satisfied
        let runnable = query_runnable_tasks(&conn)?;

        if !runnable.is_empty() {
            tracing::debug!(count = runnable.len(), "runnable tasks found");
        }

        for (task_id, project_id, title) in &runnable {
            // Build a minimal Task for slot checking
            let check_task = commander_tasks::task::Task::new(task_id, project_id, title);
            if !slot_mgr.can_run(&check_task) {
                continue;
            }

            let group_key = derive_key(&check_task, "project_id");
            let agent_id = format!("agent-{}", uuid::Uuid::new_v4().as_simple());

            // Claim: pending -> claimed
            let changed = conn.execute(
                "UPDATE tasks SET status = 'claimed', claimed_by = ?1, updated_at = datetime('now')
                 WHERE id = ?2 AND status = 'pending'",
                params![agent_id, task_id],
            )?;

            if changed == 0 {
                continue; // already claimed by another tick
            }

            tracing::info!(task = task_id, agent = agent_id, "claimed task");

            // Spawn agent
            let config_path = crate::config::commander_dir(project_dir)
                .join(format!("{agent_id}.json"));

            // Write agent config for the worker process
            let agent_config = serde_json::json!({
                "task_id": task_id,
                "title": title,
                "project_id": project_id,
                "model": crate::config::load_config(project_dir)?.runtime.default_model,
                "cwd": project_dir.canonicalize()?.to_string_lossy(),
            });
            std::fs::write(&config_path, serde_json::to_string_pretty(&agent_config)?)?;

            match spawner.spawn(&agent_id, task_id, &config_path).await {
                Ok(handle) => {
                    slot_mgr.acquire(&group_key).ok();
                    active_keys.insert(task_id.clone(), group_key);

                    // Record agent run
                    conn.execute(
                        "INSERT OR REPLACE INTO agent_runs (agent_id, task_id, pid, started_at, status)
                         VALUES (?1, ?2, ?3, datetime('now'), 'running')",
                        params![agent_id, task_id, handle.pid],
                    )?;

                    println!("  ▸ spawned {agent_id} for {task_id}: {title}");
                }
                Err(e) => {
                    tracing::error!(task = task_id, "spawn failed: {e}");
                    // Unclaim — revert to pending
                    let unclaimed = conn.execute(
                        "UPDATE tasks SET status = 'pending', claimed_by = NULL, updated_at = datetime('now')
                         WHERE id = ?1 AND status = 'claimed'",
                        params![task_id],
                    )?;
                    if unclaimed == 0 {
                        // unclaim failed too — escalate
                        tracing::error!(task = task_id, "unclaim failed after spawn failure, escalating");
                        conn.execute(
                            "UPDATE tasks SET status = 'escalated', updated_at = datetime('now') WHERE id = ?1",
                            params![task_id],
                        )?;
                    }
                    // Clean up config file
                    let _ = std::fs::remove_file(&config_path);
                }
            }
        }

        // For the prototype, auto-complete mock-spawned tasks after a tick
        // (real agents would be polled via supervisor.poll_completed())
        auto_complete_mock_tasks(&conn, &mut active_keys, &mut slot_mgr)?;

        // Check if all tasks are terminal (complete/failed/escalated)
        let remaining = count_active_tasks(&conn)?;
        if remaining == 0 && !runnable.is_empty() {
            // We just finished the last batch
        }
        if remaining == 0 {
            let total: u32 = conn.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))?;
            if total > 0 {
                println!("\nAll {total} tasks complete.");
                break;
            }
        }

        tokio::time::sleep(tick_interval).await;
    }

    Ok(())
}

fn query_runnable_tasks(conn: &rusqlite::Connection) -> Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, project_id, title, depends_on FROM tasks WHERE status = 'pending' ORDER BY
         CASE priority WHEN 'P0' THEN 0 WHEN 'P1' THEN 1 WHEN 'P2' THEN 2 ELSE 3 END"
    )?;

    let rows: Vec<(String, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut runnable = Vec::new();
    for (id, project_id, title, deps_json) in rows {
        let deps: Vec<String> = serde_json::from_str(&deps_json).unwrap_or_default();
        if deps.is_empty() {
            runnable.push((id, project_id, title));
            continue;
        }
        // Check all deps are complete
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
            runnable.push((id, project_id, title));
        }
    }

    Ok(runnable)
}

/// Prototype: auto-complete claimed tasks (since MockSpawner doesn't run real agents)
fn auto_complete_mock_tasks(
    conn: &rusqlite::Connection,
    active_keys: &mut HashMap<String, String>,
    slot_mgr: &mut SlotManager,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id FROM tasks WHERE status = 'claimed'"
    )?;
    let claimed: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;

    for task_id in claimed {
        conn.execute(
            "UPDATE tasks SET status = 'complete', updated_at = datetime('now') WHERE id = ?1",
            params![task_id],
        )?;
        if let Some(group_key) = active_keys.remove(&task_id) {
            slot_mgr.release(&group_key);
        }
        println!("  ✓ completed {task_id}");
    }

    Ok(())
}

fn count_active_tasks(conn: &rusqlite::Connection) -> Result<u32> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status NOT IN ('complete', 'failed', 'escalated')",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}
