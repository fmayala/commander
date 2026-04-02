use crate::handle::AgentHandle;
use async_trait::async_trait;
use std::path::Path;
use std::time::Instant;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpawnError {
    #[error("failed to spawn agent process: {0}")]
    SpawnFailed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Trait for spawning agent worker processes. Abstracted for testability.
#[async_trait]
pub trait ProcessSpawner: Send + Sync {
    async fn spawn(
        &self,
        agent_id: &str,
        task_id: &str,
        config_path: &Path,
    ) -> Result<AgentHandle, SpawnError>;
}

/// Real OS process spawner: launches `commander agent-worker`.
pub struct OsProcessSpawner {
    pub binary_path: String,
}

impl OsProcessSpawner {
    pub fn new(binary_path: impl Into<String>) -> Self {
        Self {
            binary_path: binary_path.into(),
        }
    }
}

#[async_trait]
impl ProcessSpawner for OsProcessSpawner {
    async fn spawn(
        &self,
        agent_id: &str,
        task_id: &str,
        config_path: &Path,
    ) -> Result<AgentHandle, SpawnError> {
        let child = tokio::process::Command::new(&self.binary_path)
            .arg("agent-worker")
            .arg("--id")
            .arg(agent_id)
            .arg("--config")
            .arg(config_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| SpawnError::SpawnFailed(e.to_string()))?;

        let pid = child.id().ok_or_else(|| {
            SpawnError::SpawnFailed("process exited immediately".into())
        })?;

        Ok(AgentHandle {
            agent_id: agent_id.into(),
            task_id: task_id.into(),
            pid,
            started_at: Instant::now(),
            last_activity: Instant::now(),
            restart_count: 0,
        })
    }
}

/// Mock spawner for testing. Always succeeds with a fake PID.
pub struct MockSpawner {
    next_pid: std::sync::atomic::AtomicU32,
    pub should_fail: std::sync::atomic::AtomicBool,
}

impl MockSpawner {
    pub fn new() -> Self {
        Self {
            next_pid: std::sync::atomic::AtomicU32::new(90000),
            should_fail: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl Default for MockSpawner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProcessSpawner for MockSpawner {
    async fn spawn(
        &self,
        agent_id: &str,
        task_id: &str,
        _config_path: &Path,
    ) -> Result<AgentHandle, SpawnError> {
        if self
            .should_fail
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(SpawnError::SpawnFailed("mock failure".into()));
        }

        let pid = self
            .next_pid
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(AgentHandle {
            agent_id: agent_id.into(),
            task_id: task_id.into(),
            pid,
            started_at: Instant::now(),
            last_activity: Instant::now(),
            restart_count: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn mock_spawner_succeeds() {
        let spawner = MockSpawner::new();
        let handle = spawner
            .spawn("agent-1", "task-1", &PathBuf::from("/tmp/config.json"))
            .await
            .unwrap();
        assert_eq!(handle.agent_id, "agent-1");
        assert_eq!(handle.task_id, "task-1");
        assert!(handle.pid >= 90000);
    }

    #[tokio::test]
    async fn mock_spawner_fails_when_configured() {
        let spawner = MockSpawner::new();
        spawner
            .should_fail
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let result = spawner
            .spawn("agent-1", "task-1", &PathBuf::from("/tmp/config.json"))
            .await;
        assert!(result.is_err());
    }
}
