use async_trait::async_trait;
use tokio::process::Command;

use super::registry::Tool;
use super::types::*;

pub struct GitTool;

#[derive(Debug)]
enum GitOperation {
    Status,
    Diff { cached: bool },
    Log { count: u32 },
    CreateBranch { name: String },
    Commit { message: String },
    Push { remote: String, branch: String },
    CreatePr { title: String, body: String },
    CheckoutFile { path: String },
}

fn parse_operation(input: &serde_json::Value) -> Result<GitOperation, String> {
    let op = input["operation"]
        .as_str()
        .ok_or("missing required field: operation")?;

    match op {
        "status" => Ok(GitOperation::Status),
        "diff" => Ok(GitOperation::Diff {
            cached: input["cached"].as_bool().unwrap_or(false),
        }),
        "log" => Ok(GitOperation::Log {
            count: input["count"].as_u64().unwrap_or(10) as u32,
        }),
        "create_branch" => {
            let name = input["name"]
                .as_str()
                .ok_or("create_branch requires 'name'")?;
            Ok(GitOperation::CreateBranch {
                name: name.to_string(),
            })
        }
        "commit" => {
            let message = input["message"]
                .as_str()
                .ok_or("commit requires 'message'")?;
            Ok(GitOperation::Commit {
                message: message.to_string(),
            })
        }
        "push" => Ok(GitOperation::Push {
            remote: input["remote"].as_str().unwrap_or("origin").to_string(),
            branch: input["branch"].as_str().unwrap_or("").to_string(),
        }),
        "create_pr" => {
            let title = input["title"]
                .as_str()
                .ok_or("create_pr requires 'title'")?;
            let body = input["body"].as_str().unwrap_or("").to_string();
            Ok(GitOperation::CreatePr {
                title: title.to_string(),
                body,
            })
        }
        "checkout_file" => {
            let path = input["path"]
                .as_str()
                .ok_or("checkout_file requires 'path'")?;
            Ok(GitOperation::CheckoutFile {
                path: path.to_string(),
            })
        }
        other => Err(format!("unknown git operation: {other}")),
    }
}

fn build_command(op: &GitOperation) -> (String, Vec<String>) {
    match op {
        GitOperation::Status => ("git".into(), vec!["status".into()]),
        GitOperation::Diff { cached } => {
            let mut args = vec!["diff".to_string()];
            if *cached {
                args.push("--cached".into());
            }
            ("git".into(), args)
        }
        GitOperation::Log { count } => (
            "git".into(),
            vec!["log".into(), "--oneline".into(), format!("-{count}")],
        ),
        GitOperation::CreateBranch { name } => (
            "git".into(),
            vec!["checkout".into(), "-b".into(), name.clone()],
        ),
        GitOperation::Commit { message } => (
            "git".into(),
            vec!["commit".into(), "-m".into(), message.clone()],
        ),
        GitOperation::Push { remote, branch } => {
            let mut args = vec!["push".into(), "-u".into(), remote.clone()];
            if !branch.is_empty() {
                args.push(branch.clone());
            }
            ("git".into(), args)
        }
        GitOperation::CreatePr { title, body } => {
            let mut args = vec![
                "pr".into(),
                "create".into(),
                "--title".into(),
                title.clone(),
            ];
            if !body.is_empty() {
                args.push("--body".into());
                args.push(body.clone());
            }
            ("gh".into(), args)
        }
        GitOperation::CheckoutFile { path } => (
            "git".into(),
            vec!["checkout".into(), "--".into(), path.clone()],
        ),
    }
}

#[async_trait]
impl Tool for GitTool {
    fn name(&self) -> &str {
        "git"
    }

