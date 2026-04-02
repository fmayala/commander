use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Inline threshold: payloads under this size are stored directly in SQLite.
pub const INLINE_THRESHOLD: usize = 4096;

/// Output from one chain step, stored for the next step to consume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainArtifact {
    pub run_id: String,
    pub step_index: u32,
    pub key: String,
    /// Payload stored inline if small enough.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_inline: Option<String>,
    /// Path to spilled file for large payloads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spill_path: Option<PathBuf>,
}

impl ChainArtifact {
    pub fn new_inline(run_id: &str, step_index: u32, key: &str, value: String) -> Self {
        Self {
            run_id: run_id.into(),
            step_index,
            key: key.into(),
            value_inline: Some(value),
            spill_path: None,
        }
    }

    pub fn new_spilled(run_id: &str, step_index: u32, key: &str, spill_path: PathBuf) -> Self {
        Self {
            run_id: run_id.into(),
            step_index,
            key: key.into(),
            value_inline: None,
            spill_path: Some(spill_path),
        }
    }

    /// Read the artifact value, loading from spill path if needed.
    pub fn read_value(&self) -> Result<String, std::io::Error> {
        if let Some(ref inline) = self.value_inline {
            Ok(inline.clone())
        } else if let Some(ref path) = self.spill_path {
            std::fs::read_to_string(path)
        } else {
            Ok(String::new())
        }
    }
}
