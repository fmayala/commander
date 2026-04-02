use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AMessage {
    pub id: String,
    pub from: String,
    pub to: String,
    pub message_type: A2AMessageType,
    pub content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum A2AMessageType {
    Handover,
    Status,
    Discovery,
    Conflict,
    FileReleaseRequest,
    CompletionNotice,
    ContractUpdate,
}
