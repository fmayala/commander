use crate::tool::*;
use async_trait::async_trait;
use serde_json::Value;

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn spec(&self) -> &ToolSpec {
        static SPEC: std::sync::OnceLock<ToolSpec> = std::sync::OnceLock::new();
        SPEC.get_or_init(|| ToolSpec {
            name: "Read".into(),
            description: "Read the contents of a file".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Absolute path to the file" },
                    "offset": { "type": "integer", "description": "Line number to start from (0-based)" },
                    "limit": { "type": "integer", "description": "Number of lines to read" }
                },
                "required": ["file_path"]
            }),
            concurrency: ConcurrencyClass::Concurrent,
        })
    }

    fn validate(&self, input: &Value) -> Result<(), ToolError> {
        if input.get("file_path").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::Validation("file_path is required".into()));
        }
        Ok(())
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let file_path = input["file_path"].as_str().unwrap();
        let path = if std::path::Path::new(file_path).is_absolute() {
            std::path::PathBuf::from(file_path)
        } else {
            ctx.cwd.join(file_path)
        };

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::Execution(format!("failed to read {}: {e}", path.display())))?;

        let offset = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        let lines: Vec<&str> = content.lines().collect();
        let end = limit.map_or(lines.len(), |l| (offset + l).min(lines.len()));
        let selected = &lines[offset.min(lines.len())..end];

        let numbered: String = selected
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{}\t{}", offset + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolOutput::success(Value::String(numbered)))
    }
}
