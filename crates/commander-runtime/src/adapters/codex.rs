use crate::adapter::*;
use async_trait::async_trait;
use commander_messages::{ContentBlock, TokenUsage};
use reqwest::Client;
use serde_json::{json, Value};

pub struct CodexAdapter {
    client: Client,
    access_token: String,
    account_id: String,
    model: String,
}

impl CodexAdapter {
    pub fn new(model: &str) -> anyhow::Result<Self> {
        // Kill switch
        let enabled = std::env::var("COMMANDER_CODEX_ENABLED").unwrap_or_default();
        if enabled != "1" {
            anyhow::bail!("Codex adapter requires COMMANDER_CODEX_ENABLED=1");
        }

        let access_token = std::env::var("CODEX_ACCESS_TOKEN")
            .map_err(|_| anyhow::anyhow!("CODEX_ACCESS_TOKEN environment variable is required"))?;

        let account_id = extract_account_id(&access_token)?;
        let model = map_model(model)?;

        Ok(Self {
            client: Client::new(),
            access_token,
            account_id,
            model,
        })
    }

    fn translate_messages(&self, messages: &[commander_messages::Message]) -> Vec<Value> {
        let mut result = Vec::new();
        for msg in messages {
            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        let role = match msg.role {
                            commander_messages::Role::User => "user",
                            commander_messages::Role::Assistant => "assistant",
                            commander_messages::Role::System => continue,
                        };
                        result.push(json!({
                            "type": "message",
                            "role": role,
                            "content": text
                        }));
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        result.push(json!({
                            "type": "function_call",
                            "name": name,
                            "call_id": id,
                            "arguments": serde_json::to_string(input).unwrap_or_default()
                        }));
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        result.push(json!({
                            "type": "function_call_output",
                            "call_id": tool_use_id,
                            "output": content
                        }));
                    }
                    ContentBlock::Image { .. } => {
                        // Image support can be added later
                    }
                }
            }
        }
        result
    }

    fn translate_tools(&self, tools: &[Value]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t["name"],
                    "description": t.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                    "parameters": t["input_schema"],
                    "strict": null
                })
            })
            .collect()
    }

    /// Build the Codex request body from an LlmRequest.
    pub fn build_request_body(&self, request: &LlmRequest) -> Value {
        let input = self.translate_messages(&request.messages);
        let tools = self.translate_tools(&request.tools);

        let mut body = json!({
            "model": self.model,
            "store": false,
            "stream": true,
            "input": input,
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "max_output_tokens": request.max_tokens,
        });

        if let Some(ref prompt) = request.system_prompt {
            body["instructions"] = Value::String(prompt.clone());
        }

        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }

        body
    }

    /// Parse accumulated SSE events into an LlmResponse.
    fn parse_sse_stream(&self, body_text: &str) -> Result<LlmResponse, AdapterError> {
        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        let mut current_text = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_call_id = String::new();
        let mut current_tool_args = String::new();
        let mut in_text_block = false;
        let mut in_tool_block = false;
        let mut usage = TokenUsage::default();

        for line in body_text.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                let data = data.trim();
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                let event: Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

                match event_type {
                    "response.output_item.added" => {
                        let item_type = event
                            .pointer("/item/type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        match item_type {
                            "message" => {
                                in_text_block = true;
                                in_tool_block = false;
                                current_text.clear();
                            }
                            "function_call" => {
                                in_tool_block = true;
                                in_text_block = false;
                                current_tool_name = event
                                    .pointer("/item/name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                current_tool_call_id = event
                                    .pointer("/item/call_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                current_tool_args.clear();
                            }
                            _ => {}
                        }
                    }
                    "response.output_text.delta" => {
                        if in_text_block {
                            if let Some(delta) = event.get("delta").and_then(|v| v.as_str()) {
                                current_text.push_str(delta);
                            }
                        }
                    }
                    "response.function_call_arguments.delta" => {
                        if in_tool_block {
                            if let Some(delta) = event.get("delta").and_then(|v| v.as_str()) {
                                current_tool_args.push_str(delta);
                            }
                        }
                    }
                    "response.output_item.done" => {
                        if in_text_block && !current_text.is_empty() {
                            content_blocks.push(ContentBlock::text(&current_text));
                            current_text.clear();
                            in_text_block = false;
                        }
                        if in_tool_block {
                            let input: Value =
                                serde_json::from_str(&current_tool_args).unwrap_or(Value::Null);
                            content_blocks.push(ContentBlock::ToolUse {
                                id: current_tool_call_id.clone(),
                                name: current_tool_name.clone(),
                                input,
                            });
                            current_tool_args.clear();
                            in_tool_block = false;
                        }
                    }
                    "response.completed" => {
                        // Finalize any remaining text
                        if in_text_block && !current_text.is_empty() {
                            content_blocks.push(ContentBlock::text(&current_text));
                        }

                        if let Some(u) = event.pointer("/response/usage") {
                            usage.input_tokens = u["input_tokens"].as_u64().unwrap_or(0) as u32;
                            usage.output_tokens = u["output_tokens"].as_u64().unwrap_or(0) as u32;
                        }
                    }
                    _ => {} // Discard reasoning deltas, etc.
                }
            }
        }

        let has_tool_calls = content_blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
        let stop_reason = if has_tool_calls {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        };

        Ok(LlmResponse {
            content: content_blocks,
            usage,
            stop_reason,
        })
    }
}

#[async_trait]
impl LlmAdapter for CodexAdapter {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, AdapterError> {
        let body = self.build_request_body(&request);

