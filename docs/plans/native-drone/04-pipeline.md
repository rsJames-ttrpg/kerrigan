# Plan 04: Drone Pipeline & Health Checks

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the stage state machine, environment health checks, exit condition checking, and git workflow enforcement in the native drone crate.

**Architecture:** `Pipeline` holds the resolved stage config. Before the agent loop starts, health checks run and must pass. After each turn (or orchestrator cycle), exit conditions are checked. `GitWorkflow` enforces branch/commit/PR policy per stage. Freeform stage is the default — full tool access, no enforced structure.

**Tech Stack:** tokio (process spawning for health checks), drone-sdk (DroneRunner trait)

**Spec:** `docs/specs/native-drone/04-drone-pipeline.md`

---

### Task 1: Stage types and resolution

**Files:**
- Create: `src/drones/native/src/pipeline.rs`

- [ ] **Step 1: Define Stage enum and StageConfig**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Spec,
    Plan,
    Implement,
    Review,
    Evolve,
    Freeform,
}

#[derive(Debug, Clone)]
pub struct StageConfig {
    pub stage: Stage,
    pub system_prompt: String,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
    pub entry_requirements: Vec<Requirement>,
    pub exit_conditions: Vec<ExitCondition>,
    pub git: StageGitConfig,
    pub max_turns: u32,
}

#[derive(Debug, Clone)]
pub enum Requirement {
    ArtifactExists { kind: String },
    FileExists { path: String },
    BranchExists { name: String },
}

#[derive(Debug, Clone)]
pub enum ExitCondition {
    FileCreated { glob: String },
    TestsPassing,
    PrCreated,
    ArtifactStored { kind: String },
    Custom(String),
}

#[derive(Debug, Clone)]
pub struct StageGitConfig {
    pub branch_name: Option<String>,
    pub allowed_operations: Option<Vec<GitOperationKind>>,
    pub commit_on_checkpoint: bool,
    pub commit_on_task_complete: bool,
    pub pr_on_stage_complete: bool,
    pub protected_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitOperationKind {
    Status,
    Diff,
    Log,
    CreateBranch,
    Commit,
    Push,
    CreatePr,
    CheckoutFile,
}
```

- [ ] **Step 2: Implement stage resolution and default configs**

Note: `JobSpec.config` is `serde_json::Value`, not `HashMap`. Access fields via `.as_str()`:

```rust
impl Stage {
    pub fn resolve(config: &serde_json::Value) -> Self {
        match config.get("stage").and_then(|v| v.as_str()) {
            Some("spec") => Stage::Spec,
            Some("plan") => Stage::Plan,
            Some("implement") => Stage::Implement,
            Some("review") => Stage::Review,
            Some("evolve") => Stage::Evolve,
            _ => Stage::Freeform,
        }
    }

