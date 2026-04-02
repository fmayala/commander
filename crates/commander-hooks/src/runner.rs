use crate::event::HookEvent;
use crate::io::{HookEntry, HookInput, HookOutput};
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Result of running a hook.
#[derive(Debug, Clone)]
pub enum HookResult {
    /// No interference; continue normally.
    Continue,
    /// Block the action with a reason.
    Deny { reason: String },
    /// Replace the tool input with a modified value.
    ModifyInput(serde_json::Value),
    /// Inject context into the conversation.
    AddContext(String),
}

/// Trait for running hooks. Implementations can be swapped for testing.
#[async_trait]
pub trait HookRunner: Send + Sync {
    async fn run(&self, event: &HookEvent) -> HookResult;
}

/// Runs hooks as subprocesses, sending JSON on stdin and reading JSON from stdout.
pub struct SubprocessHookRunner {
    entries: Vec<HookEntry>,
    session_id: String,
}

impl SubprocessHookRunner {
    pub fn new(entries: Vec<HookEntry>, session_id: String) -> Self {
        Self {
            entries,
            session_id,
        }
    }

    fn matching_entries(&self, event: &HookEvent) -> Vec<&HookEntry> {
        let event_name = match event {
            HookEvent::PreToolUse { .. } => "pre_tool_use",
            HookEvent::PostToolUse { .. } => "post_tool_use",
            HookEvent::PostAssistantMessage => "post_assistant_message",
            HookEvent::PreLlmCall => "pre_llm_call",
            HookEvent::SessionStart { .. } => "session_start",
            HookEvent::SessionEnd { .. } => "session_end",
        };

        self.entries
            .iter()
            .filter(|e| e.event == event_name || e.event == "*")
            .collect()
    }

    async fn run_entry(&self, entry: &HookEntry, event: &HookEvent) -> HookResult {
        let input = HookInput {
            event: event.clone(),
            session_id: self.session_id.clone(),
        };

        let input_json = match serde_json::to_string(&input) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("failed to serialize hook input: {e}");
                return HookResult::Continue;
            }
        };

        let mut child = match Command::new("sh")
            .arg("-c")
            .arg(&entry.command)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(hook = ?entry.name, "failed to spawn hook: {e}");
                return HookResult::Continue;
            }
        };

        // Write input to stdin
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input_json.as_bytes()).await;
            let _ = stdin.shutdown().await;
        }

        // Wait with timeout
        let result = tokio::time::timeout(entry.timeout, child.wait_with_output()).await;
        let output = match result {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                tracing::error!(hook = ?entry.name, "hook process error: {e}");
                return HookResult::Continue;
            }
            Err(_) => {
                tracing::warn!(hook = ?entry.name, "hook timed out");
                // child is consumed by wait_with_output future, which was cancelled.
                // The drop of the future will clean up the child process.
                return HookResult::Continue;
            }
        };

        if !output.status.success() {
            tracing::warn!(
                hook = ?entry.name,
                code = output.status.code(),
                "hook exited with non-zero status"
            );
            return HookResult::Continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();
        if trimmed.is_empty() {
            return HookResult::Continue;
        }

        match serde_json::from_str::<HookOutput>(trimmed) {
            Ok(ho) => {
                if ho.block {
                    return HookResult::Deny {
                        reason: ho
                            .block_reason
                            .unwrap_or_else(|| "blocked by hook".into()),
                    };
                }
                if let Some(payload) = ho.mutated_payload {
                    return HookResult::ModifyInput(payload);
                }
                if let Some(msg) = ho.inject_messages.into_iter().next() {
                    return HookResult::AddContext(msg);
                }
                HookResult::Continue
            }
            Err(e) => {
                tracing::warn!(hook = ?entry.name, "failed to parse hook output: {e}");
                HookResult::Continue
            }
        }
    }
}

#[async_trait]
impl HookRunner for SubprocessHookRunner {
    async fn run(&self, event: &HookEvent) -> HookResult {
        let entries = self.matching_entries(event);
        for entry in entries {
            if entry.blocking {
                let result = self.run_entry(entry, event).await;
                match &result {
                    HookResult::Continue => continue,
                    _ => return result,
                }
            } else {
                // Non-blocking: fire and forget
                let entry = entry.clone();
                let event = event.clone();
                let session_id = self.session_id.clone();
                tokio::spawn(async move {
                    let runner = SubprocessHookRunner::new(vec![], session_id);
                    runner.run_entry(&entry, &event).await;
                });
            }
        }
        HookResult::Continue
    }
}

/// No-op hook runner for testing or when hooks are disabled.
pub struct NoopHookRunner;

#[async_trait]
impl HookRunner for NoopHookRunner {
    async fn run(&self, _event: &HookEvent) -> HookResult {
        HookResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::HookEntry;
    use std::time::Duration;

    #[tokio::test]
    async fn blocking_hook_denies() {
        let entry = HookEntry {
            event: "pre_tool_use".into(),
            command: r#"echo '{"block": true, "block_reason": "test deny"}'"#.into(),
            cwd: None,
            timeout: Duration::from_secs(5),
            blocking: true,
            name: Some("test-deny".into()),
        };

        let runner = SubprocessHookRunner::new(vec![entry], "test-session".into());
        let event = HookEvent::PreToolUse {
            tool: "Bash".into(),
            input: serde_json::json!({"command": "rm -rf /"}),
        };

        let result = runner.run(&event).await;
        match result {
            HookResult::Deny { reason } => assert_eq!(reason, "test deny"),
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn hook_returns_continue_on_empty_stdout() {
        let entry = HookEntry {
            event: "pre_tool_use".into(),
            command: "echo ''".into(),
            cwd: None,
            timeout: Duration::from_secs(5),
            blocking: true,
            name: Some("test-pass".into()),
        };

        let runner = SubprocessHookRunner::new(vec![entry], "test-session".into());
        let event = HookEvent::PreToolUse {
            tool: "Read".into(),
            input: serde_json::json!({}),
        };

        let result = runner.run(&event).await;
        assert!(matches!(result, HookResult::Continue));
    }

    #[tokio::test]
    async fn unmatched_event_returns_continue() {
        let entry = HookEntry {
            event: "session_start".into(),
            command: r#"echo '{"block": true}'"#.into(),
            cwd: None,
            timeout: Duration::from_secs(5),
            blocking: true,
            name: None,
        };

        let runner = SubprocessHookRunner::new(vec![entry], "test-session".into());
        let event = HookEvent::PreToolUse {
            tool: "Read".into(),
            input: serde_json::json!({}),
        };

        // Event is pre_tool_use but hook subscribes to session_start
        let result = runner.run(&event).await;
        assert!(matches!(result, HookResult::Continue));
    }

    #[tokio::test]
    async fn timeout_kills_hook() {
        let entry = HookEntry {
            event: "pre_tool_use".into(),
            command: "sleep 60".into(),
            cwd: None,
            timeout: Duration::from_millis(100),
            blocking: true,
            name: Some("slow-hook".into()),
        };

        let runner = SubprocessHookRunner::new(vec![entry], "test-session".into());
        let event = HookEvent::PreToolUse {
            tool: "Read".into(),
            input: serde_json::json!({}),
        };

        let result = runner.run(&event).await;
        assert!(matches!(result, HookResult::Continue));
    }
}
