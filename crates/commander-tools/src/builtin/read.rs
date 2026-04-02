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

        // PathGuard check: enforce boundary before any read
        if let Some(guard) = &ctx.path_guard {
            guard
                .check_read(&path)
                .map_err(|e| ToolError::BoundaryViolation { path: e.path })?;
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_guard::{BoundaryViolation, PathGuard};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    struct AllowAllGuard;
    impl PathGuard for AllowAllGuard {
        fn check_write(&self, _path: &Path) -> Result<(), BoundaryViolation> {
            Ok(())
        }
    }

    struct DenyAllGuard;
    impl PathGuard for DenyAllGuard {
        fn check_write(&self, path: &Path) -> Result<(), BoundaryViolation> {
            Err(BoundaryViolation {
                path: path.display().to_string(),
            })
        }
    }

    fn make_ctx(guard: Option<Arc<dyn PathGuard>>) -> ToolContext {
        ToolContext {
            cwd: PathBuf::from("/tmp"),
            session_id: "test".into(),
            cancel: CancellationToken::new(),
            env: Default::default(),
            path_guard: guard,
        }
    }

    #[tokio::test]
    async fn read_allowed_by_guard() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello\nworld\n").unwrap();
        let ctx = make_ctx(Some(Arc::new(AllowAllGuard)));

        let input = serde_json::json!({ "file_path": path.to_str().unwrap() });
        let result = ReadTool.call(input, &ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn read_blocked_by_guard() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("secret.txt");
        std::fs::write(&path, "top secret").unwrap();
        let ctx = make_ctx(Some(Arc::new(DenyAllGuard)));

        let input = serde_json::json!({ "file_path": path.to_str().unwrap() });
        let result = ReadTool.call(input, &ctx).await;
        assert!(matches!(result, Err(ToolError::BoundaryViolation { .. })));
    }

    #[tokio::test]
    async fn read_no_guard_allows_all() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "no guard").unwrap();
        let ctx = make_ctx(None);

        let input = serde_json::json!({ "file_path": path.to_str().unwrap() });
        let result = ReadTool.call(input, &ctx).await;
        assert!(result.is_ok());
    }
}
