use crate::adapter::*;
use async_trait::async_trait;
use commander_messages::{ContentBlock, Message, Role, TokenUsage};
use reqwest::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use xxhash_rust::xxh64::xxh64;

const CCH_SEED: u64 = 0x6E52736AC806831E;
const CCH_MASK: u64 = 0xFFFFF;
const CCH_PLACEHOLDER: &str = "cch=00000";
const CC_VERSION: &str = "2.1.87";

pub struct AnthropicAdapter {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl AnthropicAdapter {
    pub fn new(model: &str) -> anyhow::Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY environment variable is required"))?;
        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .unwrap_or_else(|_| "https://api.anthropic.com".into());

        Ok(Self {
            client: Client::new(),
            api_key,
            base_url,
            model: model.into(),
        })
    }

    fn compute_fingerprint(&self, messages: &[Message]) -> String {
        let first_user_msg = messages
            .iter()
            .find(|m| m.role == Role::User)
            .and_then(|m| m.text())
            .unwrap_or("");

        let chars: Vec<char> = first_user_msg.chars().collect();
        let c4 = chars.get(4).map(|c| c.to_string()).unwrap_or_default();
        let c7 = chars.get(7).map(|c| c.to_string()).unwrap_or_default();
        let c20 = chars.get(20).map(|c| c.to_string()).unwrap_or_default();

        let input = format!("tengu_{c4}{c7}{c20}{CC_VERSION}");
        let hash = Sha256::digest(input.as_bytes());
        format!("{:02x}{:02x}", hash[0], hash[1])[..3].to_string()
    }

    fn build_attribution_header(&self, fingerprint: &str) -> String {
        format!(
            "x-anthropic-billing-header: cc_version={CC_VERSION}.{fingerprint}; cc_entrypoint=cli; {CCH_PLACEHOLDER};"
        )
    }

    fn build_system_blocks(&self, system_prompt: Option<&str>, attribution: &str) -> Value {
        let mut blocks = Vec::new();
        if let Some(prompt) = system_prompt {
            blocks.push(json!({"type": "text", "text": prompt}));
        }
        blocks.push(json!({"type": "text", "text": attribution}));
        Value::Array(blocks)
    }

