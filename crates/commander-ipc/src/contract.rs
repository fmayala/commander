use serde::{Deserialize, Serialize};

/// Cross-agent dependency: "I'll provide X" / "I need X".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    pub item: String,
    pub provider: String,
    pub status: ContractStatus,
    #[serde(default)]
    pub waiters: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractStatus {
    Pending,
    InProgress,
    Ready,
    Modified,
}
