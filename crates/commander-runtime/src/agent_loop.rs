use crate::adapter::{AdapterError, LlmAdapter, LlmRequest, LlmResponse, StopReason};
use crate::observer::LoopObserver;
use commander_hooks::{HookEvent, HookResult, HookRunner};
use commander_messages::{ContentBlock, Message, Role, TranscriptWriter};
use commander_permissions::{PermissionDecision, PermissionEngine};
use commander_tools::batch::{execute_batch, plan_batches, PendingToolCall};
use commander_tools::registry::ToolRegistry;
use commander_tools::tool::ToolContext;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Error)]
pub enum LoopError {
    #[error("adapter error: {0}")]
    Adapter(#[from] AdapterError),
    #[error("transcript error: {0}")]
    Transcript(String),
    #[error("cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOutcome {
    EndTurn,
    MaxTurns,
    Cancelled,
}

pub struct AgentLoopConfig {
    pub max_turns: u32,
    pub cwd: PathBuf,
    pub session_id: String,
    pub env: HashMap<String, String>,
    pub system_prompt: Option<String>,
    pub max_tokens: u32,
    pub checkpoint_path: Option<PathBuf>,
}

/// The core agent loop. Each iteration = one LLM call + tool execution.
pub async fn run_agent_loop(
    config: AgentLoopConfig,
    adapter: &dyn LlmAdapter,
    registry: &ToolRegistry,
    permissions: &PermissionEngine,
    hooks: &dyn HookRunner,
    observer: &dyn LoopObserver,
    transcript: &mut TranscriptWriter,
    messages: &mut Vec<Message>,
    cancel: CancellationToken,
    path_guard: Option<Arc<dyn commander_tools::PathGuard>>,
) -> Result<SessionOutcome, LoopError> {
    let tool_schemas: Vec<Value> = registry
        .schemas()
        .into_iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "description": s.description,
                "input_schema": s.input_schema
            })
        })
        .collect();

    for turn in 0..config.max_turns {
        if cancel.is_cancelled() {
            return Ok(SessionOutcome::Cancelled);
        }

        tracing::debug!(turn, "starting turn");

        // 1. Call LLM
        let request = LlmRequest {
            messages: messages.clone(),
            system_prompt: config.system_prompt.clone(),
            tools: tool_schemas.clone(),
            max_tokens: config.max_tokens,
        };

        let response: LlmResponse = adapter.complete(request).await?;

        // 2. Build assistant message
        let assistant_msg = Message {
            id: uuid::Uuid::new_v4(),
            role: Role::Assistant,
            content: response.content.clone(),
            usage: Some(response.usage),
            timestamp: chrono::Utc::now(),
            metadata: Value::Null,
        };
        messages.push(assistant_msg.clone());
        transcript
            .append(&assistant_msg)
            .await
            .map_err(|e| LoopError::Transcript(e.to_string()))?;
        save_checkpoint(config.checkpoint_path.as_deref(), messages)?;

        observer.on_assistant_message(&assistant_msg).await;

        // 3. Check stop condition
        if response.stop_reason == StopReason::EndTurn {
            return Ok(SessionOutcome::EndTurn);
        }

        // 4. Extract tool calls
        let tool_calls: Vec<PendingToolCall> = response
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, name, input } => Some(PendingToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                }),
                _ => None,
            })
            .collect();

        if tool_calls.is_empty() {
            return Ok(SessionOutcome::EndTurn);
        }

        // 5. Plan batches and execute
        let batches = plan_batches(&tool_calls, registry);
        let mut all_result_blocks = Vec::new();

        for batch in &batches {
            for call in &batch.calls {
                // Permission check
                let decision = permissions.check(&call.name);
                match decision {
                    PermissionDecision::Allow => {}
                    PermissionDecision::Deny(reason) => {
                        all_result_blocks.push(ContentBlock::ToolResult {
                            tool_use_id: call.id.clone(),
                            content: format!("Permission denied: {reason}"),
                            is_error: true,
                        });
                        continue;
                    }
                    PermissionDecision::Ask(prompt) => {
                        let approved = observer.on_permission_ask(&call.name, &prompt).await;
                        if !approved {
                            all_result_blocks.push(ContentBlock::ToolResult {
                                tool_use_id: call.id.clone(),
                                content: "Permission denied by user".into(),
                                is_error: true,
                            });
                            continue;
                        }
                    }
                }

                // Pre-tool hook
                let hook_event = HookEvent::PreToolUse {
                    tool: call.name.clone(),
                    input: call.input.clone(),
                };
                if let HookResult::Deny { reason } = hooks.run(&hook_event).await {
                    all_result_blocks.push(ContentBlock::ToolResult {
                        tool_use_id: call.id.clone(),
                        content: format!("Blocked by hook: {reason}"),
                        is_error: true,
                    });
                    continue;
                }
            }

            // Execute the batch
            let ctx = ToolContext {
                cwd: config.cwd.clone(),
                session_id: config.session_id.clone(),
                cancel: cancel.clone(),
                env: config.env.clone(),
                path_guard: path_guard.clone(),
            };

            let results = execute_batch(batch, registry, &ctx).await;

            for result in results {
                let (content, is_error) = match result.output {
                    Ok(output) => {
                        // Post-tool hook
                        let hook_event = HookEvent::PostToolUse {
                            tool: result.name.clone(),
                            output: output.content.clone(),
                        };
                        hooks.run(&hook_event).await;
                        observer.on_tool_complete(&result.name, &output).await;

                        let text = match &output.content {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        (text, output.is_error)
                    }
                    Err(e) => (e.to_string(), true),
                };

                all_result_blocks.push(ContentBlock::ToolResult {
                    tool_use_id: result.id,
                    content,
                    is_error,
                });
            }
        }

        // 6. Append tool results as a user message
        let result_msg = Message {
            id: uuid::Uuid::new_v4(),
            role: Role::User,
            content: all_result_blocks,
            usage: None,
            timestamp: chrono::Utc::now(),
            metadata: Value::Null,
        };
        messages.push(result_msg.clone());
        transcript
            .append(&result_msg)
            .await
            .map_err(|e| LoopError::Transcript(e.to_string()))?;
        save_checkpoint(config.checkpoint_path.as_deref(), messages)?;
    }

    Ok(SessionOutcome::MaxTurns)
}

