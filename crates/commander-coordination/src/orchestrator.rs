use async_trait::async_trait;
use commander_tasks::task::Task;
use serde::{Deserialize, Serialize};

/// Result of a completed task's validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub passed: bool,
    pub issues: Vec<ReviewIssue>,
    #[serde(default)]
    pub criteria_evidence: Vec<CriterionEvidence>,
    pub summary: String,
}

/// Per-acceptance-criterion evidence from the agent's complete_task call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriterionEvidence {
    pub criterion: String,
    pub evidence: String,
}

/// Context for retrying a failed task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryContext {
    pub attempt: u32,
    pub failure_reason: String,
    pub issues: Vec<ReviewIssue>,
}

/// Issue found during validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewIssue {
    pub file: String,
    pub severity: Severity,
    pub category: Category,
    pub description: String,
    #[serde(default)]
    pub fix_attempts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Suggestion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Bug,
    Logic,
    Type,
    Integration,
    Boundary,
    Style,
}

/// Validation result from the pipeline.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub passed: bool,
    pub issues: Vec<ReviewIssue>,
}

/// Layer 3 seam: the management loop drives orchestration through this trait.
///
/// Rule-based only in v0: no LLM calls. All decisions are deterministic.
#[async_trait]
pub trait Orchestrator: Send + Sync {
    /// Tasks that are Pending, have deps satisfied, highest priority first.
    async fn next_runnable_tasks(&self) -> Vec<Task>;

    /// Atomically Pending -> Claimed. Err if already claimed.
    async fn claim_task(&self, task_id: &str, agent_id: &str) -> Result<Task, OrchestratorError>;

    /// Reverts Claimed -> Pending (spawn failure compensation).
    async fn unclaim_task(&self, task_id: &str) -> Result<(), OrchestratorError>;

    /// Claimed -> Complete with result.
    async fn complete_task(
        &self,
        task_id: &str,
        result: &TaskResult,
    ) -> Result<(), OrchestratorError>;

    /// Claimed -> Retrying. Schedules requeue after backoff internally.
    async fn retry_task(
        &self,
        task_id: &str,
        context: &RetryContext,
    ) -> Result<(), OrchestratorError>;

    /// Any -> Escalated with reason.
    async fn escalate_task(
        &self,
        task_id: &str,
        reason: &str,
    ) -> Result<(), OrchestratorError>;

    /// Run the validation pipeline for a completed task.
    async fn validate(&self, task_id: &str) -> ValidationResult;
}

#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("task not found: {0}")]
    TaskNotFound(String),
    #[error("invalid transition: {0}")]
    InvalidTransition(String),
    #[error("already claimed: {0}")]
    AlreadyClaimed(String),
    #[error("internal error: {0}")]
    Internal(String),
}

/// In-memory orchestrator for testing. Wraps a TaskQueue directly.
pub struct InMemoryOrchestrator {
    pub queue: std::sync::Mutex<commander_tasks::queue::TaskQueue>,
}

impl InMemoryOrchestrator {
    pub fn new(queue: commander_tasks::queue::TaskQueue) -> Self {
        Self {
            queue: std::sync::Mutex::new(queue),
        }
    }
}

#[async_trait]
impl Orchestrator for InMemoryOrchestrator {
    async fn next_runnable_tasks(&self) -> Vec<Task> {
        let q = self.queue.lock().unwrap();
        q.next_available().into_iter().cloned().collect()
    }

    async fn claim_task(&self, task_id: &str, agent_id: &str) -> Result<Task, OrchestratorError> {
        let mut q = self.queue.lock().unwrap();
        q.claim(task_id, agent_id)
            .map(|t| t.clone())
            .map_err(|e| OrchestratorError::InvalidTransition(e.to_string()))
    }

    async fn unclaim_task(&self, task_id: &str) -> Result<(), OrchestratorError> {
        let mut q = self.queue.lock().unwrap();
        q.unclaim(task_id)
            .map_err(|e| OrchestratorError::InvalidTransition(e.to_string()))
    }

    async fn complete_task(
        &self,
        task_id: &str,
        _result: &TaskResult,
    ) -> Result<(), OrchestratorError> {
        let mut q = self.queue.lock().unwrap();
        q.complete(task_id)
            .map_err(|e| OrchestratorError::InvalidTransition(e.to_string()))
    }

    async fn retry_task(
        &self,
        task_id: &str,
        _context: &RetryContext,
    ) -> Result<(), OrchestratorError> {
        let mut q = self.queue.lock().unwrap();
        q.set_retrying(task_id)
            .map_err(|e| OrchestratorError::InvalidTransition(e.to_string()))?;
        // In a real implementation, schedule requeue after backoff
        q.requeue_retrying(task_id)
            .map_err(|e| OrchestratorError::InvalidTransition(e.to_string()))
    }

    async fn escalate_task(
        &self,
        task_id: &str,
        _reason: &str,
    ) -> Result<(), OrchestratorError> {
        let mut q = self.queue.lock().unwrap();
        q.escalate(task_id)
            .map_err(|e| OrchestratorError::InvalidTransition(e.to_string()))
    }

    async fn validate(&self, _task_id: &str) -> ValidationResult {
        ValidationResult {
            passed: true,
            issues: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commander_tasks::task::{Task, TaskStatus};
    use commander_tasks::queue::TaskQueue;

    fn task(id: &str) -> Task {
        Task::new(id, "proj", format!("Task {id}"))
    }

    #[tokio::test]
    async fn orchestrator_claim_complete_flow() {
        let mut q = TaskQueue::new();
        q.insert(task("A"));
        q.insert(task("B").with_depends_on(vec!["A".into()]));

        let orch = InMemoryOrchestrator::new(q);

        // Only A is runnable
        let runnable = orch.next_runnable_tasks().await;
        assert_eq!(runnable.len(), 1);
        assert_eq!(runnable[0].id, "A");

        // Claim and complete A
        let claimed = orch.claim_task("A", "agent-1").await.unwrap();
        assert_eq!(claimed.status, TaskStatus::Claimed);

        let result = TaskResult {
            passed: true,
            issues: vec![],
            criteria_evidence: vec![],
            summary: "done".into(),
        };
        orch.complete_task("A", &result).await.unwrap();

        // Now B is runnable
        let runnable = orch.next_runnable_tasks().await;
        assert_eq!(runnable.len(), 1);
        assert_eq!(runnable[0].id, "B");
    }

    #[tokio::test]
    async fn orchestrator_unclaim_on_spawn_failure() {
        let mut q = TaskQueue::new();
        q.insert(task("A"));

        let orch = InMemoryOrchestrator::new(q);
        orch.claim_task("A", "agent-1").await.unwrap();
        orch.unclaim_task("A").await.unwrap();

        // A is runnable again
        let runnable = orch.next_runnable_tasks().await;
        assert_eq!(runnable.len(), 1);
    }

    #[tokio::test]
    async fn orchestrator_escalate() {
        let mut q = TaskQueue::new();
        q.insert(task("A"));

        let orch = InMemoryOrchestrator::new(q);
        orch.claim_task("A", "agent-1").await.unwrap();
        orch.escalate_task("A", "too many retries").await.unwrap();

        // Escalated tasks are not runnable
        let runnable = orch.next_runnable_tasks().await;
        assert!(runnable.is_empty());
    }
}
