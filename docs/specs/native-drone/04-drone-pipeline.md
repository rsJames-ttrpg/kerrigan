# Drone: Pipeline & Orchestration

**Date:** 2026-04-04
**Parent:** [00-overview.md](00-overview.md)

## Purpose

The opinionated workflow layer on top of the runtime. Defines the pipeline state machine (stages), task orchestrator (sub-agent coordination), and enforced git workflow. This is where kerrigan's development methodology lives in code.

## Stage State Machine

```rust
pub enum Stage {
    Spec,
    Plan,
    Implement,
    Review,
    Evolve,
    Freeform,
}

pub struct StageConfig {
    pub system_prompt: String,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
    pub entry_requirements: Vec<Requirement>,
    pub exit_conditions: Vec<ExitCondition>,
    pub git: StageGitConfig,
    pub max_turns: u32,
}

pub enum Requirement {
    ArtifactExists { kind: String },
    FileExists { path: String },
    BranchExists { name: String },
}

pub enum ExitCondition {
    FileCreated { glob: String },
    TestsPassing,
    PrCreated,
    ArtifactStored { kind: String },
    Custom(String),
}
```

### Stage Resolution

```rust
fn resolve_stage(job: &JobSpec) -> Stage {
    match job.config.get("stage").map(|s| s.as_str()) {
        Some("spec") => Stage::Spec,
        Some("plan") => Stage::Plan,
        Some("implement") => Stage::Implement,
        Some("review") => Stage::Review,
        Some("evolve") => Stage::Evolve,
        _ => Stage::Freeform,
    }
}
```

Unknown or absent stage values default to Freeform. Any new job definition works immediately without drone changes.

### Stage Definitions

**Spec**
- Mission: take a problem statement, produce a design document
- Tools: file ops, grep, glob, web fetch, MCP (Overseer, Creep)
- Denied: bash, git commit/push/PR (spec is a document, not code)
- Entry: problem statement in task input
- Exit: spec file created matching `docs/specs/*.md`, stored as Overseer artifact
- Git: read-only (no branch, no commits)

**Plan**
- Mission: read spec artifact, produce an implementation plan with discrete tasks
- Tools: file ops, grep, glob, MCP
- Denied: bash, git write operations
- Entry: spec artifact exists (referenced in job config)
- Exit: plan file created matching `docs/plans/*.md`, stored as Overseer artifact
- Git: read-only

**Implement**
- Mission: execute the plan, write code, pass tests, create PR
- Tools: all (file ops, bash, git, test runner, sub-agent, MCP)
- Entry: plan artifact exists, branch created
- Exit: tests passing AND PR created
- Git: full (branch from config, commit on task completion, PR on stage complete)
- Orchestrator: active (task decomposition from plan, sub-agent coordination)

**Review**
- Mission: review existing PR, post feedback or commit fixes
- Tools: file ops, grep, glob, git (diff, commit, push), bash (read-only commands)
- Denied: write_file, edit_file by default. If job config includes `review_mode = "fix"`, write tools are enabled so the reviewer can commit fixes directly to the PR branch.
- Entry: PR URL in job config
- Exit: review posted as artifact
- Git: can commit to existing PR branch (fix mode only), no new branches

**Evolve**
- Mission: read analysis report, create GitHub issues for actionable recommendations
- Tools: git (issue creation only), file ops (read-only)
- Denied: write_file, edit_file, bash
- Entry: analysis report in task input
- Exit: issues created
- Git: no branches, no commits, no PRs

**Freeform**
- Mission: whatever the task says
- Tools: all available
- No entry requirements, no enforced exit conditions
- System prompt from job config's `system_prompt` field, or a generic default
- Git: standard policy (branch naming enforced, no force push, commit allowed)

## Orchestrator

Active only during Implement stage (and optionally Freeform if the task is large enough). Manages sub-agent coordination for parallel work.

```rust
pub struct Orchestrator {
    conversation_loop: ConversationLoop,  // parent loop, used to spawn sub-agents
    task_queue: VecDeque<Task>,
    active_agents: Vec<SubAgent>,
    completed: Vec<TaskResult>,
    max_parallel: usize,
}

pub struct Task {
    pub id: String,
    pub description: String,
    pub dependencies: Vec<String>,      // task IDs that must complete first
    pub files: Vec<String>,             // relevant files for context scoping
}

pub struct TaskResult {
    pub task_id: String,
    pub success: bool,
    pub output: String,
    pub git_commits: Vec<String>,
}
```

### Task Decomposition

The orchestrator parses the plan document (structured markdown) into `Task` structs:

