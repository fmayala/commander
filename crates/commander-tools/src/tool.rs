use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::path_guard::PathGuard;

/// Metadata about a tool: name, description, schema, concurrency class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's input.
    pub input_schema: Value,
    /// Whether this tool can run concurrently with other tools of the same class.
    #[serde(default)]
    pub concurrency: ConcurrencyClass,
}

/// Whether a tool can run in parallel with other tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConcurrencyClass {
    /// Can safely run in parallel with other Concurrent tools.
    #[default]
    Concurrent,
    /// Must run alone (e.g., Write, Edit, Bash).
    Serial,
}

/// Output from a tool execution.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: Value,
    pub is_error: bool,
    pub modifier: Option<ContextModifier>,
}

impl ToolOutput {
    pub fn success(content: Value) -> Self {
        Self {
            content,
            is_error: false,
            modifier: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: Value::String(message.into()),
            is_error: true,
            modifier: None,
        }
    }

    pub fn with_modifier(mut self, modifier: ContextModifier) -> Self {
        self.modifier = Some(modifier);
        self
    }
}

/// Side effects a tool can request on the conversation context.
#[derive(Debug, Clone)]
pub enum ContextModifier {
    InjectSystemMessage(String),
    UpdateSystemPrompt(String),
    RequestCompaction,
}

/// Context passed to every tool call. Deliberately separate from conversation context.
pub struct ToolContext {
    pub cwd: PathBuf,
    pub session_id: String,
    pub cancel: CancellationToken,
    pub env: HashMap<String, String>,
    pub path_guard: Option<Arc<dyn PathGuard>>,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("boundary violation: {path} is outside allowed scope")]
    BoundaryViolation { path: String },
    #[error("execution error: {0}")]
    Execution(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("cancelled")]
    Cancelled,
}

/// The core tool trait. Every tool (built-in, MCP, discovery) implements this.
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> &ToolSpec;
    fn validate(&self, input: &Value) -> Result<(), ToolError>;
    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError>;
}
