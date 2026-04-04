# Plan Errata

Issues found during self-review. Workers MUST apply these corrections when executing the referenced plan tasks.

## Critical: drone-sdk Type Mismatches

The plans were written against assumed types. The actual `drone-sdk` types differ. These corrections apply to **Plans 00, 06**, and anywhere `DroneMessage`, `DroneEnvironment`, `DroneOutput`, or `QueenChannel` are used.

### DroneEnvironment fields

**Actual** (`src/drone-sdk/src/protocol.rs:96-100`):
```rust
pub struct DroneEnvironment {
    pub home: PathBuf,      // NOT home_dir
    pub workspace: PathBuf, // NOT workspace_dir
}
```

**Fix Plan 00 Task 2** and **Plan 06 Task 3**: use `home` and `workspace`.

### DroneMessage variants are wrapped structs

**Actual** (`src/drone-sdk/src/protocol.rs:18-25`):
```rust
pub enum DroneMessage {
    AuthRequest(AuthRequest),
    Progress(Progress),      // NOT Progress { status, detail }
    Result(DroneOutput),
    Error(DroneError),       // NOT Error { message }
}

pub struct Progress {
    pub status: String,
    pub detail: Option<String>, // Option, not bare String
}

pub struct DroneError {
    pub message: String,
}
```

**Fix Plan 06 Task 2 (event bridge)**: every `DroneMessage::Progress { status: ..., detail: ... }` must become:
```rust
DroneMessage::Progress(Progress {
    status: "...".into(),
    detail: Some("...".into()),
})
```

### DroneOutput.conversation is Value

**Actual** (`src/drone-sdk/src/protocol.rs:63-71`):
```rust
pub struct DroneOutput {
    pub exit_code: i32,
    pub conversation: Value,       // serde_json::Value, NOT Option<String>
    pub artifacts: Vec<String>,
    pub git_refs: GitRefs,
    pub session_jsonl_gz: Option<String>,
}
```

**Fix Plan 00 Task 2** and **Plan 06 Task 3**: use `conversation: serde_json::json!({})` for placeholder, `serde_json::to_value(&session)?` for real output.

### QueenChannel is synchronous

**Actual** (`src/drone-sdk/src/harness.rs:6-58`):
```rust
pub struct QueenChannel {
    writer: Stdout,
    reader: BufReader<Stdin>,
}

impl QueenChannel {
    fn send(&mut self, msg: &DroneMessage) -> anyhow::Result<()>;  // sync, &mut self
    fn recv(&mut self) -> anyhow::Result<QueenMessage>;            // sync, &mut self
    pub fn request_auth(&mut self, url: &str, message: &str) -> anyhow::Result<AuthResponse>;
    pub fn progress(&mut self, status: &str, detail: &str) -> anyhow::Result<()>;
}
```

**Fix Plan 06 Task 2 (event bridge)**: The bridge cannot hold `QueenChannel` directly since `execute` receives `&mut QueenChannel`. Instead, use an `mpsc::Sender<DroneMessage>` in the bridge, with a forwarding task that drains the receiver and calls `channel.send()`. This allows the bridge to implement `EventSink` (which is `Send + Sync`) without needing `&mut` access to the channel:

```rust
pub struct DroneEventBridge {
    sender: tokio::sync::mpsc::UnboundedSender<DroneMessage>,
    workspace: PathBuf,
    run_id: String,
}

// In execute(), before creating the bridge:
let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
let bridge = Arc::new(DroneEventBridge::new(tx, workspace, run_id));

// Spawn forwarding task (runs on blocking thread since QueenChannel is sync)
let forward_handle = tokio::task::spawn_blocking(move || {
    while let Some(msg) = rx.blocking_recv() {
        if let Err(e) = channel.send(&msg) {
            tracing::warn!("failed to send to queen: {e}");
            break;
        }
    }
});
```

### JobSpec.config is Value, not HashMap

**Actual** (`src/drone-sdk/src/protocol.rs:29-36`):
```rust
pub struct JobSpec {
    pub job_run_id: String,
    pub repo_url: String,
    pub branch: Option<String>,
    pub task: String,
    pub config: Value,  // serde_json::Value, NOT HashMap<String, String>
}
```

**Fix Plan 04 Task 1** stage resolution: access config as `job.config["stage"].as_str()` not `job.config.get("stage")`.

## Critical: todo!() Placeholders

### Plan 04 Task 4: GitWorkflow::run_git_command

Replace `todo!()` with dispatch to shell commands:

