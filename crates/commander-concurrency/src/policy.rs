use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcurrencyPolicy {
    /// Field path into Task to derive group key, e.g. "project_id" or "global".
    pub key_expr: String,
    /// Max concurrent runs per derived group key.
    pub max_runs: u32,
    /// What to do when at capacity.
    #[serde(default)]
    pub strategy: ConcurrencyStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConcurrencyStrategy {
    /// Wait in line.
    #[default]
    Queue,
    /// Fair scheduling across groups.
    GroupRoundRobin,
    /// Newest wins, cancel oldest.
    CancelInProgress,
    /// First wins, reject new.
    CancelNewest,
}
