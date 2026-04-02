use anyhow::Result;
use rusqlite::params;
use std::path::Path;

pub fn add(project_dir: &Path, title: &str, priority: &str, depends_on: &[String]) -> Result<()> {
    let config = crate::config::load_config(project_dir)?;
    let db_path = crate::config::db_path(project_dir);
    let conn = crate::db::open_db(&db_path)?;

    let id = format!("TASK-{:03}", next_task_number(&conn)?);
    let deps_json = serde_json::to_string(depends_on)?;

    conn.execute(
        "INSERT INTO tasks (id, project_id, title, priority, depends_on, status) VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
        params![id, config.project.name, title, priority, deps_json],
    )?;

    println!("{id}: {title} [{priority}]");
    Ok(())
}

pub fn list(project_dir: &Path) -> Result<()> {
    let db_path = crate::config::db_path(project_dir);
    let conn = crate::db::open_db_readonly(&db_path)?;

    let mut stmt = conn.prepare(
        "SELECT id, title, priority, status, claimed_by FROM tasks ORDER BY
         CASE priority WHEN 'P0' THEN 0 WHEN 'P1' THEN 1 WHEN 'P2' THEN 2 ELSE 3 END,
         created_at ASC",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;

    let mut count = 0;
    for row in rows {
        let (id, title, priority, status, claimed_by) = row?;
        let agent = claimed_by
            .map(|a| format!(" ({})", a))
            .unwrap_or_default();
        println!("{id}  [{priority}]  {status:<10}  {title}{agent}");
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
        "SELECT id, title, description, priority, status, claimed_by, depends_on, acceptance_criteria FROM tasks WHERE id = ?1",
        params![task_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        },
    );

    match result {
        Ok((id, title, desc, priority, status, claimed_by, deps, criteria)) => {
            println!("Task:     {id}");
            println!("Title:    {title}");
            println!("Priority: {priority}");
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
    let count: u32 = conn.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))?;
    Ok(count + 1)
}