fn save_checkpoint(path: Option<&std::path::Path>, messages: &[Message]) -> Result<(), LoopError> {
    let Some(path) = path else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| LoopError::Transcript(e.to_string()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let content =
        serde_json::to_string(messages).map_err(|e| LoopError::Transcript(e.to_string()))?;
    std::fs::write(&tmp, content).map_err(|e| LoopError::Transcript(e.to_string()))?;
    std::fs::rename(&tmp, path).map_err(|e| LoopError::Transcript(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::*;
    use crate::observer::AutoApproveObserver;
    use commander_hooks::NoopHookRunner;
    use commander_messages::TokenUsage;
    use commander_permissions::PermissionMode;
    use commander_tools::builtin;
    use tempfile::TempDir;

    /// Mock adapter that returns a Read tool call on the first turn,
    /// then ends the conversation.
    struct MockAdapter {
        turn: std::sync::atomic::AtomicU32,
        file_to_read: String,
    }

    #[async_trait::async_trait]
    impl LlmAdapter for MockAdapter {
        async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, AdapterError> {
            let turn = self.turn.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if turn == 0 {
                Ok(LlmResponse {
                    content: vec![
                        ContentBlock::text("Let me read that file."),
                        ContentBlock::ToolUse {
                            id: "tu_1".into(),
                            name: "Read".into(),
                            input: serde_json::json!({"file_path": self.file_to_read}),
                        },
                    ],
                    usage: TokenUsage::default(),
                    stop_reason: StopReason::ToolUse,
                })
            } else {
                Ok(LlmResponse {
                    content: vec![ContentBlock::text("I read the file.")],
                    usage: TokenUsage::default(),
                    stop_reason: StopReason::EndTurn,
                })
            }
        }

        fn model_id(&self) -> &str {
            "mock"
        }
        fn context_window(&self) -> u32 {
            128_000
        }
        fn max_output_tokens(&self) -> u32 {
            4096
        }
    }

    #[tokio::test]
    async fn agent_loop_reads_file_and_ends() {
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("hello.txt");
        std::fs::write(&test_file, "Hello from test!").unwrap();

        let transcript_path = dir.path().join("transcript.jsonl");
        let mut transcript = TranscriptWriter::open(&transcript_path).await.unwrap();

        let mut registry = ToolRegistry::new();
        builtin::register_builtins(&mut registry);

        let permissions = PermissionEngine::new(PermissionMode::AutoApprove);
        let hooks = NoopHookRunner;
        let observer = AutoApproveObserver;
        let cancel = CancellationToken::new();

        let adapter = MockAdapter {
            turn: std::sync::atomic::AtomicU32::new(0),
            file_to_read: test_file.to_str().unwrap().to_string(),
        };

        let config = AgentLoopConfig {
            max_turns: 10,
            cwd: dir.path().to_path_buf(),
            session_id: "test-session".into(),
            env: HashMap::new(),
            system_prompt: Some("You are a test agent.".into()),
            max_tokens: 4096,
            checkpoint_path: None,
        };

        let mut messages = vec![Message::user("Read hello.txt")];

        let outcome = run_agent_loop(
            config,
            &adapter,
            &registry,
            &permissions,
            &hooks,
            &observer,
            &mut transcript,
            &mut messages,
            cancel,
            None,
        )
        .await
        .unwrap();

        assert_eq!(outcome, SessionOutcome::EndTurn);

        // 1 user + (assistant with tool call) + (user with tool result) + (assistant end turn)
        assert_eq!(messages.len(), 4);

        // Verify tool result contains file content
        let tool_result_msg = &messages[2];
        assert_eq!(tool_result_msg.role, Role::User);
        match &tool_result_msg.content[0] {
            ContentBlock::ToolResult {
                content, is_error, ..
            } => {
                assert!(!is_error);
                assert!(content.contains("Hello from test!"));
            }
            _ => panic!("expected ToolResult"),
        }

        // Verify transcript has all messages
        drop(transcript);
        let loaded = commander_messages::TranscriptReader::new(&transcript_path)
            .load()
            .await
            .unwrap();
        // Transcript has 3 messages (assistant + tool results + assistant end), not the initial user msg
        assert_eq!(loaded.len(), 3);
    }
}
