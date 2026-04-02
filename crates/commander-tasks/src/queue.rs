use crate::dependency::{deps_satisfied, subtasks_complete};
use crate::task::{Task, TaskStatus};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("task {0} not found")]
    NotFound(String),
    #[error("task {0} is in status {1:?}, expected {2:?}")]
    InvalidTransition(String, TaskStatus, &'static str),
    #[error("task {0} already claimed by {1}")]
    AlreadyClaimed(String, String),
}

/// In-memory task queue with priority ordering and dependency resolution.
pub struct TaskQueue {
    tasks: HashMap<String, Task>,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
        }
    }

    pub fn insert(&mut self, task: Task) {
        self.tasks.insert(task.id.clone(), task);
    }

    pub fn get(&self, id: &str) -> Option<&Task> {
        self.tasks.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.tasks.get_mut(id)
    }

    pub fn all(&self) -> &HashMap<String, Task> {
        &self.tasks
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    // --- The 8 transition methods ---

    /// Returns tasks that are Pending, have all deps complete, and are not subtask-blocked.
    /// Sorted by priority (P0 first).
    pub fn next_available(&self) -> Vec<&Task> {
        let mut available: Vec<&Task> = self
            .tasks
            .values()
            .filter(|t| t.status == TaskStatus::Pending)
            .filter(|t| deps_satisfied(t, &self.tasks))
            .filter(|t| {
                // If this task has subtasks, they must all be complete
                subtasks_complete(&t.id, &self.tasks)
            })
            .collect();

        available.sort_by_key(|t| t.priority);
        available
    }

    /// Atomic Pending -> Claimed. Returns Err if already claimed or wrong status.
    pub fn claim(&mut self, task_id: &str, agent_id: &str) -> Result<&Task, QueueError> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| QueueError::NotFound(task_id.into()))?;

        match task.status {
            TaskStatus::Pending => {
                task.status = TaskStatus::Claimed;
                task.claimed_by = Some(agent_id.into());
                Ok(self.tasks.get(task_id).unwrap())
            }
            TaskStatus::Claimed => Err(QueueError::AlreadyClaimed(
                task_id.into(),
                task.claimed_by.clone().unwrap_or_default(),
            )),
            other => Err(QueueError::InvalidTransition(
                task_id.into(),
                other,
                "Pending",
            )),
        }
    }

    /// Claimed -> Pending. Used when spawn fails after claim.
    pub fn unclaim(&mut self, task_id: &str) -> Result<(), QueueError> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| QueueError::NotFound(task_id.into()))?;

        if task.status != TaskStatus::Claimed {
            return Err(QueueError::InvalidTransition(
                task_id.into(),
                task.status,
                "Claimed",
            ));
        }

        task.status = TaskStatus::Pending;
        task.claimed_by = None;
        Ok(())
    }

    /// Claimed -> Complete.
    pub fn complete(&mut self, task_id: &str) -> Result<(), QueueError> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| QueueError::NotFound(task_id.into()))?;

        if task.status != TaskStatus::Claimed {
            return Err(QueueError::InvalidTransition(
                task_id.into(),
                task.status,
                "Claimed",
            ));
        }

        task.status = TaskStatus::Complete;
        Ok(())
    }

    /// Claimed -> Retrying. Called by orchestrator's retry_task() impl.
    pub fn set_retrying(&mut self, task_id: &str) -> Result<(), QueueError> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| QueueError::NotFound(task_id.into()))?;

        if task.status != TaskStatus::Claimed {
            return Err(QueueError::InvalidTransition(
                task_id.into(),
                task.status,
                "Claimed",
            ));
        }

        task.status = TaskStatus::Retrying;
        task.claimed_by = None;
        Ok(())
    }

    /// Any -> Escalated. Blocks further automatic processing.
    pub fn escalate(&mut self, task_id: &str) -> Result<(), QueueError> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| QueueError::NotFound(task_id.into()))?;

        task.status = TaskStatus::Escalated;
        task.claimed_by = None;
        Ok(())
    }

    /// Insert a subtask. The parent is blocked until all subtasks complete.
    pub fn add_subtask(&mut self, parent_id: &str, subtask: Task) -> Result<(), QueueError> {
        if !self.tasks.contains_key(parent_id) {
            return Err(QueueError::NotFound(parent_id.into()));
        }
        let mut task = subtask;
        task.parent_id = Some(parent_id.into());
        self.tasks.insert(task.id.clone(), task);
        Ok(())
    }

    /// Insert a discovered task (Discovered status, needs coordinator approval).
    pub fn add_discovered(&mut self, mut task: Task) {
        task.status = TaskStatus::Discovered;
        self.tasks.insert(task.id.clone(), task);
    }

    /// Promote a discovered task to Pending.
    pub fn approve_discovered(&mut self, task_id: &str) -> Result<(), QueueError> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| QueueError::NotFound(task_id.into()))?;

        if task.status != TaskStatus::Discovered {
            return Err(QueueError::InvalidTransition(
                task_id.into(),
                task.status,
                "Discovered",
            ));
        }

        task.status = TaskStatus::Pending;
        Ok(())
    }

    /// Transition Retrying -> Pending (after backoff delay has elapsed).
    pub fn requeue_retrying(&mut self, task_id: &str) -> Result<(), QueueError> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| QueueError::NotFound(task_id.into()))?;

        if task.status != TaskStatus::Retrying {
            return Err(QueueError::InvalidTransition(
                task_id.into(),
                task.status,
                "Retrying",
            ));
        }

        task.status = TaskStatus::Pending;
        Ok(())
    }
}