    fn translate_messages(&self, messages: &[Message]) -> Vec<Value> {
        let mut result = Vec::new();

        for msg in messages {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => continue, // system is handled separately
            };

            let content: Vec<Value> = msg
                .content
                .iter()
                .map(|block| match block {
                    ContentBlock::Text { text } => {
                        json!({"type": "text", "text": text})
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        json!({"type": "tool_use", "id": id, "name": name, "input": input})
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        json!({
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": content,
                            "is_error": is_error
                        })
                    }
                    ContentBlock::Image { media_type, data } => {
                        json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": media_type,
                                "data": data
                            }
                        })
                    }
                })
                .collect();

            result.push(json!({"role": role, "content": content}));
        }

        result
    }

    fn translate_tools(&self, tools: &[Value]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "name": t["name"],
                    "description": t["description"],
                    "input_schema": t["input_schema"]
                })
            })
            .collect()
    }

    /// Build the full request body and apply cch= signing.
    /// Returns the final body string ready to send.
    pub fn build_signed_body(&self, request: &LlmRequest) -> String {
        let fingerprint = self.compute_fingerprint(&request.messages);
        let attribution = self.build_attribution_header(&fingerprint);
        let system = self.build_system_blocks(request.system_prompt.as_deref(), &attribution);
        let messages = self.translate_messages(&request.messages);
        let tools = self.translate_tools(&request.tools);

        let mut body = json!({
            "model": self.model,
            "max_tokens": request.max_tokens,
            "system": system,
            "messages": messages,
        });

        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }

        // Serialize with placeholder
        let body_str = serde_json::to_string(&body).expect("json serialization");

        // Compute cch over the body with placeholder
        let hash = xxh64(body_str.as_bytes(), CCH_SEED);
        let masked = hash & CCH_MASK;
        let cch = format!("{masked:05x}");

        // Replace placeholder with computed hash (exactly one occurrence)
        body_str.replacen(CCH_PLACEHOLDER, &format!("cch={cch}"), 1)
    }

    fn parse_response(&self, body: &Value) -> Result<LlmResponse, AdapterError> {
        let content_arr = body
            .get("content")
            .and_then(|v| v.as_array())
            .ok_or_else(|| AdapterError::Api("missing content in response".into()))?;

        let mut content = Vec::new();
        for block in content_arr {
            let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match block_type {
                "text" => {
                    let text = block["text"].as_str().unwrap_or("").to_string();
                    content.push(ContentBlock::text(text));
                }
                "tool_use" => {
                    content.push(ContentBlock::ToolUse {
                        id: block["id"].as_str().unwrap_or("").to_string(),
                        name: block["name"].as_str().unwrap_or("").to_string(),
                        input: block["input"].clone(),
                    });
                }
                _ => {
                    tracing::debug!(block_type, "skipping unknown content block type");
                }
            }
        }

        let usage = body
            .get("usage")
            .map(|u| TokenUsage {
                input_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
                output_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
                cache_creation_tokens: u
                    .get("cache_creation_input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                cache_read_tokens: u
                    .get("cache_read_input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
            })
            .unwrap_or_default();

        let stop_reason = match body
            .get("stop_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("end_turn")
        {
            "tool_use" => StopReason::ToolUse,
            "max_tokens" => StopReason::MaxTokens,
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
impl LlmAdapter for AnthropicAdapter {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, AdapterError> {
        let body_str = self.build_signed_body(&request);
        let request_id = uuid::Uuid::new_v4().to_string();

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header("x-app", "cli")
            .header("x-client-request-id", &request_id)
            .body(body_str)
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
            if body_text.contains("context") && body_text.contains("too large") {
                return Err(AdapterError::ContextTooLarge {
                    tokens: 0,
                    limit: 0,
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
        200_000
    }

    fn max_output_tokens(&self) -> u32 {
        16384
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commander_messages::Message;

    fn make_adapter() -> AnthropicAdapter {
        AnthropicAdapter {
            client: Client::new(),
            api_key: "test-key".into(),
            base_url: "https://api.anthropic.com".into(),
            model: "claude-sonnet-4-6".into(),
        }
    }

    #[test]
    fn fingerprint_is_3_hex_chars() {
        let adapter = make_adapter();
        let messages = vec![Message::user(
            "This is a test message that is long enough for all indices",
        )];
        let fp = adapter.compute_fingerprint(&messages);
        assert_eq!(fp.len(), 3);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn fingerprint_short_message() {
        let adapter = make_adapter();
        let messages = vec![Message::user("hi")];
        let fp = adapter.compute_fingerprint(&messages);
        assert_eq!(fp.len(), 3);
    }

    #[test]
    fn fingerprint_deterministic() {
        let adapter = make_adapter();
        let messages = vec![Message::user("hello world this is a test")];
        let fp1 = adapter.compute_fingerprint(&messages);
        let fp2 = adapter.compute_fingerprint(&messages);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn cch_is_computed_and_replaced() {
        let adapter = make_adapter();
        let request = LlmRequest {
            messages: vec![Message::user("hello world this is a test message")],
            system_prompt: Some("You are a helpful assistant.".into()),
            tools: vec![],
            max_tokens: 4096,
        };

        let body = adapter.build_signed_body(&request);

        // Must not contain placeholder
        assert!(!body.contains("cch=00000"), "placeholder not replaced");

        // Must contain cch= with a 5-char hex value
        let cch_pos = body.find("cch=").expect("cch= not found in body");
        let cch_value = &body[cch_pos + 4..cch_pos + 9];
        assert_eq!(cch_value.len(), 5);
        assert!(
            cch_value.chars().all(|c| c.is_ascii_hexdigit()),
            "cch value is not hex: {cch_value}"
        );
    }

    #[test]
    fn cch_is_deterministic() {
        let adapter = make_adapter();
        let request = LlmRequest {
            messages: vec![Message::user("deterministic test input")],
            system_prompt: Some("system prompt".into()),
            tools: vec![],
            max_tokens: 4096,
        };

        let body1 = adapter.build_signed_body(&request);
        let body2 = adapter.build_signed_body(&request);
        assert_eq!(body1, body2);
    }

    #[test]
    fn body_contains_attribution_header() {
        let adapter = make_adapter();
        let request = LlmRequest {
            messages: vec![Message::user("test")],
            system_prompt: Some("system".into()),
            tools: vec![],
            max_tokens: 4096,
        };

        let body = adapter.build_signed_body(&request);
        assert!(body.contains("x-anthropic-billing-header"));
        assert!(body.contains("cc_version="));
        assert!(body.contains("cc_entrypoint=cli"));
    }

    #[test]
    fn tools_included_in_body() {
        let adapter = make_adapter();
        let request = LlmRequest {
            messages: vec![Message::user("test")],
            system_prompt: None,
            tools: vec![serde_json::json!({
                "name": "Read",
                "description": "Read a file",
                "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}}
            })],
            max_tokens: 4096,
        };

        let body = adapter.build_signed_body(&request);
        assert!(body.contains("\"tools\""));
        assert!(body.contains("\"Read\""));
    }

    #[test]
    fn parse_response_text_and_tool_use() {
        let adapter = make_adapter();
        let response_json = json!({
            "content": [
                {"type": "text", "text": "Let me read that file."},
                {"type": "tool_use", "id": "tu_1", "name": "Read", "input": {"file_path": "/tmp/test.txt"}}
            ],
            "usage": {"input_tokens": 100, "output_tokens": 50},
            "stop_reason": "tool_use"
        });

        let response = adapter.parse_response(&response_json).unwrap();
        assert_eq!(response.content.len(), 2);
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.usage.input_tokens, 100);
        assert_eq!(response.usage.output_tokens, 50);

        match &response.content[1] {
            ContentBlock::ToolUse { name, .. } => assert_eq!(name, "Read"),
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn parse_response_end_turn() {
        let adapter = make_adapter();
        let response_json = json!({
            "content": [{"type": "text", "text": "Done."}],
            "usage": {"input_tokens": 50, "output_tokens": 10},
            "stop_reason": "end_turn"
        });

        let response = adapter.parse_response(&response_json).unwrap();
        assert_eq!(response.stop_reason, StopReason::EndTurn);
    }
}
