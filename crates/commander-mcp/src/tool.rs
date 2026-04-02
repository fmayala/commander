use crate::client::McpClient;
use async_trait::async_trait;
use commander_tools::tool::*;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Wraps an MCP server tool as a Commander `Tool` implementation.
/// Named `mcp__{server}__{tool_name}`.
pub struct McpTool {
    spec: ToolSpec,
    server_name: String,
    tool_name: String,
    client: Arc<Mutex<McpClient>>,
}

impl McpTool {
    pub fn new(
        server_name: &str,
        tool_name: &str,
        description: Option<&str>,
        input_schema: Value,
        client: Arc<Mutex<McpClient>>,
    ) -> Self {
        let qualified_name = format!("mcp__{server_name}__{tool_name}");
        Self {
            spec: ToolSpec {
                name: qualified_name,
                description: description.unwrap_or("MCP tool").to_string(),
                input_schema,
                concurrency: ConcurrencyClass::Serial,
            },
            server_name: server_name.to_string(),
            tool_name: tool_name.to_string(),
            client,
        }
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }
}

#[async_trait]
impl Tool for McpTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, _input: &Value) -> Result<(), ToolError> {
        Ok(())
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let mut client = self.client.lock().await;
        let result = client
            .call_tool(&self.tool_name, input)
            .await
            .map_err(|e| ToolError::Execution(format!("MCP call failed: {e}")))?;

        // MCP tools/call returns {content: [{type: "text", text: "..."}], isError: bool}
        let is_error = result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let content_text = result
            .get("content")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if is_error {
            Ok(ToolOutput::error(content_text))
        } else {
            Ok(ToolOutput::success(Value::String(content_text.to_string())))
        }
    }
}
