use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use super::registry::Tool;
use super::types::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalToolConfig {
    pub name: String,
    pub description: String,
    pub binary: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub input_schema: serde_json::Value,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub output_format: ExternalOutputFormat,
}

fn default_timeout() -> u64 {
    30_000
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum ExternalOutputFormat {
    #[default]
    Json,
    Markdown,
    Raw,
}

/// Response expected from external tool on stdout
#[derive(Debug, Deserialize)]
struct ExternalToolResponse {
    output: String,
    #[serde(default)]
    is_error: bool,
}

pub struct ExternalTool {
    config: ExternalToolConfig,
}

impl ExternalTool {
    pub fn new(config: ExternalToolConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ExternalTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        &self.config.description
    }

    fn input_schema(&self) -> serde_json::Value {
        if self.config.input_schema.is_null() {
            serde_json::json!({"type": "object"})
        } else {
            self.config.input_schema.clone()
        }
    }

    fn permission(&self) -> PermissionLevel {
        PermissionLevel::FullAccess
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let input_json = match serde_json::to_string(&input) {
            Ok(j) => j,
            Err(e) => return ToolResult::error(format!("failed to serialize input: {e}")),
        };

        let child = Command::new(&self.config.binary)
            .args(&self.config.args)
            .envs(self.config.env.iter())
            .current_dir(&ctx.workspace)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(format!(
                    "failed to spawn external tool '{}': {e}",
                    self.config.binary
                ));
            }
        };

        // Write input to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(input_json.as_bytes()).await;
            let _ = stdin.flush().await;
            drop(stdin);
        }

        let timeout = std::time::Duration::from_millis(self.config.timeout_ms);
        let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);

                if exit_code != 0 {
                    let mut msg = format!("external tool exited with code {exit_code}");
                    if !stderr.is_empty() {
                        msg.push_str(&format!("\nstderr: {stderr}"));
                    }
                    return ToolResult::error(msg);
                }

                // Try to parse JSON response
                match serde_json::from_str::<ExternalToolResponse>(&stdout) {
                    Ok(resp) => {
                        let output_text = match self.config.output_format {
                            ExternalOutputFormat::Json => resp.output,
                            ExternalOutputFormat::Markdown => resp.output,
                            ExternalOutputFormat::Raw => resp.output,
                        };
                        if resp.is_error {
                            ToolResult::error(output_text)
                        } else {
                            ToolResult::success(output_text)
                        }
                    }
                    Err(_) => {
                        // Fallback: treat raw stdout as output
                        ToolResult::success(stdout.to_string())
                    }
                }
            }
            Ok(Err(e)) => ToolResult::error(format!("external tool process error: {e}")),
            Err(_) => ToolResult::error(format!(
                "external tool timed out after {}ms",
                self.config.timeout_ms
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::NullEventSink;
    use std::sync::Arc;

    fn test_ctx() -> ToolContext {
        ToolContext {
            workspace: std::env::temp_dir(),
            home: std::env::temp_dir(),
            event_sink: Arc::new(NullEventSink),
        }
    }

    #[test]
    fn test_external_tool_config_deserialize() {
        let json = serde_json::json!({
            "name": "my_tool",
            "description": "A custom tool",
            "binary": "/usr/local/bin/my-tool",
            "args": ["--json"],
            "timeout_ms": 5000
        });
        let config: ExternalToolConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.name, "my_tool");
        assert_eq!(config.timeout_ms, 5000);
    }

    #[test]
    fn test_external_tool_config_defaults() {
        let json = serde_json::json!({
            "name": "minimal",
            "description": "Minimal tool",
            "binary": "/bin/true"
        });
        let config: ExternalToolConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.timeout_ms, 30_000);
        assert!(config.args.is_empty());
    }

    #[test]
    fn test_json_protocol_roundtrip() {
        let input = serde_json::json!({"query": "test"});
        let serialized = serde_json::to_string(&input).unwrap();
        let _deserialized: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    }

    #[test]
    fn test_response_parsing() {
        let json = r#"{"output": "hello world", "is_error": false}"#;
        let resp: ExternalToolResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.output, "hello world");
        assert!(!resp.is_error);
    }

    #[test]
    fn test_error_response_parsing() {
        let json = r#"{"output": "something went wrong", "is_error": true}"#;
        let resp: ExternalToolResponse = serde_json::from_str(json).unwrap();
        assert!(resp.is_error);
    }

    #[tokio::test]
    async fn test_external_tool_echo() {
        let config = ExternalToolConfig {
            name: "echo_tool".into(),
            description: "Echo test".into(),
            binary: "bash".into(),
            args: vec![
                "-c".into(),
                r#"cat | jq -c '{output: .query, is_error: false}'"#.into(),
            ],
            env: Default::default(),
            input_schema: serde_json::json!({"type": "object"}),
            timeout_ms: 5000,
            output_format: ExternalOutputFormat::Json,
        };

        // Only run if jq is available
        if Command::new("which")
            .arg("jq")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            let tool = ExternalTool::new(config);
            let ctx = test_ctx();
            let result = tool
                .execute(serde_json::json!({"query": "hello"}), &ctx)
                .await;
            assert!(!result.is_error, "output: {}", result.output);
            assert!(result.output.contains("hello"));
        }
    }

    #[tokio::test]
    async fn test_external_tool_timeout() {
        let config = ExternalToolConfig {
            name: "slow_tool".into(),
            description: "Slow tool".into(),
            binary: "sleep".into(),
            args: vec!["30".into()],
            env: Default::default(),
            input_schema: serde_json::json!({"type": "object"}),
            timeout_ms: 100,
            output_format: ExternalOutputFormat::Raw,
        };

        let tool = ExternalTool::new(config);
        let ctx = test_ctx();
        let result = tool.execute(serde_json::json!({}), &ctx).await;
        assert!(result.is_error);
        assert!(result.output.contains("timed out"));
    }

    #[tokio::test]
    async fn test_external_tool_nonzero_exit() {
        let config = ExternalToolConfig {
            name: "fail_tool".into(),
            description: "Failing tool".into(),
            binary: "bash".into(),
            args: vec!["-c".into(), "exit 1".into()],
            env: Default::default(),
            input_schema: serde_json::json!({"type": "object"}),
            timeout_ms: 5000,
            output_format: ExternalOutputFormat::Raw,
        };

        let tool = ExternalTool::new(config);
        let ctx = test_ctx();
        let result = tool.execute(serde_json::json!({}), &ctx).await;
        assert!(result.is_error);
        assert!(result.output.contains("exited with code 1"));
    }

    #[tokio::test]
    async fn test_external_tool_raw_fallback() {
        let config = ExternalToolConfig {
            name: "raw_tool".into(),
            description: "Raw output tool".into(),
            binary: "echo".into(),
            args: vec!["raw output text".into()],
            env: Default::default(),
            input_schema: serde_json::json!({"type": "object"}),
            timeout_ms: 5000,
            output_format: ExternalOutputFormat::Raw,
        };

        let tool = ExternalTool::new(config);
        let ctx = test_ctx();
        let result = tool.execute(serde_json::json!({}), &ctx).await;
        assert!(!result.is_error);
        assert!(result.output.contains("raw output text"));
    }
}
