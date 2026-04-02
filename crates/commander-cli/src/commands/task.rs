use anyhow::Result;
use commander_tasks::task::TaskKind;
use rusqlite::params;
use std::path::Path;

pub fn add(
    project_dir: &Path,
    title: &str,
    priority: &str,
    depends_on: &[String],
    acceptance_criteria: &[String],
    files: &[String],
    kind: TaskKind,
) -> Result<()> {
    let config = crate::config::load_config(project_dir)?;
    let db_path = crate::config::db_path(project_dir);
    let conn = crate::db::open_db(&db_path)?;

    let deps_json = serde_json::to_string(depends_on)?;
    let criteria_json = serde_json::to_string(acceptance_criteria)?;
    let files_json = serde_json::to_string(files)?;
    let id = insert_task_with_retry(
        &conn,
        &config.project.name,
        title,
        priority,
        &deps_json,
        &criteria_json,
        &files_json,
        kind,
    )?;

    println!("{id}: {title} [{priority}/{kind}]", kind = kind.as_str());
    Ok(())
}

pub fn list(project_dir: &Path) -> Result<()> {
    let db_path = crate::config::db_path(project_dir);
    let conn = crate::db::open_db_readonly(&db_path)?;

    let mut stmt = conn.prepare(
        "SELECT id, title, priority, task_kind, status, claimed_by FROM tasks ORDER BY
         CASE priority WHEN 'P0' THEN 0 WHEN 'P1' THEN 1 WHEN 'P2' THEN 2 ELSE 3 END,
         created_at ASC",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })?;

    let mut count = 0;
    for row in rows {
        let (id, title, priority, task_kind, status, claimed_by) = row?;
        let agent = claimed_by.map(|a| format!(" ({})", a)).unwrap_or_default();
        println!("{id}  [{priority}/{task_kind}]  {status:<10}  {title}{agent}");
        count += 1;
    }

    if count == 0 {
        println!("No tasks. Add one with: commander task add \"description\"");
    }
    Ok(())
}

pub fn status(project_dir: &Path, task_id: &str) -> Result<()> {
    let db_path = crate::config::db_path(project_dir);
    let conn = crate::db::open_db_readonly(&db_path)?;

    let result = conn.query_row(
        "SELECT id, title, description, priority, task_kind, status, claimed_by, depends_on, acceptance_criteria FROM tasks WHERE id = ?1",
        params![task_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        },
    );

    match result {
        Ok((id, title, desc, priority, task_kind, status, claimed_by, deps, criteria)) => {
            println!("Task:     {id}");
            println!("Title:    {title}");
            println!("Priority: {priority}");
            println!("Kind:     {task_kind}");
            println!("Status:   {status}");
            if let Some(agent) = claimed_by {
                println!("Agent:    {agent}");
            }
            if !desc.is_empty() {
                println!("Desc:     {desc}");
            }
            if deps != "[]" {
                println!("Deps:     {deps}");
            }
            if criteria != "[]" {
                println!("Criteria: {criteria}");
            }
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            println!("Task {task_id} not found");
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
}

fn next_task_number(conn: &rusqlite::Connection) -> Result<u32> {
    let max_num: u32 = conn.query_row(
        "SELECT COALESCE(MAX(CAST(SUBSTR(id, 6) AS INTEGER)), 0)
         FROM tasks
         WHERE id LIKE 'TASK-%'",
        [],
        |row| row.get(0),
    )?;
    Ok(max_num + 1)
}

fn insert_task_with_retry(
    conn: &rusqlite::Connection,
    project_name: &str,
    title: &str,
    priority: &str,
    deps_json: &str,
    criteria_json: &str,
    files_json: &str,
    kind: TaskKind,
) -> Result<String> {
    let mut next_num = next_task_number(conn)?;
    for _ in 0..32 {
        let id = format!("TASK-{next_num:03}");
        match conn.execute(
            "INSERT INTO tasks
             (id, project_id, title, priority, task_kind, depends_on, acceptance_criteria, files, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending')",
            params![
                id,
                project_name,
                title,
                priority,
                kind.as_str(),
                deps_json,
                criteria_json,
                files_json
            ],
        ) {
            Ok(_) => return Ok(id),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                next_num += 1;
            }
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::bail!("failed to allocate unique task id after multiple attempts")
}
