use async_trait::async_trait;
use commander_messages::{ContentBlock, Message, TokenUsage};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("api error: {0}")]
    Api(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },
    #[error("context too large: {tokens} tokens exceeds {limit}")]
    ContextTooLarge { tokens: u32, limit: u32 },
}

/// Request sent to the LLM.
///
/// Uses borrowed slices to avoid O(n) cloning of the message history
/// on every LLM call (see HIGH-007).
#[derive(Clone, Copy)]
pub struct LlmRequest<'a> {
    pub messages: &'a [Message],
    pub system_prompt: Option<&'a str>,
    pub tools: &'a [Value],
    pub max_tokens: u32,
}

/// Response from the LLM.
pub struct LlmResponse {
    pub content: Vec<ContentBlock>,
    pub usage: TokenUsage,
    pub stop_reason: StopReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
}

/// Trait for LLM providers. Implementations handle API calls, auth, retries.
#[async_trait]
pub trait LlmAdapter: Send + Sync {
    async fn complete(&self, request: LlmRequest<'_>) -> Result<LlmResponse, AdapterError>;
    fn model_id(&self) -> &str;
    fn context_window(&self) -> u32;
    fn max_output_tokens(&self) -> u32;
}
