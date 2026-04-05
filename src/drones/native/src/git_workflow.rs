use std::path::PathBuf;

use tokio::sync::Mutex;

use crate::pipeline::{GitOperationKind, StageGitConfig};

pub struct GitWorkflow {
    config: StageGitConfig,
    protected_matchers: Vec<globset::GlobMatcher>,
    workspace: PathBuf,
    serializer: GitSerializer,
}

struct GitSerializer {
    lock: Mutex<()>,
}

#[derive(Debug, Clone)]
pub enum GitOperation {
    Status,
    Diff {
        staged: bool,
    },
    Log {
        count: u32,
    },
    CreateBranch {
        name: String,
        from: Option<String>,
    },
    Commit {
        message: String,
        paths: Vec<String>,
    },
    Push {
        force: bool,
    },
    CreatePr {
        title: String,
        body: String,
        base: Option<String>,
    },
    CheckoutFile {
        path: String,
        ref_: String,
    },
}

impl GitOperation {
    pub fn kind(&self) -> GitOperationKind {
        match self {
            GitOperation::Status => GitOperationKind::Status,
            GitOperation::Diff { .. } => GitOperationKind::Diff,
            GitOperation::Log { .. } => GitOperationKind::Log,
            GitOperation::CreateBranch { .. } => GitOperationKind::CreateBranch,
            GitOperation::Commit { .. } => GitOperationKind::Commit,
            GitOperation::Push { .. } => GitOperationKind::Push,
            GitOperation::CreatePr { .. } => GitOperationKind::CreatePr,
            GitOperation::CheckoutFile { .. } => GitOperationKind::CheckoutFile,
        }
    }
}

impl GitWorkflow {
    pub fn new(config: StageGitConfig, workspace: PathBuf) -> Self {
        let protected_matchers = config
            .protected_paths
            .iter()
            .filter_map(|p| globset::Glob::new(p).ok().map(|g| g.compile_matcher()))
            .collect();
        Self {
            config,
            protected_matchers,
            workspace,
            serializer: GitSerializer {
                lock: Mutex::new(()),
            },
        }
    }

    /// Validate and execute a git operation against the stage policy.
    pub async fn execute(&self, operation: &GitOperation) -> Result<String, GitWorkflowError> {
        // Check force push first — always denied regardless of allow-list
        if matches!(operation, GitOperation::Push { force: true }) {
            return Err(GitWorkflowError::ForcePushDenied);
        }

        // Check operation is allowed
        if let Some(allowed) = &self.config.allowed_operations {
            let kind = operation.kind();
            if !allowed.contains(&kind) {
                return Err(GitWorkflowError::OperationDenied {
                    operation: format!("{kind:?}"),
                });
            }
        }

        // Check specific policy rules
        match operation {
            GitOperation::Commit { paths, .. } => {
                for path in paths {
                    if self.is_protected(path) {
                        return Err(GitWorkflowError::ProtectedPath { path: path.clone() });
                    }
                }
            }
            GitOperation::CreateBranch { name, .. } => {
                if let Some(expected) = &self.config.branch_name {
                    if name != expected {
                        return Err(GitWorkflowError::BranchNameMismatch {
                            expected: expected.clone(),
                            got: name.clone(),
                        });
                    }
                }
            }
            _ => {}
        }

        // Execute via serializer (atomic commits)
        let _guard = self.serializer.lock.lock().await;
        self.run_git_command(operation).await
    }

    fn is_protected(&self, path: &str) -> bool {
        self.protected_matchers.iter().any(|m| m.is_match(path))
    }

