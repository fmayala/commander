use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How to execute agent(s) for a set of tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ExecutionMode {
    /// One agent, one task.
    Single { agent: String, task: String },
    /// Fan-out N agents concurrently.
    Parallel {
        tasks: Vec<(String, String)>,
        concurrency: u32,
        #[serde(default)]
        fail_fast: bool,
    },
    /// Sequential pipeline with variable threading.
    Chain { steps: Vec<ChainStep> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChainStep {
    Sequential {
        agent: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        task: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<PathBuf>,
    },
    Parallel {
        tasks: Vec<(String, String)>,
        concurrency: u32,
    },
}