```markdown
## Tasks

- [ ] **task-1**: Add auth middleware to axum router
  - Files: src/api/mod.rs, src/api/auth.rs
  - Depends: none

- [ ] **task-2**: Write auth middleware tests
  - Files: src/api/auth.rs, tests/api_auth.rs
  - Depends: task-1

- [ ] **task-3**: Add rate limiting middleware
  - Files: src/api/rate_limit.rs
  - Depends: none
```

Parsing rules:
- Checkbox items with `**task-id**:` prefix
- `Files:` line → relevant files for context scoping
- `Depends:` line → task dependency edges
- Tasks with no overlapping files and no dependency → `parallel = true`

### Execution Flow

```
1. Parse plan into task graph
2. Topological sort respecting dependencies
3. While tasks remain:
   a. Find all tasks with satisfied dependencies
   b. Spawn up to max_parallel sub-agents for ready tasks
   c. Each sub-agent gets:
      - Focused system prompt: "Implement task {id}: {description}"
      - Relevant file context only (not the whole repo)
      - Scoped tools (file ops + bash + test runner + git commit)
   d. Wait for any sub-agent to complete
   e. On completion:
      - Record result
      - Checkpoint (commit + artifact)
      - Emit TaskCompleted event
      - Check for newly unblocked tasks
4. After all tasks: run full test suite
5. If tests fail: single-agent fix-up loop (all context, test failures as input)
6. Create/update PR
```

### Git Serialization

Sub-agents share the workspace but git commits are serialized:

```rust
pub struct GitSerializer {
    lock: tokio::sync::Mutex<()>,
}

impl GitSerializer {
    pub async fn commit(&self, message: &str, paths: &[String]) -> Result<String, GitError> {
        let _guard = self.lock.lock().await;
        // Stage, commit, return SHA
    }
}
```

This prevents concurrent agents from creating conflicting commits. Each agent stages and commits atomically.

## Git Workflow Enforcement

```rust
pub struct GitWorkflow {
    config: StageGitConfig,
    repo: PathBuf,
    serializer: Arc<GitSerializer>,
}

pub struct StageGitConfig {
    pub branch_name: Option<String>,    // None = read-only stage (no branch)
    pub commit_on_checkpoint: bool,
    pub commit_on_task_complete: bool,
    pub pr_on_stage_complete: bool,
    pub protected_paths: Vec<String>,
}
```

### Policy Enforcement

The git tool delegates all operations to `GitWorkflow`, which enforces:

| Rule | Enforcement |
|------|-------------|
| Branch naming | `branch_name` from config, LLM cannot choose |
| No force push | `Push { force: true }` rejected unless stage explicitly allows |
| No default branch commits | Commit rejected if on main/master |
| Protected paths | `edit_file`/`write_file` rejected for paths matching `protected_paths` globs |
| Commit messages | Validated: non-empty, reasonable length, no garbage |
| PR creation | Uses `gh pr create` internally, title/body from LLM input |

### Branch Strategy Per Stage

| Stage | Branch |
|-------|--------|
| Spec | None (read-only) |
| Plan | None (read-only) |
| Implement | `{config.branch_prefix}{job-name}` — created at stage start |
| Review | Existing PR branch (checked out at start) |
| Evolve | None (issue creation only) |
| Freeform | `{config.branch_prefix}freeform-{run-id}` |

## Exit Condition Checking

After each turn (or after orchestrator completes all tasks), the drone checks exit conditions:

```rust
fn check_exit_conditions(stage: &StageConfig, workspace: &Path) -> Vec<ConditionResult> {
    stage.exit_conditions.iter().map(|cond| {
        match cond {
            ExitCondition::FileCreated { glob } => {
                let matches = glob_search(workspace, glob);
                ConditionResult { met: !matches.is_empty(), detail: format!("{} files match", matches.len()) }
            }
            ExitCondition::TestsPassing => {
                let result = run_test_suite(workspace);
                ConditionResult { met: result.failed == 0, detail: format!("{} passed", result.passed) }
            }
            ExitCondition::PrCreated => {
                let pr = check_pr_exists(workspace);
                ConditionResult { met: pr.is_some(), detail: pr.map(|p| p.url).unwrap_or_default() }
            }
            ExitCondition::ArtifactStored { kind } => {
                // Check via Overseer MCP
                ConditionResult { met: artifact_exists(kind), detail: kind.clone() }
            }
            ExitCondition::Custom(command) => {
                // Run shell command; exit code 0 = met, non-zero = not met
                let output = std::process::Command::new("sh").arg("-c").arg(command).output();
                let met = output.map(|o| o.status.success()).unwrap_or(false);
                ConditionResult { met, detail: command.clone() }
            }
        }
    }).collect()
}
```

If all conditions are met, the stage completes successfully. If max_turns is reached with unmet conditions, the stage fails with a report of which conditions weren't satisfied.
