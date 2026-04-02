mod commands;
mod config;
mod db;

use clap::{Parser, Subcommand};
use commander_tasks::task::TaskKind;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "commander", version, about = "AI agent orchestration kernel")]
struct Cli {
    /// Project directory (default: current directory)
    #[arg(long, default_value = ".")]
    dir: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a commander project
    Init,

    /// Manage tasks
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },

    /// Start the supervisor and run the management loop
    Run,

    /// Show project status overview
    Status,

    /// Run as an agent worker (invoked by supervisor, not user-facing)
    AgentWorker {
        /// Agent ID
        #[arg(long)]
        id: String,
        /// Path to agent config JSON
        #[arg(long)]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
enum TaskAction {
    /// Add a new task
    Add {
        /// Task title/description
        title: String,
        /// Priority (P0, P1, P2, P3)
        #[arg(short, long, default_value = "P2")]
        priority: String,
        /// Task IDs this depends on
        #[arg(short, long)]
        depends_on: Vec<String>,
        /// Acceptance criterion (repeat for multiple)
        #[arg(long = "criteria")]
        acceptance_criteria: Vec<String>,
        /// Allowed file pattern (glob, repeat for multiple)
        #[arg(long = "file")]
        files: Vec<String>,
        /// Task kind: implementation work or exploration/reporting only
        #[arg(long, value_enum, default_value_t = TaskKindArg::Implement)]
        kind: TaskKindArg,
    },
    /// List all tasks
    List,
    /// Show task details
    Status {
        /// Task ID
        id: String,
    },
}

#[derive(clap::ValueEnum, Clone, Copy)]
enum TaskKindArg {
    Implement,
    Explore,
}

impl From<TaskKindArg> for TaskKind {
    fn from(value: TaskKindArg) -> Self {
        match value {
            TaskKindArg::Implement => TaskKind::Implement,
            TaskKindArg::Explore => TaskKind::Explore,
        }
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    let project_dir = cli.dir.canonicalize().unwrap_or(cli.dir);

    match cli.command {
        Commands::Init => commands::init::run(&project_dir),
        Commands::Task { action } => match action {
            TaskAction::Add {
                title,
                priority,
                depends_on,
                acceptance_criteria,
                files,
                kind,
            } => commands::task::add(
                &project_dir,
                &title,
                &priority,
                &depends_on,
                &acceptance_criteria,
                &files,
                kind.into(),
            ),
            TaskAction::List => commands::task::list(&project_dir),
            TaskAction::Status { id } => commands::task::status(&project_dir, &id),
        },
        Commands::Run => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(commands::run::run(&project_dir))
        }
        Commands::Status => commands::status::run(&project_dir),
        Commands::AgentWorker { id, config } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(commands::agent_worker::run(&id, &config))
        }
    }
}