    pub fn default_config(&self) -> StageConfig {
        match self {
            Stage::Spec => StageConfig {
                stage: Stage::Spec,
                system_prompt: String::new(), // set by PromptBuilder later
                allowed_tools: vec!["read_file", "glob_search", "grep_search", "write_file", "edit_file"]
                    .into_iter().map(String::from).collect(),
                denied_tools: vec!["bash", "git", "test", "agent"]
                    .into_iter().map(String::from).collect(),
                entry_requirements: vec![],
                exit_conditions: vec![
                    ExitCondition::FileCreated { glob: "docs/specs/*.md".into() },
                    ExitCondition::ArtifactStored { kind: "spec".into() },
                ],
                git: StageGitConfig {
                    branch_name: None,
                    allowed_operations: Some(vec![GitOperationKind::Status, GitOperationKind::Diff, GitOperationKind::Log]),
                    commit_on_checkpoint: false,
                    commit_on_task_complete: false,
                    pr_on_stage_complete: false,
                    protected_paths: vec![],
                },
                max_turns: 25,
            },
            Stage::Plan => StageConfig {
                stage: Stage::Plan,
                system_prompt: String::new(),
                allowed_tools: vec!["read_file", "glob_search", "grep_search", "write_file", "edit_file"]
                    .into_iter().map(String::from).collect(),
                denied_tools: vec!["bash", "git", "test", "agent"]
                    .into_iter().map(String::from).collect(),
                entry_requirements: vec![
                    Requirement::ArtifactExists { kind: "spec".into() },
                ],
                exit_conditions: vec![
                    ExitCondition::FileCreated { glob: "docs/plans/*.md".into() },
                    ExitCondition::ArtifactStored { kind: "plan".into() },
                ],
                git: StageGitConfig {
                    branch_name: None,
                    allowed_operations: Some(vec![GitOperationKind::Status, GitOperationKind::Diff, GitOperationKind::Log]),
                    commit_on_checkpoint: false,
                    commit_on_task_complete: false,
                    pr_on_stage_complete: false,
                    protected_paths: vec![],
                },
                max_turns: 25,
            },
            Stage::Implement => StageConfig {
                stage: Stage::Implement,
                system_prompt: String::new(),
                allowed_tools: vec![], // empty = all allowed
                denied_tools: vec![],
                entry_requirements: vec![
                    Requirement::ArtifactExists { kind: "plan".into() },
                ],
                exit_conditions: vec![
                    ExitCondition::TestsPassing,
                    ExitCondition::PrCreated,
                ],
                git: StageGitConfig {
                    branch_name: None, // set from job config at resolution time
                    allowed_operations: None, // all operations
                    commit_on_checkpoint: true,
                    commit_on_task_complete: true,
                    pr_on_stage_complete: true,
                    protected_paths: vec!["CLAUDE.md".into()],
                },
                max_turns: 100,
            },
            Stage::Review => StageConfig {
                stage: Stage::Review,
                system_prompt: String::new(),
                allowed_tools: vec!["read_file", "glob_search", "grep_search", "git", "bash"]
                    .into_iter().map(String::from).collect(),
                denied_tools: vec!["write_file", "edit_file"] // unless review_mode=fix in job config
                    .into_iter().map(String::from).collect(),
                entry_requirements: vec![],
                exit_conditions: vec![
                    ExitCondition::ArtifactStored { kind: "review".into() },
                ],
                git: StageGitConfig {
                    branch_name: None, // existing PR branch, set at resolution
                    allowed_operations: Some(vec![
                        GitOperationKind::Status, GitOperationKind::Diff, GitOperationKind::Log,
                        GitOperationKind::Commit, GitOperationKind::Push,
                    ]),
                    commit_on_checkpoint: false,
                    commit_on_task_complete: false,
                    pr_on_stage_complete: false,
                    protected_paths: vec![],
                },
                max_turns: 25,
            },
            Stage::Evolve => StageConfig {
                stage: Stage::Evolve,
                system_prompt: String::new(),
                allowed_tools: vec!["read_file", "glob_search", "grep_search", "bash"]
                    .into_iter().map(String::from).collect(),
                denied_tools: vec!["write_file", "edit_file", "git", "test", "agent"]
                    .into_iter().map(String::from).collect(),
                entry_requirements: vec![],
                exit_conditions: vec![],
                git: StageGitConfig {
                    branch_name: None,
                    allowed_operations: Some(vec![]), // no git ops
                    commit_on_checkpoint: false,
                    commit_on_task_complete: false,
                    pr_on_stage_complete: false,
                    protected_paths: vec![],
                },
                max_turns: 25,
            },
            Stage::Freeform => StageConfig {
                stage: Stage::Freeform,
                system_prompt: String::new(), // from job config system_prompt field, or default
                allowed_tools: vec![],
                denied_tools: vec![],
                entry_requirements: vec![],
                exit_conditions: vec![],
                git: StageGitConfig {
                    branch_name: None, // set from config at resolution time
                    allowed_operations: None,
                    commit_on_checkpoint: true,
                    commit_on_task_complete: false,
                    pr_on_stage_complete: false,
                    protected_paths: vec![],
                },
                max_turns: 50,
            },
        }
    }
}
```

Tests: resolve from `serde_json::Value` config, default configs have expected tool restrictions per stage, implement stage allows all tools, spec stage denies bash/git.

- [ ] **Step 3: Run tests, commit**

```bash
git add src/drones/native/
git commit -m "add stage types, resolution, and default configs"
```

---

### Task 2: Environment health checks

**Files:**
- Create: `src/drones/native/src/health.rs`

- [ ] **Step 1: Define health check types**

```rust
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct HealthCheck {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub expected_exit_code: i32,
    pub timeout: Duration,
    pub required: bool,
}

#[derive(Debug)]
pub struct HealthCheckResult {
    pub name: String,
    pub passed: bool,
    pub required: bool,
    pub output: String,
    pub duration_ms: u64,
}

#[derive(Debug)]
pub struct HealthReport {
    pub checks: Vec<HealthCheckResult>,
}

impl HealthReport {
    pub fn all_required_passed(&self) -> bool {
        self.checks.iter().all(|c| !c.required || c.passed)
    }

