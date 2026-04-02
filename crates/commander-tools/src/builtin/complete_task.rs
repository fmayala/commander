use crate::tool::*;
use async_trait::async_trait;
use serde_json::Value;

/// Orchestration completion marker.
/// The worker uses this tool call as the explicit "task done" signal.
pub struct CompleteTaskTool;

#[async_trait]
impl Tool for CompleteTaskTool {
    fn spec(&self) -> &ToolSpec {
        static SPEC: std::sync::OnceLock<ToolSpec> = std::sync::OnceLock::new();
        SPEC.get_or_init(|| ToolSpec {
            name: "complete_task".into(),
            description: "Declare task completion only after making required file changes and verification; include criteria evidence and summary".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "criteria_met": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "criterion": { "type": "string" },
                                "evidence": { "type": "string" }
                            },
                            "required": ["criterion", "evidence"]
                        }
                    },
                    "summary": { "type": "string" }
                },
                "required": ["summary", "criteria_met"]
            }),
            concurrency: ConcurrencyClass::Serial,
        })
    }

    fn validate(&self, input: &Value) -> Result<(), ToolError> {
        if input.get("summary").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::Validation("summary is required".into()));
        }
        let Some(criteria) = input.get("criteria_met") else {
            return Err(ToolError::Validation("criteria_met is required".into()));
        };
        let Some(arr) = criteria.as_array() else {
            return Err(ToolError::Validation(
                "criteria_met must be an array".into(),
            ));
        };
        for item in arr {
            if item.get("criterion").and_then(|v| v.as_str()).is_none() {
                return Err(ToolError::Validation(
                    "each criteria_met item requires criterion".into(),
                ));
            }
            if item.get("evidence").and_then(|v| v.as_str()).is_none() {
                return Err(ToolError::Validation(
                    "each criteria_met item requires evidence".into(),
                ));
            }
        }
        Ok(())
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        self.validate(&input)?;
        Ok(ToolOutput::success(Value::String(
            "complete_task acknowledged".into(),
        )))
    }
}