    fn description(&self) -> &str {
        "Execute git operations in the workspace"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["operation"],
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["status", "diff", "log", "create_branch", "commit", "push", "create_pr", "checkout_file"],
                    "description": "The git operation to perform"
                },
                "name": { "type": "string", "description": "Branch name (for create_branch)" },
                "message": { "type": "string", "description": "Commit message (for commit)" },
                "cached": { "type": "boolean", "description": "Show staged changes (for diff)" },
                "count": { "type": "integer", "description": "Number of log entries (for log)" },
                "remote": { "type": "string", "description": "Remote name (for push, default: origin)" },
                "branch": { "type": "string", "description": "Branch name (for push)" },
                "title": { "type": "string", "description": "PR title (for create_pr)" },
                "body": { "type": "string", "description": "PR body (for create_pr)" },
                "path": { "type": "string", "description": "File path (for checkout_file)" }
            }
        })
    }

    fn permission(&self) -> PermissionLevel {
        PermissionLevel::FullAccess
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let op = match parse_operation(&input) {
            Ok(op) => op,
            Err(e) => return ToolResult::error(e),
        };

        let (cmd, args) = build_command(&op);

        let child = Command::new(&cmd)
            .args(&args)
            .current_dir(&ctx.workspace)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let child = match child {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("failed to spawn {cmd}: {e}")),
        };

        let timeout = std::time::Duration::from_secs(60);
        let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);

                let mut text = String::new();
                text.push_str(&format!("```\n$ {cmd} {}\n", args.join(" ")));

                if !stdout.is_empty() {
                    text.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    text.push_str(&stderr);
                }
                text.push_str("```\n");

                if exit_code != 0 {
                    ToolResult::error(text)
                } else {
                    ToolResult::success(text)
                }
            }
            Ok(Err(e)) => ToolResult::error(format!("process error: {e}")),
            Err(_) => ToolResult::error("git command timed out after 60s".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_status() {
        let input = serde_json::json!({"operation": "status"});
        let op = parse_operation(&input).unwrap();
        assert!(matches!(op, GitOperation::Status));
    }

    #[test]
    fn test_parse_diff_cached() {
        let input = serde_json::json!({"operation": "diff", "cached": true});
        let op = parse_operation(&input).unwrap();
        assert!(matches!(op, GitOperation::Diff { cached: true }));
    }

    #[test]
    fn test_parse_log_default_count() {
        let input = serde_json::json!({"operation": "log"});
        let op = parse_operation(&input).unwrap();
        assert!(matches!(op, GitOperation::Log { count: 10 }));
    }

    #[test]
    fn test_parse_create_branch() {
        let input = serde_json::json!({"operation": "create_branch", "name": "feat/test"});
        let op = parse_operation(&input).unwrap();
        match op {
            GitOperation::CreateBranch { name } => assert_eq!(name, "feat/test"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_parse_commit_requires_message() {
        let input = serde_json::json!({"operation": "commit"});
        assert!(parse_operation(&input).is_err());
    }

    #[test]
    fn test_parse_unknown_operation() {
        let input = serde_json::json!({"operation": "rebase"});
        assert!(parse_operation(&input).is_err());
    }

    #[test]
    fn test_build_status_command() {
        let op = GitOperation::Status;
        let (cmd, args) = build_command(&op);
        assert_eq!(cmd, "git");
        assert_eq!(args, vec!["status"]);
    }

    #[test]
    fn test_build_diff_cached_command() {
        let op = GitOperation::Diff { cached: true };
        let (cmd, args) = build_command(&op);
        assert_eq!(cmd, "git");
        assert_eq!(args, vec!["diff", "--cached"]);
    }

    #[test]
    fn test_build_log_command() {
        let op = GitOperation::Log { count: 5 };
        let (cmd, args) = build_command(&op);
        assert_eq!(cmd, "git");
        assert_eq!(args, vec!["log", "--oneline", "-5"]);
    }

    #[test]
    fn test_build_create_pr_command() {
        let op = GitOperation::CreatePr {
            title: "Add feature".into(),
            body: "Description".into(),
        };
        let (cmd, args) = build_command(&op);
        assert_eq!(cmd, "gh");
        assert!(args.contains(&"pr".to_string()));
        assert!(args.contains(&"create".to_string()));
        assert!(args.contains(&"Add feature".to_string()));
    }

    #[test]
    fn test_build_push_with_branch() {
        let op = GitOperation::Push {
            remote: "origin".into(),
            branch: "main".into(),
        };
        let (cmd, args) = build_command(&op);
        assert_eq!(cmd, "git");
        assert_eq!(args, vec!["push", "-u", "origin", "main"]);
    }
}