    pub fn summary(&self) -> String {
        let failed: Vec<_> = self.checks.iter()
            .filter(|c| !c.passed)
            .map(|c| format!("{} ({})", c.name, if c.required { "required" } else { "optional" }))
            .collect();
        if failed.is_empty() {
            "all checks passed".into()
        } else {
            format!("failed: {}", failed.join(", "))
        }
    }
}
```

- [ ] **Step 2: Implement health check runner**

```rust
pub async fn run_health_checks(checks: &[HealthCheck]) -> HealthReport {
    let mut results = Vec::new();
    for check in checks {
        let start = std::time::Instant::now();
        let output = tokio::time::timeout(
            check.timeout,
            tokio::process::Command::new(&check.command)
                .args(&check.args)
                .output(),
        )
        .await;

        let (passed, output_str) = match output {
            Ok(Ok(o)) => (
                o.status.code() == Some(check.expected_exit_code),
                String::from_utf8_lossy(&o.stdout).to_string()
                    + &String::from_utf8_lossy(&o.stderr),
            ),
            Ok(Err(e)) => (false, format!("failed to execute: {e}")),
            Err(_) => (false, "timed out".to_string()),
        };

        results.push(HealthCheckResult {
            name: check.name.clone(),
            passed,
            required: check.required,
            output: output_str,
            duration_ms: start.elapsed().as_millis() as u64,
        });
    }
    HealthReport { checks: results }
}
```

- [ ] **Step 3: Implement stage-specific check sets**

```rust
pub fn checks_for_stage(stage: &Stage) -> Vec<HealthCheck> {
    let mut checks = vec![
        HealthCheck { name: "cargo".into(), command: "cargo".into(), args: vec!["--version".into()], expected_exit_code: 0, timeout: Duration::from_secs(10), required: true },
        HealthCheck { name: "rustc".into(), command: "rustc".into(), args: vec!["--version".into()], expected_exit_code: 0, timeout: Duration::from_secs(10), required: true },
        HealthCheck { name: "git".into(), command: "git".into(), args: vec!["--version".into()], expected_exit_code: 0, timeout: Duration::from_secs(10), required: true },
    ];

    match stage {
        Stage::Implement => {
            checks.push(HealthCheck { name: "build".into(), command: "cargo".into(), args: vec!["check".into()], expected_exit_code: 0, timeout: Duration::from_secs(300), required: true });
            checks.push(HealthCheck { name: "tests".into(), command: "cargo".into(), args: vec!["test".into()], expected_exit_code: 0, timeout: Duration::from_secs(600), required: true });
            checks.push(HealthCheck { name: "gh".into(), command: "gh".into(), args: vec!["--version".into()], expected_exit_code: 0, timeout: Duration::from_secs(10), required: true });
        }
        Stage::Review => {
            checks.push(HealthCheck { name: "build".into(), command: "cargo".into(), args: vec!["check".into()], expected_exit_code: 0, timeout: Duration::from_secs(300), required: true });
            checks.push(HealthCheck { name: "gh".into(), command: "gh".into(), args: vec!["--version".into()], expected_exit_code: 0, timeout: Duration::from_secs(10), required: true });
        }
        _ => {}
    }

    checks.push(HealthCheck { name: "creep".into(), command: "creep-cli".into(), args: vec!["--version".into()], expected_exit_code: 0, timeout: Duration::from_secs(10), required: false });

    checks
}
```

Tests: verify correct checks for each stage, verify required flag assignment.

- [ ] **Step 4: Run tests, commit**

```bash
git add src/drones/native/
git commit -m "add environment health checks with stage-specific check sets"
```

---

### Task 3: Exit condition checking

**Files:**
- Create: `src/drones/native/src/exit_conditions.rs`

- [ ] **Step 1: Implement exit condition evaluator**

```rust
pub struct ConditionResult {
    pub condition: String,
    pub met: bool,
    pub detail: String,
}

pub async fn check_exit_conditions(
    conditions: &[ExitCondition],
    workspace: &std::path::Path,
) -> Vec<ConditionResult> {
    let mut results = Vec::new();
    for cond in conditions {
        let result = match cond {
            ExitCondition::FileCreated { glob } => {
                // Use globset to check for matching files
                check_file_created(workspace, glob)
            }
            ExitCondition::TestsPassing => {
                check_tests_passing(workspace).await
            }
            ExitCondition::PrCreated => {
                check_pr_exists(workspace).await
            }
            ExitCondition::ArtifactStored { kind } => {
                // Placeholder — checked via Overseer MCP in real usage
                ConditionResult { condition: format!("artifact:{kind}"), met: false, detail: "requires MCP check".into() }
            }
            ExitCondition::Custom(command) => {
                check_custom_command(command).await
            }
        };
        results.push(result);
    }
    results
}
```

Implement each checker. `check_tests_passing` runs `cargo test` and checks exit code. `check_pr_exists` runs `gh pr view` and checks exit code. `check_custom_command` runs `sh -c {command}`.

Tests: file created check with temp dir, custom command check.

- [ ] **Step 2: Run tests, commit**

```bash
git add src/drones/native/
git commit -m "add exit condition checking for stage completion"
```

---

### Task 4: Git workflow enforcement

**Files:**
- Create: `src/drones/native/src/git_workflow.rs`

- [ ] **Step 1: Implement GitWorkflow with policy enforcement**

```rust
use std::path::PathBuf;
use tokio::sync::Mutex;

pub struct GitWorkflow {
    config: StageGitConfig,
    workspace: PathBuf,
    serializer: GitSerializer,
}

pub struct GitSerializer {
    lock: Mutex<()>,
}

impl GitWorkflow {
    pub fn new(config: StageGitConfig, workspace: PathBuf) -> Self {
        Self { config, workspace, serializer: GitSerializer { lock: Mutex::new(()) } }
    }

