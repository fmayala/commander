use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// Sent to hook subprocess stdin as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookInput {
    pub event: crate::event::HookEvent,
    pub session_id: String,
}

/// Read from hook subprocess stdout as JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookOutput {
    /// If set, replace the tool input with this value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutated_payload: Option<Value>,
    /// If true, block the action.
    #[serde(default)]
    pub block: bool,
    /// Reason for blocking (shown to the LLM as a tool error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_reason: Option<String>,
    /// Additional messages to inject into the conversation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inject_messages: Vec<String>,
}

/// Configuration for a single hook entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEntry {
    /// Which event this hook subscribes to (matched by event name).
    pub event: String,
    /// Shell command to execute.
    pub command: String,
    /// Working directory for the hook process.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Timeout for hook execution.
    #[serde(
        default = "default_timeout",
        with = "humantime_serde",
        skip_serializing_if = "is_default_timeout"
    )]
    pub timeout: Duration,
    /// If true, the action waits for this hook to complete.
    #[serde(default = "default_blocking")]
    pub blocking: bool,
    /// Human-readable name for logging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

fn default_timeout() -> Duration {
    Duration::from_secs(10)
}

fn default_blocking() -> bool {
    true
}

fn is_default_timeout(d: &Duration) -> bool {
    *d == default_timeout()
}

mod humantime_serde {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}
