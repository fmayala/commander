use crate::tool::*;
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn spec(&self) -> &ToolSpec {
        static SPEC: std::sync::OnceLock<ToolSpec> = std::sync::OnceLock::new();
        SPEC.get_or_init(|| ToolSpec {
            name: "Write".into(),
            description: "Write content to a file (creates or overwrites)".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Absolute path to write" },
                    "content": { "type": "string", "description": "Content to write" }
                },
                "required": ["file_path", "content"]
            }),
            concurrency: ConcurrencyClass::Serial,
        })
    }

    fn validate(&self, input: &Value) -> Result<(), ToolError> {
        if input.get("file_path").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::Validation("file_path is required".into()));
        }
        if input.get("content").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::Validation("content is required".into()));
        }
        Ok(())
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let file_path = input["file_path"].as_str().unwrap();
        let content = input["content"].as_str().unwrap();

        let path = if std::path::Path::new(file_path).is_absolute() {
            PathBuf::from(file_path)
        } else {
            ctx.cwd.join(file_path)
        };

        // PathGuard check: enforce boundary before any write
        if let Some(guard) = &ctx.path_guard {
            guard.check_write(&path).map_err(|e| ToolError::BoundaryViolation {
                path: e.path,
            })?;
        }

        // Create parent directories
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(&path, content).await?;

        Ok(ToolOutput::success(Value::String(format!(
            "Wrote {} bytes to {}",
            content.len(),
            path.display()
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_guard::{BoundaryViolation, PathGuard};
    use std::path::Path;
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
    async fn write_allowed_by_guard() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        let ctx = make_ctx(Some(Arc::new(AllowAllGuard)));

        let input = serde_json::json!({
            "file_path": path.to_str().unwrap(),
            "content": "hello"
        });

        let result = WriteTool.call(input, &ctx).await;
        assert!(result.is_ok());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    }

    #[tokio::test]
    async fn write_blocked_by_guard() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("blocked.txt");
        let ctx = make_ctx(Some(Arc::new(DenyAllGuard)));

        let input = serde_json::json!({
            "file_path": path.to_str().unwrap(),
            "content": "nope"
        });

        let result = WriteTool.call(input, &ctx).await;
        assert!(matches!(result, Err(ToolError::BoundaryViolation { .. })));
        // File must not exist on disk
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn write_no_guard_allows_all() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        let ctx = make_ctx(None);

        let input = serde_json::json!({
            "file_path": path.to_str().unwrap(),
            "content": "no guard"
        });

        let result = WriteTool.call(input, &ctx).await;
        assert!(result.is_ok());
    }
}
