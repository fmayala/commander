use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// Durable operations that survive process death.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DurableOp {
    /// Sleep that survives process death. On resume, only remaining time is waited.
    Sleep { duration_ms: u64 },
    /// Wait for external event (human approval, webhook).
    WaitForEvent {
        key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        filter: Option<String>,
    },
    /// Spawn child agent, parent freed until child completes.
    SpawnChild {
        profile_name: String,
        input: Value,
    },
    /// Cache pure computation result. On replay, returns cached result.
    Memo { key: String, result: Value },
}

impl DurableOp {
    pub fn sleep(duration: Duration) -> Self {
        Self::Sleep {
            duration_ms: duration.as_millis() as u64,
        }
    }

    pub fn memo(key: impl Into<String>, result: Value) -> Self {
        Self::Memo {
            key: key.into(),
            result,
        }
    }
}
