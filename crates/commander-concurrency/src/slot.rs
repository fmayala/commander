use crate::key::derive_key;
use crate::policy::ConcurrencyPolicy;
use commander_tasks::task::Task;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SlotError {
    #[error("at capacity for group {key}: {active}/{max}")]
    AtCapacity { key: String, active: u32, max: u32 },
}

/// Tracks active runs per derived group key.
///
/// Acquire/release are keyed by the group key (not task ID).
/// The management loop stores a task_id -> group_key mapping separately.
pub struct SlotManager {
    active: HashMap<String, u32>,
    policies: Vec<ConcurrencyPolicy>,
}

impl SlotManager {
    pub fn new(policies: Vec<ConcurrencyPolicy>) -> Self {
        Self {
            active: HashMap::new(),
            policies,
        }
    }

    /// Check if a task can run under all configured policies.
    pub fn can_run(&self, task: &Task) -> bool {
        for policy in &self.policies {
            let key = derive_key(task, &policy.key_expr);
            let current = self.active.get(&key).copied().unwrap_or(0);
            if current >= policy.max_runs {
                return false;
            }
        }
        true
    }

    /// Acquire a slot for the given group key.
    pub fn acquire(&mut self, key: &str) -> Result<(), SlotError> {
        // Check against all policies that match this key
        for policy in &self.policies {
            // For the "global" policy, all keys match
            if policy.key_expr == "global" || key == derive_key_static(&policy.key_expr, key) {
                let current = self.active.get(key).copied().unwrap_or(0);
                if current >= policy.max_runs {
                    return Err(SlotError::AtCapacity {
                        key: key.into(),
                        active: current,
                        max: policy.max_runs,
                    });
                }
            }
        }

        *self.active.entry(key.into()).or_insert(0) += 1;
        Ok(())
    }

    /// Release a slot for the given group key.
    pub fn release(&mut self, key: &str) {
        if let Some(count) = self.active.get_mut(key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.active.remove(key);
            }
        }
    }

    /// Filter tasks that can run under current concurrency policies.
    pub fn filter_allowed<'a>(&self, tasks: &'a [Task]) -> Vec<&'a Task> {
        tasks.iter().filter(|t| self.can_run(t)).collect()
    }

    /// Current active count for a key.
    pub fn active_count(&self, key: &str) -> u32 {
        self.active.get(key).copied().unwrap_or(0)
    }

    /// Derive the group key for a task against all policies.
    /// Returns the first policy's derived key (for the management loop's active_keys map).
    pub fn derive_key(&self, task: &Task) -> String {
        self.policies
            .first()
            .map(|p| derive_key(task, &p.key_expr))
            .unwrap_or_else(|| "_global".into())
    }
}

/// Helper: for policies, the key is already derived from the task.
fn derive_key_static(key_expr: &str, derived_key: &str) -> String {
    if key_expr == "global" {
        "_global".into()
    } else {
        derived_key.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::ConcurrencyPolicy;
    use commander_tasks::task::Task;

    fn task(id: &str, project: &str) -> Task {
        Task::new(id, project, format!("Task {id}"))
    }

    #[test]
    fn slot_acquire_release() {
        let policies = vec![ConcurrencyPolicy {
            key_expr: "project_id".into(),
            max_runs: 2,
            strategy: Default::default(),
        }];
        let mut mgr = SlotManager::new(policies);

        let t1 = task("1", "proj-a");
        let _t2 = task("2", "proj-a");
        let t3 = task("3", "proj-a");

        assert!(mgr.can_run(&t1));
        mgr.acquire("proj-a").unwrap();
        mgr.acquire("proj-a").unwrap();

        // At capacity
        assert!(!mgr.can_run(&t3));

        // Different project still allowed
        let t4 = task("4", "proj-b");
        assert!(mgr.can_run(&t4));

        // Release one
        mgr.release("proj-a");
        assert!(mgr.can_run(&t3));
    }

    #[test]
    fn filter_allowed() {
        let policies = vec![ConcurrencyPolicy {
            key_expr: "project_id".into(),
            max_runs: 1,
            strategy: Default::default(),
        }];
        let mut mgr = SlotManager::new(policies);

        let tasks = vec![
            task("1", "proj-a"),
            task("2", "proj-a"),
            task("3", "proj-b"),
        ];

        // Before any acquisition, all pass (can_run checks active, not pending)
        let allowed = mgr.filter_allowed(&tasks);
        assert_eq!(allowed.len(), 3);

        // Acquire one for proj-a
        mgr.acquire("proj-a").unwrap();

        // Now proj-a tasks are blocked, proj-b still allowed
        let allowed = mgr.filter_allowed(&tasks);
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0].project_id, "proj-b");
    }
}