        let response = self
            .client
            .post("https://chatgpt.com/backend-api/codex/responses")
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("chatgpt-account-id", &self.account_id)
            .header("originator", "pi")
            .header("OpenAI-Beta", "responses=experimental")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AdapterError::Network(e.to_string()))?;

        let status = response.status();
        let body_text = response
            .text()
            .await
            .map_err(|e| AdapterError::Network(e.to_string()))?;

        if !status.is_success() {
            if status.as_u16() == 429 {
                return Err(AdapterError::RateLimited {
                    retry_after_ms: 5000,
                });
            }
            return Err(AdapterError::Api(format!(
                "HTTP {}: {}",
                status.as_u16(),
                &body_text[..body_text.len().min(500)]
            )));
        }

        self.parse_sse_stream(&body_text)
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn context_window(&self) -> u32 {
        200_000
    }

    fn max_output_tokens(&self) -> u32 {
        16384
    }
}

/// Extract chatgpt_account_id from JWT payload (no signature verification).
fn extract_account_id(token: &str) -> anyhow::Result<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        anyhow::bail!("CODEX_ACCESS_TOKEN is not a valid JWT (expected 3 parts)");
    }

    // Decode the payload (middle segment)
    use base64::Engine;
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| anyhow::anyhow!("failed to decode JWT payload: {e}"))?;

    let payload: Value = serde_json::from_slice(&payload_bytes)
        .map_err(|e| anyhow::anyhow!("failed to parse JWT payload JSON: {e}"))?;

    let account_id = payload
        .pointer("/https://api.openai.com/auth/chatgpt_account_id")
        .or_else(|| payload.pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("JWT payload missing chatgpt_account_id claim"))?
        .to_string();

    Ok(account_id)
}

/// Map a model string to a Codex model identifier.
fn map_model(model: &str) -> anyhow::Result<String> {
    let lower = model.to_lowercase();
    if lower.contains("opus") {
        Ok("gpt-5.1-codex-max".into())
    } else if lower.contains("sonnet") {
        Ok("gpt-5.2-codex".into())
    } else if lower.contains("haiku") {
        Ok("gpt-5.1-codex-mini".into())
    } else if lower.starts_with("gpt-") {
        Ok(model.to_string())
    } else {
        Err(anyhow::anyhow!("unknown model for codex provider: {model}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_mapping() {
        assert_eq!(map_model("claude-opus-4-6").unwrap(), "gpt-5.1-codex-max");
        assert_eq!(map_model("claude-sonnet-4-6").unwrap(), "gpt-5.2-codex");
        assert_eq!(map_model("claude-haiku-4-5").unwrap(), "gpt-5.1-codex-mini");
        assert_eq!(map_model("gpt-5.4").unwrap(), "gpt-5.4");
        assert!(map_model("llama-3").is_err());
    }

    #[test]
    fn request_body_structure() {
        let adapter = CodexAdapter {
            client: Client::new(),
            access_token: "fake".into(),
            account_id: "acc-123".into(),
            model: "gpt-5.2-codex".into(),
        };

        let request = LlmRequest {
            messages: vec![commander_messages::Message::user("hello")],
            system_prompt: Some("You are helpful.".into()),
            tools: vec![json!({
                "name": "Read",
                "description": "Read a file",
                "input_schema": {"type": "object"}
            })],
            max_tokens: 4096,
        };

        let body = adapter.build_request_body(&request);
        assert_eq!(body["model"], "gpt-5.2-codex");
        assert_eq!(body["instructions"], "You are helpful.");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert!(body["input"].is_array());
        assert!(body["tools"].is_array());
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "Read");
    }

    #[test]
    fn sse_parse_text_response() {
        let adapter = CodexAdapter {
            client: Client::new(),
            access_token: "fake".into(),
            account_id: "acc-123".into(),
            model: "gpt-5.2-codex".into(),
        };

        let sse = r#"data: {"type":"response.output_item.added","item":{"type":"message","role":"assistant"}}

data: {"type":"response.output_text.delta","delta":"Hello "}

data: {"type":"response.output_text.delta","delta":"world!"}

data: {"type":"response.output_item.done"}

data: {"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":5}}}

"#;

        let response = adapter.parse_sse_stream(sse).unwrap();
        assert_eq!(response.content.len(), 1);
        match &response.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Hello world!"),
            _ => panic!("expected text"),
        }
        assert_eq!(response.stop_reason, StopReason::EndTurn);
        assert_eq!(response.usage.input_tokens, 10);
    }

    #[test]
    fn sse_parse_tool_call() {
        let adapter = CodexAdapter {
            client: Client::new(),
            access_token: "fake".into(),
            account_id: "acc-123".into(),
            model: "gpt-5.2-codex".into(),
        };

        let sse = r#"data: {"type":"response.output_item.added","item":{"type":"function_call","name":"Read","call_id":"call_1"}}

data: {"type":"response.function_call_arguments.delta","delta":"{\"file_path\":"}

data: {"type":"response.function_call_arguments.delta","delta":"\"/tmp/test.txt\"}"}

data: {"type":"response.output_item.done"}

data: {"type":"response.completed","response":{"usage":{"input_tokens":20,"output_tokens":15}}}

"#;

        let response = adapter.parse_sse_stream(sse).unwrap();
        assert_eq!(response.content.len(), 1);
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        match &response.content[0] {
            ContentBlock::ToolUse { name, id, input } => {
                assert_eq!(name, "Read");
                assert_eq!(id, "call_1");
                assert_eq!(input["file_path"], "/tmp/test.txt");
            }
            _ => panic!("expected ToolUse"),
        }
    }
}
