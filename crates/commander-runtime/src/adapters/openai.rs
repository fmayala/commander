use crate::adapter::*;
use async_trait::async_trait;
use commander_messages::{ContentBlock, Role, TokenUsage};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashSet;

pub struct OpenAiAdapter {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenAiAdapter {
    pub fn new(model: &str) -> anyhow::Result<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| anyhow::anyhow!("OPENAI_API_KEY environment variable is required"))?;
        let base_url =
            std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com".into());

        Ok(Self {
            client: Client::new(),
            api_key,
            base_url,
            model: model.into(),
        })
    }

    fn translate_messages(&self, request: &LlmRequest<'_>) -> Vec<Value> {
        let mut result = Vec::new();
        let mut pending_tool_ids: HashSet<String> = HashSet::new();
        let mut pending_assistant_index: Option<usize> = None;

        // System message first
        if let Some(ref prompt) = request.system_prompt {
            result.push(json!({"role": "system", "content": prompt}));
        }

        for msg in request.messages {
            match msg.role {
                Role::System => continue,
                Role::User => {
                    let mut matched_tool_results = 0_usize;
                    for block in &msg.content {
                        if let ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } = block
                        {
                            if pending_tool_ids.remove(tool_use_id) {
                                result.push(json!({
                                    "role": "tool",
                                    "tool_call_id": tool_use_id,
                                    "content": content
                                }));
                                matched_tool_results += 1;
                            }
                        }
                    }

                    if matched_tool_results == 0 && pending_tool_ids.is_empty() {
                        let text = msg.text().unwrap_or("");
                        if !text.is_empty() {
                            result.push(json!({"role": "user", "content": text}));
                        }
                    }
                }
                Role::Assistant => {
                    // Drop dangling assistant tool-call turns from recovered/stale checkpoints.
                    if !pending_tool_ids.is_empty() {
                        if let Some(start_idx) = pending_assistant_index {
                            result.truncate(start_idx);
                        }
                        pending_tool_ids.clear();
                    }

                    let tool_uses: Vec<Value> = msg
                        .content
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::ToolUse { id, name, input } => Some(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(input).unwrap_or_default()
                                }
                            })),
                            _ => None,
                        })
                        .collect();

                    let text = msg.text().unwrap_or("");

                    let mut message = json!({"role": "assistant"});
                    if !text.is_empty() {
                        message["content"] = Value::String(text.to_string());
                    } else if !tool_uses.is_empty() {
                        message["content"] = Value::Null;
                    }
                    if !tool_uses.is_empty() {
                        for tc in &tool_uses {
                            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                pending_tool_ids.insert(id.to_string());
                            }
                        }
                        pending_assistant_index = Some(result.len());
                        message["tool_calls"] = Value::Array(tool_uses);
                    } else {
                        pending_assistant_index = None;
                    }
                    result.push(message);
                }
            }
        }

        // Guardrail: never send unresolved tool calls to OpenAI-compatible APIs.
        if !pending_tool_ids.is_empty() {
            if let Some(start_idx) = pending_assistant_index {
                result.truncate(start_idx);
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
                    "function": {
                        "name": t["name"],
                        "description": t.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                        "parameters": t["input_schema"]
                    }
                })
            })
            .collect()
    }

    /// Build the request body (exposed for testing).
    pub fn build_request_body(&self, request: &LlmRequest<'_>) -> Value {
        let messages = self.translate_messages(request);
        let tools = self.translate_tools(request.tools);

        let mut body = json!({
            "model": self.model,
            "max_tokens": request.max_tokens,
            "messages": messages,
            "stream": false,
        });

        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }

        body
    }

    fn parse_response(&self, body: &Value) -> Result<LlmResponse, AdapterError> {
        let choice = body
            .pointer("/choices/0/message")
            .ok_or_else(|| AdapterError::Api("missing choices[0].message".into()))?;

        let mut content = Vec::new();

        // Text content
        if let Some(text) = choice.get("content").and_then(|v| v.as_str()) {
            if !text.is_empty() {
                content.push(ContentBlock::text(text));
            }
        }

        // Tool calls
        if let Some(tool_calls) = choice.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tool_calls {
                let id = tc["id"].as_str().unwrap_or("").to_string();
                let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                let input: Value = serde_json::from_str(args_str).unwrap_or(Value::Null);
                content.push(ContentBlock::ToolUse { id, name, input });
            }
        }

        let usage = body
            .get("usage")
            .map(|u| TokenUsage {
                input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
                ..Default::default()
            })
            .unwrap_or_default();

        let finish_reason = body
            .pointer("/choices/0/finish_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("stop");

        let stop_reason = match finish_reason {
            "tool_calls" => StopReason::ToolUse,
            "length" => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        Ok(LlmResponse {
            content,
            usage,
            stop_reason,
        })
    }
}

