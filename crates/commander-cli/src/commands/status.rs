use anyhow::Result;
use std::path::Path;

pub fn run(project_dir: &Path) -> Result<()> {
    let config = crate::config::load_config(project_dir)?;
    let db_path = crate::config::db_path(project_dir);
    let conn = crate::db::open_db_readonly(&db_path)?;

    println!("Commander: {}", config.project.name);
    println!();

    // Task counts by status
    let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM tasks GROUP BY status")?;
    let counts: Vec<(String, u32)> = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    let total: u32 = counts.iter().map(|(_, c)| c).sum();
    println!("Tasks: {total}");
    for (status, count) in &counts {
        println!("  {status:<12} {count}");
    }

    // Running agents
    let running: u32 = conn.query_row(
        "SELECT COUNT(*) FROM agent_runs WHERE status = 'running'",
        [],
        |row| row.get(0),
    )?;
    println!("\nAgents running: {running}/{}", config.supervisor.max_agents);

    Ok(())
}
