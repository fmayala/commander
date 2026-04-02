pub mod anthropic;
pub mod codex;
pub mod openai;
pub mod openrouter;

use crate::adapter::LlmAdapter;
use anyhow::{anyhow, Result};

/// Create an LLM adapter based on provider name. No fallback — unknown provider is a hard error.
pub fn create_adapter(provider: &str, model: &str) -> Result<Box<dyn LlmAdapter>> {
    match provider {
        "anthropic" => Ok(Box::new(anthropic::AnthropicAdapter::new(model)?)),
        "codex" => Ok(Box::new(codex::CodexAdapter::new(model)?)),
        "openai" => Ok(Box::new(openai::OpenAiAdapter::new(model)?)),
        "openrouter" => Ok(Box::new(openrouter::OpenRouterAdapter::new(model)?)),
        other => Err(anyhow!("unknown provider: {other}")),
    }
}