```rust
async fn run_git_command(&self, operation: &GitOperation) -> Result<String, GitWorkflowError> {
    let output = match operation {
        GitOperation::Status => {
            self.exec_git(&["status", "--porcelain"]).await?
        }
        GitOperation::Diff { staged } => {
            let args = if *staged { vec!["diff", "--staged"] } else { vec!["diff"] };
            self.exec_git(&args).await?
        }
        GitOperation::Log { count } => {
            self.exec_git(&["log", "--oneline", &format!("-{count}")]).await?
        }
        GitOperation::CreateBranch { name, from } => {
            let mut args = vec!["checkout", "-b", name.as_str()];
            if let Some(base) = from {
                args.push(base.as_str());
            }
            self.exec_git(&args).await?
        }
        GitOperation::Commit { message, paths } => {
            for path in paths {
                self.exec_git(&["add", path.as_str()]).await?;
            }
            self.exec_git(&["commit", "-m", message.as_str()]).await?
        }
        GitOperation::Push { force } => {
            let mut args = vec!["push"];
            if *force { args.push("--force"); }
            self.exec_git(&args).await?
        }
        GitOperation::CreatePr { title, body, base } => {
            let mut args = vec!["pr", "create", "--title", title.as_str(), "--body", body.as_str()];
            if let Some(b) = base {
                args.extend(["--base", b.as_str()]);
            }
            self.exec_gh(&args).await?
        }
        GitOperation::CheckoutFile { path, ref_ } => {
            self.exec_git(&["checkout", ref_.as_str(), "--", path.as_str()]).await?
        }
    };
    Ok(output)
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
```

### Plan 07 Task 3: Orchestrator::spawn_task_agent

Replace `todo!()`:

```rust
async fn spawn_task_agent(
    &self,
    parent: &ConversationLoop,
    task: Task,
) -> tokio::task::JoinHandle<TaskResult> {
    let event_sink = self.event_sink.clone();
    let git_workflow = self.git_workflow.clone();
    let task_id = task.id.clone();

    // Use the parent's factory to create a sub-agent conversation loop
    let prompt = format!(
        "Implement task {}: {}\n\nRelevant files: {}",
        task.id, task.description, task.files.join(", ")
    );

    tokio::spawn(async move {
        event_sink.emit(RuntimeEvent::TurnStart { task: prompt.clone() });

        // Create sub-agent via parent's spawn mechanism
        let result = parent.spawn_sub_agent(runtime::tools::AgentRequest {
            task: prompt,
            tools: None, // inherit parent tools
            max_iterations: Some(25),
            files: Some(task.files.clone()),
        }).await;

        let (success, output) = match result {
            Ok(text) => (true, text),
            Err(e) => (false, format!("task failed: {e}")),
        };

        TaskResult {
            task_id,
            success,
            output,
            commits: vec![],
        }
    })
}
```

### Plan 08 Task 3: parse_locations

Replace `todo!()`:

```rust
fn parse_locations(value: serde_json::Value) -> anyhow::Result<Vec<SymbolLocation>> {
    // Null response = no results
    if value.is_null() {
        return Ok(vec![]);
    }

    // Single Location object: { "uri": "...", "range": { "start": { "line": N, "character": M }, ... }}
    if value.is_object() && value.get("uri").is_some() {
        return Ok(vec![parse_single_location(&value)?]);
    }

    // Array of Location or LocationLink
    if let Some(arr) = value.as_array() {
        let mut locations = Vec::new();
        for item in arr {
            if item.get("targetUri").is_some() {
                // LocationLink: use targetUri and targetRange
                let uri = item["targetUri"].as_str().unwrap_or_default();
                let range = &item["targetRange"];
                locations.push(SymbolLocation {
                    file: PathBuf::from(uri.strip_prefix("file://").unwrap_or(uri)),
                    start_line: range["start"]["line"].as_u64().unwrap_or(0) as u32,
                    start_column: range["start"]["character"].as_u64().unwrap_or(0) as u32,
                    end_line: range["end"]["line"].as_u64().unwrap_or(0) as u32,
                    end_column: range["end"]["character"].as_u64().unwrap_or(0) as u32,
                });
            } else {
                locations.push(parse_single_location(item)?);
            }
        }
        return Ok(locations);
    }

    Ok(vec![])
}

fn parse_single_location(value: &serde_json::Value) -> anyhow::Result<SymbolLocation> {
    let uri = value["uri"].as_str().unwrap_or_default();
    let range = &value["range"];
    Ok(SymbolLocation {
        file: PathBuf::from(uri.strip_prefix("file://").unwrap_or(uri)),
        start_line: range["start"]["line"].as_u64().unwrap_or(0) as u32,
        start_column: range["start"]["character"].as_u64().unwrap_or(0) as u32,
        end_line: range["end"]["line"].as_u64().unwrap_or(0) as u32,
        end_column: range["end"]["character"].as_u64().unwrap_or(0) as u32,
    })
}
```