#[async_trait]
impl LlmAdapter for OpenAiAdapter {
    async fn complete(&self, request: LlmRequest<'_>) -> Result<LlmResponse, AdapterError> {
        let body = self.build_request_body(&request);

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
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

        let body: Value = serde_json::from_str(&body_text)
            .map_err(|e| AdapterError::Api(format!("invalid JSON response: {e}")))?;

        self.parse_response(&body)
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn context_window(&self) -> u32 {
        128_000
    }

    fn max_output_tokens(&self) -> u32 {
        16384
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commander_messages::Message;

    fn make_adapter() -> OpenAiAdapter {
        OpenAiAdapter {
            client: Client::new(),
            api_key: "test-key".into(),
            base_url: "https://api.openai.com".into(),
            model: "gpt-5.4".into(),
        }
    }

    #[test]
    fn request_body_structure() {
        let adapter = make_adapter();
        let messages = vec![Message::user("hello")];
        let tools = vec![json!({
            "name": "Read",
            "description": "Read a file",
            "input_schema": {"type": "object"}
        })];
        let request = LlmRequest {
            messages: &messages,
            system_prompt: Some("You are helpful."),
            tools: &tools,
            max_tokens: 4096,
        };

        let body = adapter.build_request_body(&request);
        assert_eq!(body["model"], "gpt-5.4");
        assert_eq!(body["stream"], false);

        // System message first, then user
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");

        // Tool format
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "Read");
    }

    #[test]
    fn parse_text_response() {
        let adapter = make_adapter();
        let response_json = json!({
            "choices": [{
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        });

        let response = adapter.parse_response(&response_json).unwrap();
        assert_eq!(response.content.len(), 1);
        assert_eq!(response.stop_reason, StopReason::EndTurn);
        assert_eq!(response.usage.input_tokens, 10);
    }

    #[test]
    fn parse_tool_call_response() {
        let adapter = make_adapter();
        let response_json = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "Read",
                            "arguments": "{\"file_path\":\"/tmp/test.txt\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 15}
        });

        let response = adapter.parse_response(&response_json).unwrap();
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        match &response.content[0] {
            ContentBlock::ToolUse { name, input, .. } => {
                assert_eq!(name, "Read");
                assert_eq!(input["file_path"], "/tmp/test.txt");
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn drops_dangling_tool_call_turn_from_request_history() {
        let adapter = make_adapter();
        let mut assistant = Message::assistant("");
        assistant.content = vec![
            ContentBlock::ToolUse {
                id: "call_1".into(),
                name: "Read".into(),
                input: json!({"file_path":"a.txt"}),
            },
            ContentBlock::ToolUse {
                id: "call_2".into(),
                name: "Read".into(),
                input: json!({"file_path":"b.txt"}),
            },
        ];
        let mut partial_results = Message::user("");
        partial_results.content = vec![ContentBlock::ToolResult {
            tool_use_id: "call_1".into(),
            content: "ok".into(),
            is_error: false,
        }];

        let msgs = vec![Message::user("start"), assistant, partial_results];
        let request = LlmRequest {
            messages: &msgs,
            system_prompt: None,
            tools: &[],
            max_tokens: 1024,
        };
        let body = adapter.build_request_body(&request);
        let messages = body["messages"].as_array().unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "start");
    }
}
