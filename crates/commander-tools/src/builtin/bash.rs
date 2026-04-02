use crate::tool::*;
use async_trait::async_trait;
use serde_json::Value;
use tokio::process::Command;

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn spec(&self) -> &ToolSpec {
        static SPEC: std::sync::OnceLock<ToolSpec> = std::sync::OnceLock::new();
        SPEC.get_or_init(|| ToolSpec {
            name: "Bash".into(),
            description: "Execute a bash command and return its output".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The bash command to execute" },
                    "timeout": { "type": "integer", "description": "Timeout in milliseconds" }
                },
                "required": ["command"]
            }),
            concurrency: ConcurrencyClass::Serial,
        })
    }

    fn validate(&self, input: &Value) -> Result<(), ToolError> {
        if input.get("command").and_then(|v| v.as_str()).is_none() {
            return Err(ToolError::Validation("command is required".into()));
        }
        Ok(())
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let command = input["command"].as_str().unwrap();
        if is_disallowed_long_running(command) {
            return Ok(ToolOutput::error(
                "Long-running dev server commands are disabled for agent runs. Use build/test commands instead.",
            ));
        }
        let timeout_ms = input
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(120_000);
        let command_cwd = ctx
            .env
            .get("COMMANDER_TASK_CWD")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| ctx.cwd.clone());

        let mut child = Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&command_cwd)
            .envs(&ctx.env)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        // Take stdout/stderr before waiting so we retain the child handle for explicit
        // kill on timeout. Reading concurrently avoids pipe-buffer deadlock on large output.
        let mut stdout_reader = child.stdout.take().expect("stdout is piped");
        let mut stderr_reader = child.stderr.take().expect("stderr is piped");
        let stdout_task = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            stdout_reader.read_to_end(&mut buf).await.map(|_| buf)
        });
        let stderr_task = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            stderr_reader.read_to_end(&mut buf).await.map(|_| buf)
        });

        let timeout = std::time::Duration::from_millis(timeout_ms);
        // child.wait() takes &mut self, preserving child for explicit kill on timeout
        match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) => {
                let stdout_bytes = stdout_task.await.ok().and_then(|r| r.ok()).unwrap_or_default();
                let stderr_bytes = stderr_task.await.ok().and_then(|r| r.ok()).unwrap_or_default();
                let stdout = String::from_utf8_lossy(&stdout_bytes);
                let stderr = String::from_utf8_lossy(&stderr_bytes);
                let code = status.code().unwrap_or(-1);

                let combined = if stderr.is_empty() {
                    stdout.to_string()
                } else if stdout.is_empty() {
                    stderr.to_string()
                } else {
                    format!("{stdout}\n{stderr}")
                };

                if status.success() {
                    Ok(ToolOutput::success(Value::String(combined)))
                } else {
                    Ok(ToolOutput::error(format!("Exit code {code}\n{combined}")))
                }
            }
            Ok(Err(e)) => {
                stdout_task.abort();
                stderr_task.abort();
                Err(ToolError::Execution(format!("process error: {e}")))
            }
            Err(_) => {
                // Timeout — explicitly kill the child so it doesn't become an orphan
                let _ = child.start_kill();
                stdout_task.abort();
                stderr_task.abort();
                Ok(ToolOutput::error(format!(
                    "Command timed out after {timeout_ms}ms"
                )))
            }
        }
    }
}

fn is_disallowed_long_running(command: &str) -> bool {
    let c = command.to_ascii_lowercase();
    let normalized = c.split_whitespace().collect::<Vec<_>>().join(" ");
    normalized.contains("npm run dev")
        || normalized.contains("pnpm dev")
        || normalized.contains("yarn dev")
        || normalized.contains("npm start")
        || normalized == "vite"
        || normalized.starts_with("vite ")
        || normalized == "npx vite"
        || normalized.starts_with("npx vite ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use tokio_util::sync::CancellationToken;

    fn ctx() -> ToolContext {
        ToolContext {
            cwd: PathBuf::from("/tmp"),
            session_id: "test".into(),
            cancel: CancellationToken::new(),
            env: HashMap::new(),
            path_guard: None,
        }
    }

    #[tokio::test]
    async fn echo_command() {
        let input = serde_json::json!({"command": "echo hello"});
        let result = BashTool.call(input, &ctx()).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content.as_str().unwrap().trim(), "hello");
    }

    #[tokio::test]
    async fn failing_command() {
        let input = serde_json::json!({"command": "exit 1"});
        let result = BashTool.call(input, &ctx()).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.as_str().unwrap().contains("Exit code 1"));
    }

    #[tokio::test]
    async fn timeout_command() {
        let input = serde_json::json!({"command": "sleep 60", "timeout": 100});
        let result = BashTool.call(input, &ctx()).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.as_str().unwrap().contains("timed out"));
    }

    #[tokio::test]
    async fn timeout_kills_child_process() {
        // Verify the child bash process is explicitly killed on timeout.
        // "sleep 0.5 && touch file" requires bash to be alive after sleep exits.
        // If bash is killed at timeout, the touch never runs.
        let tmp = std::env::temp_dir()
            .join(format!("commander_kill_test_{}", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let cmd = format!("sleep 0.5 && touch {}", tmp.display());
        let input = serde_json::json!({"command": cmd, "timeout": 50});
        let result = BashTool.call(input, &ctx()).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.as_str().unwrap().contains("timed out"));
        // Wait past the sleep delay; if bash was killed the touch never runs
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        let file_exists = tmp.exists();
        let _ = std::fs::remove_file(&tmp);
        assert!(!file_exists, "child process was not killed: file was created after timeout");
    }

    #[tokio::test]
    async fn blocks_long_running_dev_server_commands() {
        let input = serde_json::json!({"command": "npm run dev"});
        let result = BashTool.call(input, &ctx()).await.unwrap();
        assert!(result.is_error);
        assert!(result
            .content
            .as_str()
            .unwrap()
            .contains("Long-running dev server commands are disabled"));
    }
}