## Important: Stage Default Configs

**Plan 04 Task 1 Step 2**: Replace comment-only match arms with actual configs. Use the spec definitions in `docs/specs/native-drone/04-drone-pipeline.md` "Stage Definitions" section. Each stage needs:
- `system_prompt`: generated by `PromptBuilder::for_stage` (pass empty string here, it's set later)
- `allowed_tools` / `denied_tools`: per spec
- `entry_requirements` / `exit_conditions`: per spec
- `git`: `StageGitConfig` with `branch_name`, `allowed_operations`, `commit_on_*`, `pr_on_*`, `protected_paths`
- `max_turns`: reasonable defaults (spec: 25, implement: 100, freeform: 50)

## Important: HealthCheckResult needs required flag

**Plan 04 Task 2**: Add `required: bool` to `HealthCheckResult`:

```rust
pub struct HealthCheckResult {
    pub name: String,
    pub passed: bool,
    pub required: bool,  // ADD THIS
    pub output: String,
    pub duration_ms: u64,
}

impl HealthReport {
    pub fn all_required_passed(&self) -> bool {
        self.checks.iter().all(|c| !c.required || c.passed)
    }
}
```

## Important: Two Role enums need translation

**Plans 01 and 03** define separate `Role` enums:
- `api::types::Role` = `{ User, Assistant }` (wire format)
- `conversation::session::Role` = `{ System, User, Assistant, Tool }` (internal)

Plan 03 Task 2's `build_request()` MUST translate:
- `Session::Role::System` messages → `ApiRequest.system` blocks (not in messages array)
- `Session::Role::User` → `api::Role::User`
- `Session::Role::Assistant` → `api::Role::Assistant`
- `Session::Role::Tool` → `api::Role::User` with tool_result content blocks (Anthropic format)

Similarly, `ContentBlock` differs (`blocks` vs `content`, `output` vs `content`). The `build_request` method is where this translation happens.

## Important: TurnResult missing fields

**Plan 03 Task 2**: `TurnResult` should match the spec:

```rust
pub struct TurnResult {
    pub iterations: u32,
    pub compacted: bool,
    pub usage: TokenUsage,
}
```

(`messages` and `tool_calls` can be accessed from the session directly rather than duplicated in the result.)

## Deferred: Not in Initial Plans

These spec requirements are acknowledged but deferred to follow-up plans:

1. **Bash sandboxing** (Linux namespaces) — complex, platform-specific. Basic workspace restriction is in Plan 02. Namespace sandboxing is a follow-up.
2. **Tool cache eviction** (LRU, max_size_mb) — Plan 05 defines the config. Actual cache + eviction logic is a follow-up.
3. **LSP idle timeout / crash recovery** — Plan 08 covers core LSP functionality. Grace period timers and restart logic are a follow-up.
4. **Queen-side handling of DroneEvent** — Plan 06 extends the protocol. Queen supervisor changes to forward events to Overseer are a follow-up (Queen works fine ignoring unknown variants via serde).
5. **Diagnostics prompt section (priority 140)** — depends on Plan 08 (Creep LSP) being complete. Added as a follow-up after both Plans 05 and 08 are done.
6. **Health check failure injection into system prompt** — the health report should be passed to the prompt builder for implement stage when tests are pre-failing. Follow-up after Plans 04 and 05 are integrated.
7. **StreamInterrupted retry** — add to Plan 01 Task 5's retry match arms.
8. **GitRefs.commits field** — add `commits: Vec<String>` to `GitRefs` in drone-sdk when extending the protocol in Plan 06 Task 1.

## Repo Cache and Worktrees

Plan 05 Task 3 uses `git worktree` for the repo cache. User memory notes say "Don't use git worktrees; Buck2 cold cache makes them slow." This applies to **development worktrees** (where Buck2 rebuilds everything). For **drone workspaces** this is different — the drone doesn't run Buck2 builds from the worktree, it uses `cargo` directly. The worktree approach is correct here for fast repo checkout. If this causes issues, fall back to a full clone from the bare cache.