    /// Validate and execute a git operation against the stage policy
    pub async fn execute(&self, operation: &GitOperation) -> Result<String, GitWorkflowError> {
        // Check operation is allowed
        if let Some(allowed) = &self.config.allowed_operations {
            let kind = operation.kind();
            if !allowed.contains(&kind) {
                return Err(GitWorkflowError::OperationDenied { operation: format!("{kind:?}") });
            }
        }

        // Check specific policy rules
        match operation {
            GitOperation::Push { force: true } => {
                return Err(GitWorkflowError::ForcePushDenied);
            }
            GitOperation::Commit { paths, .. } => {
                for path in paths {
                    if self.is_protected(path) {
                        return Err(GitWorkflowError::ProtectedPath { path: path.clone() });
                    }
                }
            }
            GitOperation::CreateBranch { name, .. } => {
                // Validate branch name if config specifies one
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
        self.config.protected_paths.iter().any(|p| {
            globset::Glob::new(p)
                .ok()
                .and_then(|g| g.compile_matcher().is_match(path).then_some(()))
                .is_some()
        })
    }

    async fn run_git_command(&self, operation: &GitOperation) -> Result<String, GitWorkflowError> {
        match operation {
            GitOperation::Status => self.exec_git(&["status", "--porcelain"]).await,
            GitOperation::Diff { staged } => {
                if *staged { self.exec_git(&["diff", "--staged"]).await }
                else { self.exec_git(&["diff"]).await }
            }
            GitOperation::Log { count } => {
                self.exec_git(&["log", "--oneline", &format!("-{count}")]).await
            }
            GitOperation::CreateBranch { name, from } => {
                let mut args = vec!["checkout", "-b", name.as_str()];
                if let Some(base) = from { args.push(base.as_str()); }
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
                if *force { args.push("--force"); }
                self.exec_git(&args).await
            }
            GitOperation::CreatePr { title, body, base } => {
                let mut args = vec!["pr", "create", "--title", title.as_str(), "--body", body.as_str()];
                if let Some(b) = base { args.extend(["--base", b.as_str()]); }
                self.exec_gh(&args).await
            }
            GitOperation::CheckoutFile { path, ref_ } => {
                self.exec_git(&["checkout", ref_.as_str(), "--", path.as_str()]).await
            }
        }
    }

    async fn exec_git(&self, args: &[&str]) -> Result<String, GitWorkflowError> {
        let output = tokio::process::Command::new("git")
            .args(args).current_dir(&self.workspace).output().await
            .map_err(|e| GitWorkflowError::CommandFailed(e.to_string()))?;
        if !output.status.success() {
            return Err(GitWorkflowError::CommandFailed(String::from_utf8_lossy(&output.stderr).to_string()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn exec_gh(&self, args: &[&str]) -> Result<String, GitWorkflowError> {
        let output = tokio::process::Command::new("gh")
            .args(args).current_dir(&self.workspace).output().await
            .map_err(|e| GitWorkflowError::CommandFailed(e.to_string()))?;
        if !output.status.success() {
            return Err(GitWorkflowError::CommandFailed(String::from_utf8_lossy(&output.stderr).to_string()));
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
```

Implement `run_git_command` for each operation variant (status, diff, log, create_branch, commit, push, create_pr, checkout_file).

Tests: force push denied, protected path blocked, branch name enforced, read-only operations allowed.

- [ ] **Step 2: Run tests, buckify, verify build**

Run: `cd src/drones/native && cargo test`
Run: `./tools/buckify.sh`
Run: `buck2 build root//src/drones/native:native-drone`

- [ ] **Step 3: Commit**

```bash
git add src/drones/native/ Cargo.lock third-party/BUCK
git commit -m "add git workflow enforcement with stage policy"
```
