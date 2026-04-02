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
pub struct LlmRequest {
    pub messages: Vec<Message>,
    pub system_prompt: Option<String>,
    pub tools: Vec<Value>,
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
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, AdapterError>;
    fn model_id(&self) -> &str;
    fn context_window(&self) -> u32;
    fn max_output_tokens(&self) -> u32;
}
