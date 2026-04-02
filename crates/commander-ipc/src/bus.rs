use crate::message::A2AMessage;
use async_trait::async_trait;

/// Trait for inter-agent message delivery.
/// In v0, routed through the supervisor via Unix socket; persisted in SQLite.
#[async_trait]
pub trait MessageBus: Send + Sync {
    async fn send(&self, msg: &A2AMessage) -> Result<(), BusError>;
    async fn inbox(&self, agent_id: &str) -> Result<Vec<A2AMessage>, BusError>;
    async fn acknowledge(&self, agent_id: &str, msg_id: &str) -> Result<(), BusError>;
}

#[derive(Debug, thiserror::Error)]
pub enum BusError {
    #[error("delivery failed: {0}")]
    DeliveryFailed(String),
    #[error("agent not found: {0}")]
    AgentNotFound(String),
}

/// In-memory message bus for testing.
pub struct InMemoryBus {
    messages: std::sync::Mutex<Vec<A2AMessage>>,
}

impl InMemoryBus {
    pub fn new() -> Self {
        Self {
            messages: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl Default for InMemoryBus {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MessageBus for InMemoryBus {
    async fn send(&self, msg: &A2AMessage) -> Result<(), BusError> {
        self.messages.lock().unwrap().push(msg.clone());
        Ok(())
    }

    async fn inbox(&self, agent_id: &str) -> Result<Vec<A2AMessage>, BusError> {
        let msgs = self.messages.lock().unwrap();
        Ok(msgs.iter().filter(|m| m.to == agent_id).cloned().collect())
    }

    async fn acknowledge(&self, _agent_id: &str, msg_id: &str) -> Result<(), BusError> {
        let mut msgs = self.messages.lock().unwrap();
        msgs.retain(|m| m.id != msg_id);
        Ok(())
    }
}