impl Default for TaskQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Priority;

    fn task(id: &str) -> Task {
        Task::new(id, "test-project", format!("Task {id}"))
    }

    #[test]
    fn next_available_respects_priority() {
        let mut q = TaskQueue::new();
        q.insert(task("A").with_priority(Priority::P2));
        q.insert(task("B").with_priority(Priority::P0));
        q.insert(task("C").with_priority(Priority::P1));

        let avail = q.next_available();
        assert_eq!(avail.len(), 3);
        assert_eq!(avail[0].id, "B"); // P0 first
        assert_eq!(avail[1].id, "C"); // P1
        assert_eq!(avail[2].id, "A"); // P2
    }

    #[test]
    fn next_available_respects_dependencies() {
        let mut q = TaskQueue::new();
        q.insert(task("A"));
        q.insert(task("B").with_depends_on(vec!["A".into()]));
        q.insert(task("C").with_depends_on(vec!["B".into()]));

        let avail = q.next_available();
        assert_eq!(avail.len(), 1);
        assert_eq!(avail[0].id, "A");

        q.claim("A", "agent-1").unwrap();
        q.complete("A").unwrap();

        let avail = q.next_available();
        assert_eq!(avail.len(), 1);
        assert_eq!(avail[0].id, "B");
    }

    #[test]
    fn diamond_dependency() {
        let mut q = TaskQueue::new();
        q.insert(task("A"));
        q.insert(task("B").with_depends_on(vec!["A".into()]));
        q.insert(task("C").with_depends_on(vec!["A".into()]));
        q.insert(task("D").with_depends_on(vec!["B".into(), "C".into()]));

        assert_eq!(q.next_available().len(), 1); // only A

        q.claim("A", "a1").unwrap();
        q.complete("A").unwrap();
        let avail = q.next_available();
        assert_eq!(avail.len(), 2); // B and C

        q.claim("B", "a2").unwrap();
        q.complete("B").unwrap();
        assert_eq!(q.next_available().len(), 1); // only C, D still blocked

        q.claim("C", "a3").unwrap();
        q.complete("C").unwrap();
        let avail = q.next_available();
        assert_eq!(avail.len(), 1);
        assert_eq!(avail[0].id, "D");
    }

    #[test]
    fn claim_idempotent_rejection() {
        let mut q = TaskQueue::new();
        q.insert(task("A"));

        q.claim("A", "agent-1").unwrap();
        let err = q.claim("A", "agent-2").unwrap_err();
        assert!(matches!(err, QueueError::AlreadyClaimed(_, _)));
    }

    #[test]
    fn unclaim_reverts_to_pending() {
        let mut q = TaskQueue::new();
        q.insert(task("A"));

        q.claim("A", "agent-1").unwrap();
        q.unclaim("A").unwrap();

        let t = q.get("A").unwrap();
        assert_eq!(t.status, TaskStatus::Pending);
        assert!(t.claimed_by.is_none());
    }

    #[test]
    fn set_retrying_and_requeue() {
        let mut q = TaskQueue::new();
        q.insert(task("A"));

        q.claim("A", "agent-1").unwrap();
        q.set_retrying("A").unwrap();
        assert_eq!(q.get("A").unwrap().status, TaskStatus::Retrying);

        q.requeue_retrying("A").unwrap();
        assert_eq!(q.get("A").unwrap().status, TaskStatus::Pending);

        // Should be available again
        assert_eq!(q.next_available().len(), 1);
    }

    #[test]
    fn escalate_from_any_status() {
        let mut q = TaskQueue::new();
        q.insert(task("A"));
        q.escalate("A").unwrap();
        assert_eq!(q.get("A").unwrap().status, TaskStatus::Escalated);

        let mut q2 = TaskQueue::new();
        q2.insert(task("B"));
        q2.claim("B", "a1").unwrap();
        q2.escalate("B").unwrap();
        assert_eq!(q2.get("B").unwrap().status, TaskStatus::Escalated);
    }

    #[test]
    fn subtask_blocks_parent() {
        let mut q = TaskQueue::new();
        q.insert(task("A"));
        q.add_subtask("A", task("A.1")).unwrap();
        q.add_subtask("A", task("A.2")).unwrap();

        // A has uncompleted subtasks, so it's not available
        let avail = q.next_available();
        let ids: Vec<&str> = avail.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&"A.1"));
        assert!(ids.contains(&"A.2"));
        assert!(!ids.contains(&"A")); // blocked by subtasks

        q.claim("A.1", "a1").unwrap();
        q.complete("A.1").unwrap();
        q.claim("A.2", "a2").unwrap();
        q.complete("A.2").unwrap();

        // Now A is available
        let avail = q.next_available();
        assert!(avail.iter().any(|t| t.id == "A"));
    }

    #[test]
    fn discovered_tasks_need_approval() {
        let mut q = TaskQueue::new();
        q.add_discovered(task("D1"));

        // Not available yet
        assert!(q.next_available().is_empty());

        q.approve_discovered("D1").unwrap();
        assert_eq!(q.next_available().len(), 1);
    }
}
