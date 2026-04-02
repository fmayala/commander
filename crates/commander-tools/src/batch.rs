use crate::registry::ToolRegistry;
use crate::tool::{ConcurrencyClass, Tool, ToolContext, ToolError, ToolOutput};
use serde_json::Value;
use std::sync::Arc;

/// A group of tool calls that can execute together.
#[derive(Debug)]
pub struct ToolBatch {
    pub calls: Vec<PendingToolCall>,
    pub is_concurrent: bool,
}

#[derive(Debug, Clone)]
pub struct PendingToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

pub struct BatchResult {
    pub id: String,
    pub name: String,
    pub output: Result<ToolOutput, ToolError>,
}

/// Partitions tool calls into batches based on concurrency class.
pub fn plan_batches(calls: &[PendingToolCall], registry: &ToolRegistry) -> Vec<ToolBatch> {
    if calls.is_empty() {
        return vec![];
    }

    let mut concurrent_batch: Vec<PendingToolCall> = Vec::new();
    let mut batches: Vec<ToolBatch> = Vec::new();

    for call in calls {
        let class = registry
            .get(&call.name)
            .map(|t| t.spec().concurrency)
            .unwrap_or(ConcurrencyClass::Serial);

        match class {
            ConcurrencyClass::Concurrent => {
                concurrent_batch.push(call.clone());
            }
            ConcurrencyClass::Serial => {
                // Flush any accumulated concurrent calls first
                if !concurrent_batch.is_empty() {
                    batches.push(ToolBatch {
                        calls: std::mem::take(&mut concurrent_batch),
                        is_concurrent: true,
                    });
                }
                batches.push(ToolBatch {
                    calls: vec![call.clone()],
                    is_concurrent: false,
                });
            }
        }
    }

    // Flush remaining concurrent calls
    if !concurrent_batch.is_empty() {
        batches.push(ToolBatch {
            calls: concurrent_batch,
            is_concurrent: true,
        });
    }

    batches
}

/// Execute a single batch of tool calls.
pub async fn execute_batch(
    batch: &ToolBatch,
    registry: &ToolRegistry,
    ctx: &ToolContext,
) -> Vec<BatchResult> {
    if batch.is_concurrent && batch.calls.len() > 1 {
        // Run all concurrent calls in parallel via JoinSet
        let mut set = tokio::task::JoinSet::new();
        for call in &batch.calls {
            let tool: Arc<dyn Tool> = match registry.get(&call.name) {
                Some(t) => Arc::clone(t),
                None => {
                    // Unknown tool: return error immediately, don't spawn
                    // We'll handle this outside the JoinSet
                    continue;
                }
            };
            let input = call.input.clone();
            let id = call.id.clone();
            let name = call.name.clone();
            let cwd = ctx.cwd.clone();
            let session_id = ctx.session_id.clone();
            let cancel = ctx.cancel.clone();
            let env = ctx.env.clone();
            let path_guard = ctx.path_guard.clone();

            set.spawn(async move {
                let ctx = ToolContext {
                    cwd,
                    session_id,
                    cancel,
                    env,
                    path_guard,
                };
                let output = match tool.validate(&input) {
                    Ok(()) => tool.call(input, &ctx).await,
                    Err(e) => Err(e),
                };
                BatchResult { id, name, output }
            });
        }

        let mut results = Vec::new();
        // Collect missing tools as errors
        for call in &batch.calls {
            if registry.get(&call.name).is_none() {
                results.push(BatchResult {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    output: Err(ToolError::Execution(format!("unknown tool: {}", call.name))),
                });
            }
        }
        while let Some(res) = set.join_next().await {
            match res {
                Ok(batch_result) => results.push(batch_result),
                Err(e) => {
                    tracing::error!("tool task panicked: {e}");
                }
            }
        }
        results
    } else {
        // Sequential execution
        let mut results = Vec::new();
        for call in &batch.calls {
            let output = match registry.get(&call.name) {
                Some(tool) => match tool.validate(&call.input) {
                    Ok(()) => tool.call(call.input.clone(), ctx).await,
                    Err(e) => Err(e),
                },
                None => Err(ToolError::Execution(format!("unknown tool: {}", call.name))),
            };
            results.push(BatchResult {
                id: call.id.clone(),
                name: call.name.clone(),
                output,
            });
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::*;
    use async_trait::async_trait;

    struct ConcurrentTool(String);
    struct SerialTool(String);

    #[async_trait]
    impl Tool for ConcurrentTool {
        fn spec(&self) -> &ToolSpec {
            Box::leak(Box::new(ToolSpec {
                name: self.0.clone(),
                description: String::new(),
                input_schema: serde_json::json!({}),
                concurrency: ConcurrencyClass::Concurrent,
            }))
        }
        fn validate(&self, _: &Value) -> Result<(), ToolError> {
            Ok(())
        }
        async fn call(&self, _: Value, _: &ToolContext) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(Value::String(self.0.clone())))
        }
    }

    #[async_trait]
    impl Tool for SerialTool {
        fn spec(&self) -> &ToolSpec {
            Box::leak(Box::new(ToolSpec {
                name: self.0.clone(),
                description: String::new(),
                input_schema: serde_json::json!({}),
                concurrency: ConcurrencyClass::Serial,
            }))
        }
        fn validate(&self, _: &Value) -> Result<(), ToolError> {
            Ok(())
        }
        async fn call(&self, _: Value, _: &ToolContext) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(Value::String(self.0.clone())))
        }
    }

    #[test]
    fn plans_concurrent_batch() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(ConcurrentTool("Read".into())));
        reg.register(Arc::new(ConcurrentTool("Glob".into())));

        let calls = vec![
            PendingToolCall {
                id: "1".into(),
                name: "Read".into(),
                input: Value::Null,
            },
            PendingToolCall {
                id: "2".into(),
                name: "Glob".into(),
                input: Value::Null,
            },
        ];

        let batches = plan_batches(&calls, &reg);
        assert_eq!(batches.len(), 1);
        assert!(batches[0].is_concurrent);
        assert_eq!(batches[0].calls.len(), 2);
    }

    #[test]
    fn serial_tools_get_own_batch() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(ConcurrentTool("Read".into())));
        reg.register(Arc::new(SerialTool("Write".into())));
        reg.register(Arc::new(ConcurrentTool("Glob".into())));

        let calls = vec![
            PendingToolCall {
                id: "1".into(),
                name: "Read".into(),
                input: Value::Null,
            },
            PendingToolCall {
                id: "2".into(),
                name: "Write".into(),
                input: Value::Null,
            },
            PendingToolCall {
                id: "3".into(),
                name: "Glob".into(),
                input: Value::Null,
            },
        ];

        let batches = plan_batches(&calls, &reg);
        // Read (concurrent) | Write (serial) | Glob (concurrent)
        assert_eq!(batches.len(), 3);
        assert!(batches[0].is_concurrent);
        assert!(!batches[1].is_concurrent);
        assert!(batches[2].is_concurrent);
    }
}
