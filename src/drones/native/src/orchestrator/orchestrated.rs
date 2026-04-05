use std::path::Path;

/// Run a shell command and return (success, stdout, stderr).
async fn run_test_command(command: &str, workspace: &Path) -> (bool, String, String) {
    let output = tokio::process::Command::new("sh")
        .args(["-c", command])
        .current_dir(workspace)
        .output()
        .await;

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            (output.status.success(), stdout, stderr)
        }
        Err(e) => (false, String::new(), format!("failed to run command: {e}")),
    }
}

/// Truncate test output to the last `max_bytes` bytes to avoid blowing context.
fn truncate_output(output: &str, max_bytes: usize) -> &str {
    if output.len() <= max_bytes {
        output
    } else {
        let start = output.len() - max_bytes;
        // Find next char boundary to avoid splitting a multi-byte char
        &output[output.ceil_char_boundary(start)..]
    }
}

/// Build a structured summary of orchestrator results for the fix-up agent.
fn build_orchestrator_summary(results: &[super::TaskResult]) -> String {
    let mut summary = String::from("## Orchestrator Task Results\n\n");
    for result in results {
        let status = if result.success { "PASS" } else { "FAIL" };
        summary.push_str(&format!(
            "- **{}** [{}]: {}\n",
            result.task_id, status, result.output
        ));
        if !result.commits.is_empty() {
            summary.push_str(&format!("  Commits: {}\n", result.commits.join(", ")));
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_test_command_success() {
        let dir = tempfile::tempdir().unwrap();
        let (success, stdout, _stderr) = run_test_command("echo hello", dir.path()).await;
        assert!(success);
        assert_eq!(stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn run_test_command_failure() {
        let dir = tempfile::tempdir().unwrap();
        let (success, _stdout, _stderr) = run_test_command("false", dir.path()).await;
        assert!(!success);
    }

    #[tokio::test]
    async fn run_test_command_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let (success, _, stderr) =
            run_test_command("nonexistent_command_12345", dir.path()).await;
        assert!(!success);
        assert!(!stderr.is_empty());
    }

    #[test]
    fn truncate_output_short() {
        let output = "short";
        assert_eq!(truncate_output(output, 1000), "short");
    }

    #[test]
    fn truncate_output_long() {
        let output = "a".repeat(100);
        let truncated = truncate_output(&output, 50);
        assert_eq!(truncated.len(), 50);
    }

    #[test]
    fn build_orchestrator_summary_mixed() {
        let results = vec![
            super::super::TaskResult {
                task_id: "task-1".into(),
                success: true,
                output: "completed in 5 iterations".into(),
                commits: vec!["abc123".into()],
            },
            super::super::TaskResult {
                task_id: "task-2".into(),
                success: false,
                output: "task failed: compilation error".into(),
                commits: vec![],
            },
        ];
        let summary = build_orchestrator_summary(&results);
        assert!(summary.contains("task-1"));
        assert!(summary.contains("[PASS]"));
        assert!(summary.contains("task-2"));
        assert!(summary.contains("[FAIL]"));
        assert!(summary.contains("abc123"));
    }
}
