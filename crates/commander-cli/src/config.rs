use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct CommanderConfig {
    pub project: ProjectConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub supervisor: SupervisorConfig,
    #[serde(default)]
    pub validation: ValidationConfig,
}

#[derive(Debug, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    #[serde(default = "default_root")]
    pub root: String,
}

fn default_root() -> String {
    ".".into()
}

#[derive(Debug, Deserialize)]
pub struct RuntimeConfig {
    #[serde(default = "default_provider")]
    provider: String,
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default = "default_max_tokens")]
    pub max_output_tokens: u32,
}

impl RuntimeConfig {
    pub fn provider(&self) -> &str {
        &self.provider
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            default_model: default_model(),
            max_output_tokens: default_max_tokens(),
        }
    }
}

fn default_provider() -> String {
    "anthropic".into()
}

fn default_model() -> String {
    "claude-sonnet-4-6".into()
}

fn default_max_tokens() -> u32 {
    16384
}

#[derive(Debug, Deserialize)]
pub struct SupervisorConfig {
    #[serde(default = "default_max_agents")]
    pub max_agents: u32,
    #[serde(default = "default_tick_interval_ms")]
    pub tick_interval_ms: u64,
    #[serde(default = "default_nudge_after_ms")]
    pub nudge_after_ms: u64,
    #[serde(default = "default_restart_after_ms")]
    pub restart_after_ms: u64,
    #[serde(default = "default_max_restarts")]
    pub max_restarts: u32,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            max_agents: default_max_agents(),
            tick_interval_ms: default_tick_interval_ms(),
            nudge_after_ms: default_nudge_after_ms(),
            restart_after_ms: default_restart_after_ms(),
            max_restarts: default_max_restarts(),
        }
    }
}

fn default_max_agents() -> u32 {
    5
}

fn default_tick_interval_ms() -> u64 {
    2000
}

fn default_nudge_after_ms() -> u64 {
    120_000
}

fn default_restart_after_ms() -> u64 {
    300_000
}

fn default_max_restarts() -> u32 {
    2
}

#[derive(Debug, Deserialize)]
pub struct ValidationConfig {
    #[serde(default = "default_test_command")]
    pub test_command: Option<String>,
    #[serde(default = "default_max_fix_cycles")]
    pub max_fix_cycles: u32,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            test_command: default_test_command(),
            max_fix_cycles: default_max_fix_cycles(),
        }
    }
}

fn default_test_command() -> Option<String> {
    None
}

fn default_max_fix_cycles() -> u32 {
    3
}

pub fn load_config(project_dir: &Path) -> Result<CommanderConfig> {
    let config_path = project_dir.join("commander.toml");
    if !config_path.exists() {
        anyhow::bail!(
            "commander.toml not found in {}. Run `commander init` first.",
            project_dir.display()
        );
    }
    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    let config: CommanderConfig =
        toml::from_str(&content).with_context(|| "parsing commander.toml")?;
    Ok(config)
}

pub fn commander_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(".commander")
}

pub fn db_path(project_dir: &Path) -> PathBuf {
    commander_dir(project_dir).join("db.sqlite")
}

pub fn lock_path(project_dir: &Path) -> PathBuf {
    commander_dir(project_dir).join("supervisor.lock")
}
