use async_trait::async_trait;
use regex::Regex;
use tokio::process::Command;

use super::registry::Tool;
use super::types::*;

pub struct TestRunnerTool;

#[derive(Debug)]
pub struct TestResult {
    pub passed: u32,
    pub failed: u32,
    pub ignored: u32,
    pub failures: Vec<String>,
    pub raw_output: String,
}

fn parse_cargo_test_output(output: &str) -> Option<TestResult> {
    // Match: test result: ok. N passed; M failed; K ignored; ...
    // or:    test result: FAILED. N passed; M failed; K ignored; ...
    let summary_re =
        Regex::new(r"test result: (?:ok|FAILED)\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored")
            .ok()?;

    let caps = summary_re.captures(output)?;
    let passed: u32 = caps[1].parse().unwrap_or(0);
    let failed: u32 = caps[2].parse().unwrap_or(0);
    let ignored: u32 = caps[3].parse().unwrap_or(0);

    // Extract failure names: "test name ... FAILED"
    let failure_re = Regex::new(r"test (.+?) \.\.\. FAILED").ok()?;
    let failures: Vec<String> = failure_re
        .captures_iter(output)
        .map(|c| c[1].to_string())
        .collect();

    Some(TestResult {
        passed,
        failed,
        ignored,
        failures,
        raw_output: output.to_string(),
    })
}

fn format_test_result(result: &TestResult) -> String {
    let mut output = String::new();

    let status = if result.failed > 0 { "FAILED" } else { "ok" };
    output.push_str(&format!("## Test Result: {status}\n\n"));
    output.push_str(&format!(
        "- **Passed:** {}\n- **Failed:** {}\n- **Ignored:** {}\n",
        result.passed, result.failed, result.ignored
    ));

    if !result.failures.is_empty() {
        output.push_str("\n### Failures\n\n");
        for name in &result.failures {
            output.push_str(&format!("- `{name}`\n"));
        }
    }

    output
}

#[async_trait]
impl Tool for TestRunnerTool {
    fn name(&self) -> &str {
        "test_runner"
    }

    fn description(&self) -> &str {
        "Run tests and parse the output for structured results"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": { "type": "string", "description": "Test command to run (e.g. 'cargo test')" },
                "filter": { "type": "string", "description": "Test name filter pattern" },
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

        let mut cmd_str = command.to_string();
        if let Some(filter) = input["filter"].as_str() {
            cmd_str.push_str(&format!(" {filter}"));
        }

        let working_dir = input["working_dir"]
            .as_str()
            .map(|p| std::path::PathBuf::from(p))
            .unwrap_or_else(|| ctx.workspace.clone());

        let child = Command::new("bash")
            .arg("-c")
            .arg(&cmd_str)
            .current_dir(&working_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let child = match child {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("failed to spawn test command: {e}")),
        };

        let timeout = std::time::Duration::from_secs(300);
        let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{stdout}{stderr}");

                // Try to parse as cargo test output
                if let Some(test_result) = parse_cargo_test_output(&combined) {
                    let formatted = format_test_result(&test_result);
                    if test_result.failed > 0 {
                        ToolResult::error(formatted)
                    } else {
                        ToolResult::success(formatted)
                    }
                } else {
                    // Fallback: raw output
                    let exit_code = output.status.code().unwrap_or(-1);
                    let mut text = combined;
                    if exit_code != 0 {
                        text.push_str(&format!("\n\nexit code: {exit_code}"));
                        ToolResult::error(text.to_string())
                    } else {
                        ToolResult::success(text.to_string())
                    }
                }
            }
            Ok(Err(e)) => ToolResult::error(format!("test process error: {e}")),
            Err(_) => ToolResult::error("test command timed out after 300s".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARGO_TEST_SUCCESS: &str = "\
running 5 tests
test tests::test_one ... ok
test tests::test_two ... ok
test tests::test_three ... ok
test tests::test_four ... ok
test tests::test_five ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
";

    const CARGO_TEST_MIXED: &str = "\
running 4 tests
test tests::test_one ... ok
test tests::test_two ... FAILED
test tests::test_three ... ok
test tests::test_four ... FAILED

failures:

---- tests::test_two stdout ----
assertion failed: false

---- tests::test_four stdout ----
assertion failed: false

failures:
    tests::test_two
    tests::test_four

test result: FAILED. 2 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
";

    const CARGO_TEST_WITH_IGNORED: &str = "\
running 3 tests
test tests::test_one ... ok
test tests::test_two ... ignored
test tests::test_three ... FAILED

test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s
";

    #[test]
    fn test_parse_success() {
        let result = parse_cargo_test_output(CARGO_TEST_SUCCESS).unwrap();
        assert_eq!(result.passed, 5);
        assert_eq!(result.failed, 0);
        assert_eq!(result.ignored, 0);
        assert!(result.failures.is_empty());
    }

    #[test]
    fn test_parse_mixed() {
        let result = parse_cargo_test_output(CARGO_TEST_MIXED).unwrap();
        assert_eq!(result.passed, 2);
        assert_eq!(result.failed, 2);
        assert_eq!(result.ignored, 0);
        assert_eq!(result.failures.len(), 2);
        assert!(result.failures.contains(&"tests::test_two".to_string()));
        assert!(result.failures.contains(&"tests::test_four".to_string()));
    }

    #[test]
    fn test_parse_with_ignored() {
        let result = parse_cargo_test_output(CARGO_TEST_WITH_IGNORED).unwrap();
        assert_eq!(result.passed, 1);
        assert_eq!(result.failed, 1);
        assert_eq!(result.ignored, 1);
    }

    #[test]
    fn test_parse_unknown_format() {
        let result = parse_cargo_test_output("some random output");
        assert!(result.is_none());
    }

    #[test]
    fn test_format_success() {
        let result = TestResult {
            passed: 5,
            failed: 0,
            ignored: 0,
            failures: vec![],
            raw_output: String::new(),
        };
        let formatted = format_test_result(&result);
        assert!(formatted.contains("ok"));
        assert!(formatted.contains("Passed:** 5"));
    }

    #[test]
    fn test_format_failure() {
        let result = TestResult {
            passed: 2,
            failed: 1,
            ignored: 0,
            failures: vec!["tests::broken".into()],
            raw_output: String::new(),
        };
        let formatted = format_test_result(&result);
        assert!(formatted.contains("FAILED"));
        assert!(formatted.contains("tests::broken"));
    }
}
