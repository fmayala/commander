use crate::config::commander_dir;
use crate::db;
use anyhow::Result;
use std::path::Path;

pub fn run(project_dir: &Path) -> Result<()> {
    let cmd_dir = commander_dir(project_dir);
    std::fs::create_dir_all(&cmd_dir)?;

    // Create default commander.toml if it doesn't exist
    let config_path = project_dir.join("commander.toml");
    if !config_path.exists() {
        let project_name = project_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project");

        let default_config = format!(
            r#"[project]
name = "{project_name}"

[runtime]
provider = "anthropic"
default_model = "claude-sonnet-4-6"
max_output_tokens = 16384

[supervisor]
max_agents = 5
tick_interval_ms = 2000
nudge_after_ms = 120000
restart_after_ms = 300000
max_restarts = 2

[validation]
# test_command = "cargo test"
max_fix_cycles = 3
"#
        );
        std::fs::write(&config_path, default_config)?;
        println!("Created commander.toml");
    }

    // Create profiles directory
    let profiles_dir = cmd_dir.join("profiles");
    std::fs::create_dir_all(&profiles_dir)?;

    // Write default worker profile
    let default_profile = profiles_dir.join("default-worker.md");
    if !default_profile.exists() {
        std::fs::write(
            &default_profile,
            r#"---
name: "default-worker"
model: "claude-sonnet-4-6"
permission_mode: "auto"
max_turns: 50
timeout: "30m"
---

You are a software engineer. Complete the assigned task by reading relevant files,
making changes, and verifying your work. When done, call the `complete_task` tool
with evidence for each acceptance criterion.
"#,
        )?;
        println!("Created default worker profile");
    }

    // Initialize SQLite
    let db_path = crate::config::db_path(project_dir);
    let conn = db::open_db(&db_path)?;
    db::init_schema(&conn)?;
    println!("Initialized database at {}", db_path.display());

    println!("\nCommander initialized in {}", cmd_dir.display());
    Ok(())
}
