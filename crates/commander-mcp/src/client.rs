use crate::config::{McpServerConfig, McpTransport};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Debug, Error)]
pub enum McpError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json-rpc error: code={code}, message={message}")]
    RpcError { code: i64, message: String },
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("transport not supported: {0}")]
    UnsupportedTransport(String),
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("timeout")]
    Timeout,
}

/// JSON-RPC 2.0 request.
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    id: Option<u64>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

/// Client for a single MCP server connection (currently stdio only).
pub struct McpClient {
    config: McpServerConfig,
    child: Option<Child>,
    stdin: Option<tokio::process::ChildStdin>,
    stdout: Option<BufReader<tokio::process::ChildStdout>>,
    next_id: AtomicU64,
}

impl McpClient {
    /// Connect to an MCP server. Currently only supports Stdio transport.
    pub async fn connect(config: McpServerConfig) -> Result<Self, McpError> {
        match &config.transport {
            McpTransport::Stdio { command, args } => {
                let mut child = Command::new(command)
                    .args(args)
                    .envs(&config.env)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .map_err(|e| McpError::ConnectionFailed(format!("{command}: {e}")))?;

                let stdin = child.stdin.take().unwrap();
                let stdout = BufReader::new(child.stdout.take().unwrap());

                let mut client = Self {
                    config,
                    child: Some(child),
                    stdin: Some(stdin),
                    stdout: Some(stdout),
                    next_id: AtomicU64::new(1),
                };

                // Initialize handshake
                client.initialize().await?;
                Ok(client)
            }
            McpTransport::Sse { .. } => {
                Err(McpError::UnsupportedTransport("SSE not yet implemented".into()))
            }
            McpTransport::Http { .. } => {
                Err(McpError::UnsupportedTransport("HTTP not yet implemented".into()))
            }
        }
    }

    async fn initialize(&mut self) -> Result<(), McpError> {
        let _resp = self
            .request(
                "initialize",
                Some(serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "commander",
                        "version": "0.1.0"
                    }
                })),
            )
            .await?;

        // Send initialized notification (no response expected)
        self.notify("notifications/initialized", None).await?;
        Ok(())
    }

    /// Discover all tools offered by this server.
    pub async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>, McpError> {
        let resp = self.request("tools/list", None).await?;
        let tools = resp
            .get("tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        tools
            .into_iter()
            .map(|v| {
                serde_json::from_value::<McpToolInfo>(v)
                    .map_err(|e| McpError::InvalidResponse(e.to_string()))
            })
            .collect()
    }

    /// Call a tool on the server.
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, McpError> {
        let resp = self
            .request(
                "tools/call",
                Some(serde_json::json!({
                    "name": name,
                    "arguments": arguments
                })),
            )
            .await?;
        Ok(resp)
    }

    async fn request(&mut self, method: &str, params: Option<Value>) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        };

        let mut req_bytes = serde_json::to_vec(&req)
            .map_err(|e| McpError::InvalidResponse(e.to_string()))?;
        req_bytes.push(b'\n');

        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(&req_bytes).await?;
        stdin.flush().await?;

        // Read response line
        let stdout = self.stdout.as_mut().unwrap();
        let mut line = String::new();

        let timeout = self.config.request_timeout;
        match tokio::time::timeout(timeout, stdout.read_line(&mut line)).await {
            Ok(Ok(0)) => Err(McpError::InvalidResponse("server closed connection".into())),
            Ok(Ok(_)) => {
                let resp: JsonRpcResponse = serde_json::from_str(line.trim())
                    .map_err(|e| McpError::InvalidResponse(e.to_string()))?;
                if let Some(err) = resp.error {
                    Err(McpError::RpcError {
                        code: err.code,
                        message: err.message,
                    })
                } else {
                    Ok(resp.result.unwrap_or(Value::Null))
                }
            }
            Ok(Err(e)) => Err(McpError::Io(e)),
            Err(_) => Err(McpError::Timeout),
        }
    }

    async fn notify(&mut self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.unwrap_or(Value::Null)
        });
        let mut bytes = serde_json::to_vec(&notification)
            .map_err(|e| McpError::InvalidResponse(e.to_string()))?;
        bytes.push(b'\n');

        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(&bytes).await?;
        stdin.flush().await?;
        Ok(())
    }

    pub fn server_name(&self) -> &str {
        &self.config.name
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
    }
}

/// Tool info as returned by MCP tools/list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,
}
