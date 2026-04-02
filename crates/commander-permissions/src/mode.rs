use serde::{Deserialize, Serialize};

/// How the permission engine handles tool calls that don't match any explicit rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PermissionMode {
    /// Prompt the user for each unmatched tool call.
    #[serde(alias = "ask")]
    #[default]
    Ask,

    /// Allow read-only tools automatically; prompt for writes.
    #[serde(alias = "normal")]
    Normal,

    /// Allow all tool calls automatically unless explicitly denied.
    #[serde(alias = "auto")]
    AutoApprove,
}
