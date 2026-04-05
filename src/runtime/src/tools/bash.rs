use async_trait::async_trait;
use tokio::process::Command;

use super::file_ops::validate_path;
use super::registry::Tool;
use super::types::*;

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command in the workspace"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": { "type": "string", "description": "The bash command to execute" },
                "timeout": { "type": "integer", "description": "Timeout in milliseconds (default 120000)" },
                "working_dir": { "type": "string", "description": "Working directory (default: workspace)" }
            }
        })
    }

    fn permission(&self) -> PermissionLevel {
        PermissionLevel::FullAccess
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let command = match input["command"].as_str() {
            Some(c) => c,
            None => return ToolResult::error("missing required field: command".into()),
        };

        let timeout_ms = input["timeout"].as_u64().unwrap_or(120_000);
        let working_dir = match input["working_dir"].as_str() {
            Some(p) => match validate_path(&ctx.workspace, p) {
                Ok(path) => path,
                Err(e) => return ToolResult::error(e),
            },
            None => ctx.workspace.clone(),
        };

        let child = Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&working_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let child = match child {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("failed to spawn process: {e}")),
        };

        let timeout_duration = std::time::Duration::from_millis(timeout_ms);
        let result = tokio::time::timeout(timeout_duration, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);

                let mut text = String::new();
                if !stdout.is_empty() {
                    text.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str("stderr:\n");
                    text.push_str(&stderr);
                }
                if text.is_empty() {
                    text.push_str("(no output)");
                }

                if exit_code != 0 {
                    text.push_str(&format!("\n\nexit code: {exit_code}"));
                    ToolResult::error(text)
                } else {
                    ToolResult::success(text)
                }
            }
            Ok(Err(e)) => ToolResult::error(format!("process error: {e}")),
            Err(_) => ToolResult::error(format!("command timed out after {timeout_ms}ms")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::NullEventSink;
    use crate::tools::ToolRegistry;
    use std::sync::Arc;

    fn test_ctx() -> ToolContext {
        ToolContext {
            workspace: std::env::temp_dir(),
            home: std::env::temp_dir(),
            event_sink: Arc::new(NullEventSink),
            tool_registry: Arc::new(ToolRegistry::new()),
            agent_depth: 0,
        }
    }

    fn test_ctx_in(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            workspace: dir.to_path_buf(),
            home: dir.to_path_buf(),
            event_sink: Arc::new(NullEventSink),
            tool_registry: Arc::new(ToolRegistry::new()),
            agent_depth: 0,
        }
    }

    #[tokio::test]
    async fn test_simple_command() {
        let ctx = test_ctx();
        let result = BashTool
            .execute(serde_json::json!({"command": "echo hello"}), &ctx)
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("hello"));
    }

    #[tokio::test]
    async fn test_non_zero_exit() {
        let ctx = test_ctx();
        let result = BashTool
            .execute(serde_json::json!({"command": "exit 42"}), &ctx)
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("exit code: 42"));
    }

    #[tokio::test]
    async fn test_timeout() {
        let ctx = test_ctx();
        let result = BashTool
            .execute(
                serde_json::json!({"command": "sleep 30", "timeout": 100}),
                &ctx,
            )
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("timed out"));
    }

    #[tokio::test]
    async fn test_working_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let sub = dir.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();
        let ctx = test_ctx_in(dir.path());
        let result = BashTool
            .execute(
                serde_json::json!({"command": "pwd", "working_dir": sub.to_str().unwrap()}),
                &ctx,
            )
            .await;
        assert!(!result.is_error);
        assert!(result.output.contains("subdir"));
    }

    #[tokio::test]
    async fn test_working_dir_outside_workspace() {
        let ctx = test_ctx();
        let result = BashTool
            .execute(
                serde_json::json!({"command": "pwd", "working_dir": "/etc"}),
                &ctx,
            )
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("escapes workspace"));
    }
}
