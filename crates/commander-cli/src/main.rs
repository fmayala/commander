mod commands;
mod config;
mod db;

use clap::{Parser, Subcommand};
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
    },
    /// List all tasks
    List,
    /// Show task details
    Status {
        /// Task ID
        id: String,
    },
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
            } => commands::task::add(&project_dir, &title, &priority, &depends_on),
            TaskAction::List => commands::task::list(&project_dir),
            TaskAction::Status { id } => commands::task::status(&project_dir, &id),
        },
        Commands::Run => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(commands::run::run(&project_dir))
        }
        Commands::Status => commands::status::run(&project_dir),
    }
}
