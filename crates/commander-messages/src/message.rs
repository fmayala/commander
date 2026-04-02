use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub role: Role,
    pub content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    pub timestamp: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            role: Role::User,
            content: vec![ContentBlock::text(text)],
            usage: None,
            timestamp: Utc::now(),
            metadata: Value::Null,
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            role: Role::Assistant,
            content: vec![ContentBlock::text(text)],
            usage: None,
            timestamp: Utc::now(),
            metadata: Value::Null,
        }
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            role: Role::System,
            content: vec![ContentBlock::text(text)],
            usage: None,
            timestamp: Utc::now(),
            metadata: Value::Null,
        }
    }

    pub fn text(&self) -> Option<&str> {
        self.content.iter().find_map(|b| match b {
            ContentBlock::Text { text: t } => Some(t.as_str()),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    Image {
        media_type: String,
        data: String,
    },
}

impl ContentBlock {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text { text: s.into() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_creation_tokens: u32,
    #[serde(default)]
    pub cache_read_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_user_roundtrip() {
        let msg = Message::user("hello world");
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role, Role::User);
        assert_eq!(back.text(), Some("hello world"));
    }

    #[test]
    fn content_block_tool_use_serde() {
        let block = ContentBlock::ToolUse {
            id: "tu_1".into(),
            name: "Read".into(),
            input: serde_json::json!({"path": "/tmp/test.txt"}),
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "tool_use");
        assert_eq!(json["name"], "Read");
    }

    #[test]
    fn content_block_tool_result_serde() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "tu_1".into(),
            content: "file contents".into(),
            is_error: false,
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "tool_result");

        let back: ContentBlock = serde_json::from_value(json).unwrap();
        match back {
            ContentBlock::ToolResult {
                content, is_error, ..
            } => {
                assert_eq!(content, "file contents");
                assert!(!is_error);
            }
            _ => panic!("wrong variant"),
        }
    }
}
