use serde::{Deserialize, Serialize};

/// Marker stored in Message.metadata when a range of messages is compacted into a summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionMarker {
    /// Index of the first message that was compacted (inclusive).
    pub start_index: usize,
    /// Index of the last message that was compacted (inclusive).
    pub end_index: usize,
    /// Total tokens across the compacted messages.
    pub original_tokens: u32,
    /// Tokens in the summary that replaced them.
    pub summary_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_marker_roundtrip() {
        let marker = CompactionMarker {
            start_index: 0,
            end_index: 10,
            original_tokens: 5000,
            summary_tokens: 200,
        };
        let json = serde_json::to_string(&marker).unwrap();
        let back: CompactionMarker = serde_json::from_str(&json).unwrap();
        assert_eq!(back.start_index, 0);
        assert_eq!(back.end_index, 10);
        assert_eq!(back.original_tokens, 5000);
        assert_eq!(back.summary_tokens, 200);
    }
}