    async fn run_git_command(&self, operation: &GitOperation) -> Result<String, GitWorkflowError> {
        match operation {
            GitOperation::Status => self.exec_git(&["status", "--porcelain"]).await,
            GitOperation::Diff { staged } => {
                if *staged {
                    self.exec_git(&["diff", "--staged"]).await
                } else {
                    self.exec_git(&["diff"]).await
                }
            }
            GitOperation::Log { count } => {
                self.exec_git(&["log", "--oneline", &format!("-{count}")])
                    .await
            }
            GitOperation::CreateBranch { name, from } => {
                let mut args = vec!["checkout", "-b", name.as_str()];
                if let Some(base) = from {
                    args.push(base.as_str());
                }
                self.exec_git(&args).await
            }
            GitOperation::Commit { message, paths } => {
                for path in paths {
                    self.exec_git(&["add", path.as_str()]).await?;
                }
                self.exec_git(&["commit", "-m", message.as_str()]).await
            }
            GitOperation::Push { force } => {
                let mut args = vec!["push"];
                if *force {
                    args.push("--force");
                }
                self.exec_git(&args).await
            }
            GitOperation::CreatePr { title, body, base } => {
                let mut args = vec![
                    "pr",
                    "create",
                    "--title",
                    title.as_str(),
                    "--body",
                    body.as_str(),
                ];
                if let Some(b) = base {
                    args.extend(["--base", b.as_str()]);
                }
                self.exec_gh(&args).await
            }
            GitOperation::CheckoutFile { path, ref_ } => {
                self.exec_git(&["checkout", ref_.as_str(), "--", path.as_str()])
                    .await
            }
        }
    }

