use async_trait::async_trait;
use commander_messages::{CompactionMarker, Message};
use commander_tools::ToolOutput;

/// Layer 2 seam: allows orchestration to observe and control the agent loop
/// without Layer 1 knowing anything about orchestration.
#[async_trait]
pub trait LoopObserver: Send + Sync {
    /// Called when a tool needs user approval. Return true to approve.
    async fn on_permission_ask(&self, tool: &str, prompt: &str) -> bool;

    /// Called after a tool completes execution.
    async fn on_tool_complete(&self, tool: &str, result: &ToolOutput);

    /// Called after the assistant produces a message. Return true to continue the loop.
    async fn on_assistant_message(&self, msg: &Message) -> bool;

    /// Called after context compaction occurs.
    async fn on_compaction(&self, marker: &CompactionMarker);
}

/// Default observer that auto-approves all permissions and continues the loop.
pub struct AutoApproveObserver;

#[async_trait]
impl LoopObserver for AutoApproveObserver {
    async fn on_permission_ask(&self, _tool: &str, _prompt: &str) -> bool {
        true
    }
    async fn on_tool_complete(&self, _tool: &str, _result: &ToolOutput) {}
    async fn on_assistant_message(&self, _msg: &Message) -> bool {
        true
    }
    async fn on_compaction(&self, _marker: &CompactionMarker) {}
}
