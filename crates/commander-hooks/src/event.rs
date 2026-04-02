use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Lifecycle events that hooks can subscribe to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HookEvent {
    PreToolUse { tool: String, input: Value },
    PostToolUse { tool: String, output: Value },
    PostAssistantMessage,
    PreLlmCall,
    SessionStart { session_id: String },
    SessionEnd { session_id: String },
}