    async fn exec_git(&self, args: &[&str]) -> Result<String, GitWorkflowError> {
        let output = tokio::process::Command::new("git")
            .args(args)
            .current_dir(&self.workspace)
            .output()
            .await
            .map_err(|e| GitWorkflowError::CommandFailed(e.to_string()))?;
        if !output.status.success() {
            return Err(GitWorkflowError::CommandFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn exec_gh(&self, args: &[&str]) -> Result<String, GitWorkflowError> {
        let output = tokio::process::Command::new("gh")
            .args(args)
            .current_dir(&self.workspace)
            .output()
            .await
            .map_err(|e| GitWorkflowError::CommandFailed(e.to_string()))?;
        if !output.status.success() {
            return Err(GitWorkflowError::CommandFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitWorkflowError {
    #[error("operation denied: {operation}")]
    OperationDenied { operation: String },
    #[error("force push is not allowed")]
    ForcePushDenied,
    #[error("cannot modify protected path: {path}")]
    ProtectedPath { path: String },
    #[error("branch name mismatch: expected {expected}, got {got}")]
    BranchNameMismatch { expected: String, got: String },
    #[error("git command failed: {0}")]
    CommandFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_only_config() -> StageGitConfig {
        StageGitConfig {
            branch_name: None,
            allowed_operations: Some(vec![
                GitOperationKind::Status,
                GitOperationKind::Diff,
                GitOperationKind::Log,
            ]),
            commit_on_checkpoint: false,
            commit_on_task_complete: false,
            pr_on_stage_complete: false,
            protected_paths: vec![],
        }
    }

    fn implement_config() -> StageGitConfig {
        StageGitConfig {
            branch_name: Some("feat/test-branch".into()),
            allowed_operations: None, // all allowed
            commit_on_checkpoint: true,
            commit_on_task_complete: true,
            pr_on_stage_complete: true,
            protected_paths: vec!["CLAUDE.md".into(), "*.lock".into()],
        }
    }

    fn no_git_config() -> StageGitConfig {
        StageGitConfig {
            branch_name: None,
            allowed_operations: Some(vec![]),
            commit_on_checkpoint: false,
            commit_on_task_complete: false,
            pr_on_stage_complete: false,
            protected_paths: vec![],
        }
    }

    #[tokio::test]
    async fn force_push_always_denied() {
        let wf = GitWorkflow::new(implement_config(), PathBuf::from("/tmp"));
        let result = wf.execute(&GitOperation::Push { force: true }).await;
        assert!(matches!(result, Err(GitWorkflowError::ForcePushDenied)));
    }

    #[tokio::test]
    async fn protected_path_blocked() {
        let wf = GitWorkflow::new(implement_config(), PathBuf::from("/tmp"));
        let result = wf
            .execute(&GitOperation::Commit {
                message: "test".into(),
                paths: vec!["CLAUDE.md".into()],
            })
            .await;
        assert!(matches!(
            result,
            Err(GitWorkflowError::ProtectedPath { .. })
        ));
    }

    #[tokio::test]
    async fn protected_path_glob_match() {
        let wf = GitWorkflow::new(implement_config(), PathBuf::from("/tmp"));
        let result = wf
            .execute(&GitOperation::Commit {
                message: "test".into(),
                paths: vec!["Cargo.lock".into()],
            })
            .await;
        assert!(matches!(
            result,
            Err(GitWorkflowError::ProtectedPath { .. })
        ));
    }

    #[tokio::test]
    async fn branch_name_enforced() {
        let wf = GitWorkflow::new(implement_config(), PathBuf::from("/tmp"));
        let result = wf
            .execute(&GitOperation::CreateBranch {
                name: "wrong-name".into(),
                from: None,
            })
            .await;
        assert!(matches!(
            result,
            Err(GitWorkflowError::BranchNameMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn branch_name_accepted_when_matching() {
        let wf = GitWorkflow::new(implement_config(), PathBuf::from("/tmp"));
        // This will fail at the git command level (no repo), but it should pass policy
        let result = wf
            .execute(&GitOperation::CreateBranch {
                name: "feat/test-branch".into(),
                from: None,
            })
            .await;
        // Should be CommandFailed (passed policy, failed at git level), not BranchNameMismatch
        assert!(matches!(result, Err(GitWorkflowError::CommandFailed(_))));
    }

    #[tokio::test]
    async fn read_only_allows_status() {
        let dir = tempfile::tempdir().unwrap();
        // Init a git repo so status works
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .await
            .unwrap();

        let wf = GitWorkflow::new(read_only_config(), dir.path().to_path_buf());
        let result = wf.execute(&GitOperation::Status).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn read_only_denies_commit() {
        let wf = GitWorkflow::new(read_only_config(), PathBuf::from("/tmp"));
        let result = wf
            .execute(&GitOperation::Commit {
                message: "test".into(),
                paths: vec!["file.rs".into()],
            })
            .await;
        assert!(matches!(
            result,
            Err(GitWorkflowError::OperationDenied { .. })
        ));
    }

    #[tokio::test]
    async fn read_only_denies_push() {
        let wf = GitWorkflow::new(read_only_config(), PathBuf::from("/tmp"));
        let result = wf.execute(&GitOperation::Push { force: false }).await;
        assert!(matches!(
            result,
            Err(GitWorkflowError::OperationDenied { .. })
        ));
    }

    #[tokio::test]
    async fn no_git_denies_everything() {
        let wf = GitWorkflow::new(no_git_config(), PathBuf::from("/tmp"));
        let result = wf.execute(&GitOperation::Status).await;
        assert!(matches!(
            result,
            Err(GitWorkflowError::OperationDenied { .. })
        ));
    }

    #[tokio::test]
    async fn status_in_real_repo() {
        let dir = tempfile::tempdir().unwrap();
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .await
            .unwrap();

        let config = StageGitConfig {
            branch_name: None,
            allowed_operations: None,
            commit_on_checkpoint: false,
            commit_on_task_complete: false,
            pr_on_stage_complete: false,
            protected_paths: vec![],
        };
        let wf = GitWorkflow::new(config, dir.path().to_path_buf());
        let output = wf.execute(&GitOperation::Status).await.unwrap();
        // Empty repo, no files
        assert!(output.is_empty() || output.trim().is_empty());
    }

    #[test]
    fn git_operation_kind_mapping() {
        assert_eq!(GitOperation::Status.kind(), GitOperationKind::Status);
        assert_eq!(
            GitOperation::Diff { staged: true }.kind(),
            GitOperationKind::Diff
        );
        assert_eq!(
            GitOperation::Log { count: 10 }.kind(),
            GitOperationKind::Log
        );
        assert_eq!(
            GitOperation::CreateBranch {
                name: "x".into(),
                from: None,
            }
            .kind(),
            GitOperationKind::CreateBranch
        );
        assert_eq!(
            GitOperation::Commit {
                message: "x".into(),
                paths: vec![],
            }
            .kind(),
            GitOperationKind::Commit
        );
        assert_eq!(
            GitOperation::Push { force: false }.kind(),
            GitOperationKind::Push
        );
        assert_eq!(
            GitOperation::CreatePr {
                title: "x".into(),
                body: "y".into(),
                base: None,
            }
            .kind(),
            GitOperationKind::CreatePr
        );
        assert_eq!(
            GitOperation::CheckoutFile {
                path: "x".into(),
                ref_: "y".into(),
            }
            .kind(),
            GitOperationKind::CheckoutFile
        );
    }
}
