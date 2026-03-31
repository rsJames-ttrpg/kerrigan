# Drones Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `drone-sdk` crate (protocol, harness, DroneRunner trait) and the first concrete drone (`claude-drone`) that runs Claude Code as a subprocess with embedded config.

**Architecture:** `drone-sdk` defines a JSON-line protocol over stdin/stdout and a `DroneRunner` trait (setup/execute/teardown). `claude-drone` links it, embeds user-level Claude Code config at build time, creates an isolated home dir at runtime, and spawns the `claude` CLI in `--print --output-format stream-json` mode.

**Tech Stack:** Rust (edition 2024), tokio, serde, serde_json, async-trait, anyhow, tempfile

---

## File Structure

```
src/
  drone-sdk/
    Cargo.toml
    BUCK
    src/
      lib.rs              # Re-exports
      protocol.rs         # Message envelopes + types (JobSpec, DroneOutput, etc.)
      runner.rs           # DroneRunner trait + DroneEnvironment
      harness.rs          # harness::run() entrypoint + QueenChannel
  drones/
    claude/
      base/
        Cargo.toml
        BUCK
        src/
          main.rs         # harness::run(ClaudeDrone::new())
          drone.rs        # ClaudeDrone: DroneRunner impl
          environment.rs  # Temp home creation, config extraction, auth symlinks
        config/
          settings.json   # Claude Code user-level settings
          CLAUDE.md       # Base instructions for Claude drones
```

---

### Task 1: drone-sdk crate scaffolding

**Files:**
- Create: `src/drone-sdk/Cargo.toml`
- Create: `src/drone-sdk/BUCK`
- Create: `src/drone-sdk/src/lib.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create Cargo.toml**

Create `src/drone-sdk/Cargo.toml`:

```toml
[package]
name = "drone-sdk"
version = "0.1.0"
edition = "2024"

[dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread", "io-util", "io-std"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
anyhow = "1"
tracing = "0.1"
```

- [ ] **Step 2: Add to workspace**

In root `Cargo.toml`, change:
```toml
members = ["src/overseer", "src/queen"]
```
to:
```toml
members = ["src/overseer", "src/queen", "src/drone-sdk"]
```

- [ ] **Step 3: Create lib.rs placeholder**

Create `src/drone-sdk/src/lib.rs`:

```rust
pub mod protocol;
pub mod runner;
pub mod harness;
```

Create empty files so it compiles:
- `src/drone-sdk/src/protocol.rs` with just a comment `// Protocol types`
- `src/drone-sdk/src/runner.rs` with just a comment `// DroneRunner trait`
- `src/drone-sdk/src/harness.rs` with just a comment `// Harness entrypoint`

- [ ] **Step 4: Create BUCK file**

Create `src/drone-sdk/BUCK`:

```python
SDK_SRCS = glob(["src/**/*.rs"])

SDK_DEPS = [
    "//third-party:anyhow",
    "//third-party:async-trait",
    "//third-party:serde",
    "//third-party:serde_json",
    "//third-party:tokio",
    "//third-party:tracing",
]

rust_library(
    name = "drone-sdk",
    srcs = SDK_SRCS,
    deps = SDK_DEPS,
    visibility = ["PUBLIC"],
)

rust_test(
    name = "drone-sdk-test",
    srcs = SDK_SRCS,
    deps = SDK_DEPS,
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 5: Regenerate third-party BUCK**

Run: `cd /home/jackm/repos/kerrigan && ./tools/buckify.sh`

- [ ] **Step 6: Verify builds**

Run: `cargo check -p drone-sdk`
Run: `buck2 build root//src/drone-sdk:drone-sdk`

- [ ] **Step 7: Commit**

```bash
git add src/drone-sdk/ Cargo.toml Cargo.lock
git commit -m "feat(drone-sdk): scaffold drone-sdk crate"
```

---

### Task 2: Protocol types

**Files:**
- Modify: `src/drone-sdk/src/protocol.rs`

- [ ] **Step 1: Write tests for protocol serialization**

Replace `src/drone-sdk/src/protocol.rs`:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

// ── Queen -> Drone messages ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
#[serde(rename_all = "snake_case")]
pub enum QueenMessage {
    Job(JobSpec),
    AuthResponse(AuthResponse),
    Cancel {},
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSpec {
    pub job_run_id: String,
    pub repo_url: String,
    pub branch: Option<String>,
    pub task: String,
    #[serde(default)]
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub approved: bool,
}

// ── Drone -> Queen messages ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
#[serde(rename_all = "snake_case")]
pub enum DroneMessage {
    AuthRequest(AuthRequest),
    Progress(Progress),
    Result(DroneOutput),
    Error(DroneError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub url: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DroneOutput {
    pub exit_code: i32,
    pub conversation: Value,
    pub artifacts: Vec<String>,
    pub git_refs: GitRefs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRefs {
    pub branch: Option<String>,
    pub pr_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DroneError {
    pub message: String,
}

// ── Drone environment ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DroneEnvironment {
    pub home: PathBuf,
    pub workspace: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queen_job_message_roundtrip() {
        let msg = QueenMessage::Job(JobSpec {
            job_run_id: "run-123".to_string(),
            repo_url: "https://github.com/user/repo".to_string(),
            branch: Some("main".to_string()),
            task: "Fix the bug".to_string(),
            config: serde_json::json!({}),
        });
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: QueenMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            QueenMessage::Job(spec) => {
                assert_eq!(spec.job_run_id, "run-123");
                assert_eq!(spec.task, "Fix the bug");
            }
            _ => panic!("expected Job"),
        }
    }

    #[test]
    fn test_queen_cancel_message() {
        let msg = QueenMessage::Cancel {};
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("cancel"));
        let parsed: QueenMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, QueenMessage::Cancel {}));
    }

    #[test]
    fn test_drone_result_message_roundtrip() {
        let msg = DroneMessage::Result(DroneOutput {
            exit_code: 0,
            conversation: serde_json::json!({"messages": []}),
            artifacts: vec!["artifact-1".to_string()],
            git_refs: GitRefs {
                branch: Some("feat/thing".to_string()),
                pr_url: Some("https://github.com/user/repo/pull/1".to_string()),
            },
        });
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: DroneMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            DroneMessage::Result(output) => {
                assert_eq!(output.exit_code, 0);
                assert_eq!(output.artifacts.len(), 1);
                assert!(output.git_refs.pr_url.is_some());
            }
            _ => panic!("expected Result"),
        }
    }

    #[test]
    fn test_drone_auth_request() {
        let msg = DroneMessage::AuthRequest(AuthRequest {
            url: "https://claude.ai/auth/xyz".to_string(),
            message: "Please approve".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: DroneMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, DroneMessage::AuthRequest(_)));
    }

    #[test]
    fn test_drone_progress() {
        let msg = DroneMessage::Progress(Progress {
            status: "running".to_string(),
            detail: "Implementing feature".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: DroneMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, DroneMessage::Progress(_)));
    }

    #[test]
    fn test_drone_error() {
        let msg = DroneMessage::Error(DroneError {
            message: "something broke".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: DroneMessage = serde_json::from_str(&json).unwrap();
        match parsed {
            DroneMessage::Error(e) => assert_eq!(e.message, "something broke"),
            _ => panic!("expected Error"),
        }
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd src/drone-sdk && cargo test protocol::tests`
Expected: ALL PASS (6 tests)

- [ ] **Step 3: Commit**

```bash
git add src/drone-sdk/
git commit -m "feat(drone-sdk): add protocol types with JSON-line serialization"
```

---

### Task 3: DroneRunner trait

**Files:**
- Modify: `src/drone-sdk/src/runner.rs`

- [ ] **Step 1: Define the trait**

Replace `src/drone-sdk/src/runner.rs`:

```rust
use async_trait::async_trait;

use crate::protocol::{DroneEnvironment, DroneOutput, JobSpec};
use crate::harness::QueenChannel;

/// Trait that every drone binary implements.
///
/// The harness calls these methods in order:
/// 1. `setup` — create isolated environment (temp dirs, extract config, clone repo)
/// 2. `execute` — run the agent CLI, communicate with Queen via channel
/// 3. `teardown` — clean up temp dirs and child processes
#[async_trait]
pub trait DroneRunner: Send + Sync {
    /// Prepare the drone's isolated environment.
    async fn setup(&self, job: &JobSpec) -> anyhow::Result<DroneEnvironment>;

    /// Run the agent in the prepared environment.
    /// Use `channel` to send auth requests and progress updates to Queen.
    async fn execute(
        &self,
        env: &DroneEnvironment,
        channel: &QueenChannel,
    ) -> anyhow::Result<DroneOutput>;

    /// Clean up the environment after execution.
    async fn teardown(&self, env: &DroneEnvironment);
}
```

- [ ] **Step 2: Verify build**

Run: `cd src/drone-sdk && cargo check`
Expected: compiles (will fail until harness.rs defines QueenChannel — do that next)

- [ ] **Step 3: Move to Task 4 (harness) before committing — runner depends on QueenChannel**

---

### Task 4: Harness and QueenChannel

**Files:**
- Modify: `src/drone-sdk/src/harness.rs`

- [ ] **Step 1: Implement harness and QueenChannel**

Replace `src/drone-sdk/src/harness.rs`:

```rust
use std::sync::Arc;

use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use crate::protocol::*;
use crate::runner::DroneRunner;

/// Communication channel for sending messages to Queen during execution.
pub struct QueenChannel {
    writer: Arc<Mutex<io::Stdout>>,
    reader: Arc<Mutex<BufReader<io::Stdin>>>,
}

impl QueenChannel {
    fn new() -> Self {
        Self {
            writer: Arc::new(Mutex::new(io::stdout())),
            reader: Arc::new(Mutex::new(BufReader::new(io::stdin()))),
        }
    }

    /// Send a message to Queen.
    async fn send(&self, msg: &DroneMessage) -> anyhow::Result<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        let mut writer = self.writer.lock().await;
        writer.write_all(line.as_bytes()).await?;
        writer.flush().await?;
        Ok(())
    }

    /// Read a message from Queen.
    async fn recv(&self) -> anyhow::Result<QueenMessage> {
        let mut line = String::new();
        let mut reader = self.reader.lock().await;
        reader.read_line(&mut line).await?;
        let msg: QueenMessage = serde_json::from_str(line.trim())?;
        Ok(msg)
    }

    /// Request auth from the human operator via Queen.
    /// Sends an auth_request and blocks until Queen responds.
    pub async fn request_auth(&self, url: &str, message: &str) -> anyhow::Result<bool> {
        self.send(&DroneMessage::AuthRequest(AuthRequest {
            url: url.to_string(),
            message: message.to_string(),
        }))
        .await?;

        // Wait for auth_response from Queen
        let msg = self.recv().await?;
        match msg {
            QueenMessage::AuthResponse(resp) => Ok(resp.approved),
            QueenMessage::Cancel {} => anyhow::bail!("cancelled by queen"),
            _ => anyhow::bail!("unexpected message from queen: expected auth_response"),
        }
    }

    /// Send a progress update to Queen.
    pub async fn progress(&self, status: &str, detail: &str) -> anyhow::Result<()> {
        self.send(&DroneMessage::Progress(Progress {
            status: status.to_string(),
            detail: detail.to_string(),
        }))
        .await
    }
}

/// Main entrypoint for all drone binaries.
///
/// Reads a job spec from stdin (sent by Queen), runs the drone through
/// setup/execute/teardown, and writes the result to stdout.
pub async fn run(runner: impl DroneRunner) -> anyhow::Result<()> {
    let channel = QueenChannel::new();

    // 1. Read job from Queen
    let msg = channel.recv().await?;
    let job = match msg {
        QueenMessage::Job(spec) => spec,
        _ => anyhow::bail!("expected Job message from queen, got: {:?}", msg),
    };

    tracing::info!(job_run_id = %job.job_run_id, "drone starting");

    // 2. Setup environment
    let env = match runner.setup(&job).await {
        Ok(env) => env,
        Err(e) => {
            channel
                .send(&DroneMessage::Error(DroneError {
                    message: format!("setup failed: {e}"),
                }))
                .await?;
            return Err(e);
        }
    };

    channel
        .progress("setup_complete", "environment ready")
        .await?;

    // 3. Execute
    let result = match runner.execute(&env, &channel).await {
        Ok(output) => output,
        Err(e) => {
            channel
                .send(&DroneMessage::Error(DroneError {
                    message: format!("execution failed: {e}"),
                }))
                .await?;
            runner.teardown(&env).await;
            return Err(e);
        }
    };

    // 4. Send result
    channel.send(&DroneMessage::Result(result)).await?;

    // 5. Teardown
    runner.teardown(&env).await;

    tracing::info!(job_run_id = %job.job_run_id, "drone finished");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queen_channel_creates() {
        // Just verify the type constructs without panicking.
        // Actual I/O tests would need mock stdin/stdout.
        let _ = QueenChannel::new();
    }
}
```

- [ ] **Step 2: Run tests and verify build**

Run: `cd src/drone-sdk && cargo test`
Expected: ALL PASS (7 tests — 6 protocol + 1 harness)

Run: `cd src/drone-sdk && cargo check`
Expected: compiles

- [ ] **Step 3: Commit Task 3 and Task 4 together**

```bash
git add src/drone-sdk/
git commit -m "feat(drone-sdk): add DroneRunner trait, harness entrypoint, and QueenChannel"
```

---

### Task 5: Claude drone crate scaffolding

**Files:**
- Create: `src/drones/claude/base/Cargo.toml`
- Create: `src/drones/claude/base/BUCK`
- Create: `src/drones/claude/base/src/main.rs`
- Create: `src/drones/claude/base/config/settings.json`
- Create: `src/drones/claude/base/config/CLAUDE.md`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create Cargo.toml**

Create `src/drones/claude/base/Cargo.toml`:

```toml
[package]
name = "claude-drone"
version = "0.1.0"
edition = "2024"

[dependencies]
drone-sdk = { path = "../../../drone-sdk" }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "process", "io-util", "fs"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

- [ ] **Step 2: Add to workspace**

In root `Cargo.toml`:
```toml
members = ["src/overseer", "src/queen", "src/drone-sdk", "src/drones/claude/base"]
```

- [ ] **Step 3: Create config files**

Create `src/drones/claude/base/config/settings.json`:
```json
{
  "permissions": {
    "allow": [],
    "deny": []
  },
  "model": "sonnet"
}
```

Create `src/drones/claude/base/config/CLAUDE.md`:
```markdown
# Claude Drone Base

You are a Claude Code drone operating within the Kerrigan agentic platform. You execute tasks assigned by the Queen process manager.

## Behavior

- Focus on the assigned task
- Report progress clearly
- Commit work frequently
- Do not modify files outside the workspace unless explicitly instructed
```

- [ ] **Step 4: Create minimal main.rs**

Create `src/drones/claude/base/src/main.rs`:

```rust
fn main() {
    println!("claude-drone placeholder");
}
```

- [ ] **Step 5: Create BUCK file**

Create `src/drones/claude/base/BUCK`:

```python
CLAUDE_DRONE_SRCS = glob(["src/**/*.rs"])

CLAUDE_DRONE_DEPS = [
    "//src/drone-sdk:drone-sdk",
    "//third-party:anyhow",
    "//third-party:async-trait",
    "//third-party:serde",
    "//third-party:serde_json",
    "//third-party:tokio",
    "//third-party:tracing",
    "//third-party:tracing-subscriber",
]

rust_binary(
    name = "claude-drone",
    srcs = CLAUDE_DRONE_SRCS,
    crate_root = "src/main.rs",
    deps = CLAUDE_DRONE_DEPS,
    resources = glob(["config/**"]),
    visibility = ["PUBLIC"],
)

rust_test(
    name = "claude-drone-test",
    srcs = CLAUDE_DRONE_SRCS,
    crate_root = "src/main.rs",
    deps = CLAUDE_DRONE_DEPS,
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 6: Regenerate third-party BUCK and verify**

Run: `cd /home/jackm/repos/kerrigan && ./tools/buckify.sh`
Run: `cargo check -p claude-drone`
Run: `buck2 build root//src/drones/claude/base:claude-drone`

- [ ] **Step 7: Commit**

```bash
git add src/drones/ Cargo.toml Cargo.lock
git commit -m "feat(claude-drone): scaffold claude-drone crate with config files"
```

---

### Task 6: Environment setup (environment.rs)

**Files:**
- Create: `src/drones/claude/base/src/environment.rs`
- Modify: `src/drones/claude/base/src/main.rs`

- [ ] **Step 1: Implement environment setup**

Create `src/drones/claude/base/src/environment.rs`:

```rust
use std::path::{Path, PathBuf};

use drone_sdk::protocol::DroneEnvironment;

/// Embedded config files — baked in at compile time.
const SETTINGS_JSON: &[u8] = include_bytes!("../config/settings.json");
const CLAUDE_MD: &[u8] = include_bytes!("../config/CLAUDE.md");

/// Create an isolated home directory for the Claude Code session.
///
/// Layout:
///   {home}/.claude/settings.json    — from embedded config
///   {home}/CLAUDE.md                — base instructions
///   {home}/workspace/               — cloned repo goes here
///
/// Auth tokens are symlinked from the real user's Claude config
/// so the drone can reuse existing OAuth without re-authenticating.
pub async fn create_home(job_run_id: &str) -> anyhow::Result<DroneEnvironment> {
    let home = PathBuf::from(format!("/tmp/drone-{job_run_id}"));
    let claude_dir = home.join(".claude");
    let workspace = home.join("workspace");

    // Create directories
    tokio::fs::create_dir_all(&claude_dir).await?;
    tokio::fs::create_dir_all(&workspace).await?;

    // Write embedded config
    tokio::fs::write(claude_dir.join("settings.json"), SETTINGS_JSON).await?;
    tokio::fs::write(home.join("CLAUDE.md"), CLAUDE_MD).await?;

    // Symlink auth from real user config if it exists
    let real_credentials = dirs::home_dir()
        .map(|h| h.join(".claude/.credentials.json"))
        .unwrap_or_default();
    if real_credentials.exists() {
        let drone_credentials = claude_dir.join(".credentials.json");
        // Use symlink so auth updates propagate
        if let Err(e) = tokio::fs::symlink(&real_credentials, &drone_credentials).await {
            tracing::warn!(
                error = %e,
                "failed to symlink credentials, drone may need to re-authenticate"
            );
        }
    }

    Ok(DroneEnvironment { home, workspace })
}

/// Clone the target repo into the workspace directory.
pub async fn clone_repo(
    repo_url: &str,
    branch: Option<&str>,
    workspace: &Path,
) -> anyhow::Result<()> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("clone");
    if let Some(b) = branch {
        cmd.arg("--branch").arg(b);
    }
    cmd.arg("--depth").arg("1");
    cmd.arg(repo_url);
    cmd.arg(workspace);

    let status = cmd.status().await?;
    if !status.success() {
        anyhow::bail!("git clone failed with exit code {:?}", status.code());
    }

    Ok(())
}

/// Remove the drone's temporary home directory.
pub async fn cleanup(home: &Path) {
    if let Err(e) = tokio::fs::remove_dir_all(home).await {
        tracing::warn!(error = %e, path = %home.display(), "failed to clean up drone home");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_home_creates_dirs() {
        let env = create_home("test-env-setup").await.expect("create home");

        assert!(env.home.exists());
        assert!(env.home.join(".claude").exists());
        assert!(env.home.join(".claude/settings.json").exists());
        assert!(env.home.join("CLAUDE.md").exists());
        assert!(env.workspace.exists());

        // Verify embedded config content
        let settings = tokio::fs::read_to_string(env.home.join(".claude/settings.json"))
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&settings).unwrap();
        assert!(parsed.get("permissions").is_some());

        // Cleanup
        cleanup(&env.home).await;
        assert!(!env.home.exists());
    }

    #[tokio::test]
    async fn test_cleanup_nonexistent_is_ok() {
        // Should not panic
        cleanup(&PathBuf::from("/tmp/drone-nonexistent-test")).await;
    }
}
```

Note: Add `dirs` crate dependency. In `src/drones/claude/base/Cargo.toml` add `dirs = "6"`. In the BUCK file add `"//third-party:dirs"` to deps. Run `./tools/buckify.sh`.

- [ ] **Step 2: Wire into main.rs**

Add `mod environment;` to `src/drones/claude/base/src/main.rs`.

- [ ] **Step 3: Run tests**

Run: `cd src/drones/claude/base && cargo test environment::tests`
Expected: ALL PASS (2 tests)

- [ ] **Step 4: Commit**

```bash
git add src/drones/ Cargo.toml Cargo.lock
git commit -m "feat(claude-drone): add environment setup with embedded config and auth symlinks"
```

---

### Task 7: ClaudeDrone DroneRunner implementation

**Files:**
- Create: `src/drones/claude/base/src/drone.rs`
- Modify: `src/drones/claude/base/src/main.rs`

- [ ] **Step 1: Implement ClaudeDrone**

Create `src/drones/claude/base/src/drone.rs`:

```rust
use async_trait::async_trait;

use drone_sdk::harness::QueenChannel;
use drone_sdk::protocol::{DroneEnvironment, DroneOutput, GitRefs, JobSpec};
use drone_sdk::runner::DroneRunner;

use crate::environment;

pub struct ClaudeDrone;

#[async_trait]
impl DroneRunner for ClaudeDrone {
    async fn setup(&self, job: &JobSpec) -> anyhow::Result<DroneEnvironment> {
        let env = environment::create_home(&job.job_run_id).await?;

        environment::clone_repo(
            &job.repo_url,
            job.branch.as_deref(),
            &env.workspace,
        )
        .await?;

        Ok(env)
    }

    async fn execute(
        &self,
        env: &DroneEnvironment,
        channel: &QueenChannel,
    ) -> anyhow::Result<DroneOutput> {
        channel.progress("starting", "launching claude CLI").await?;

        // Build claude CLI command
        let mut cmd = tokio::process::Command::new("claude");
        cmd.arg("--print");
        cmd.arg("--output-format").arg("json");
        cmd.arg("--dangerously-skip-permissions");

        // Point Claude at the drone's settings
        let settings_path = env.home.join(".claude/settings.json");
        cmd.arg("--settings").arg(&settings_path);

        // Set HOME so Claude uses the drone's isolated config
        cmd.env("HOME", &env.home);

        // Work in the cloned repo
        cmd.current_dir(&env.workspace);

        // The task is the prompt — read it from a file if long, or pass directly.
        // For now, pass as the prompt argument. The job spec's task field is the prompt.
        // We use --append-system-prompt to include the CLAUDE.md content.
        let claude_md_path = env.home.join("CLAUDE.md");
        cmd.arg("--append-system-prompt-file").arg(&claude_md_path);

        // Capture stdout/stderr
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Pass the task via stdin
        cmd.stdin(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;

        // Write the task prompt to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(job_task_as_bytes(&env).as_bytes()).await?;
            drop(stdin); // Close stdin so claude reads it
        }

        channel.progress("running", "claude is working").await?;

        let output = child.wait_with_output().await?;
        let exit_code = output.status.code().unwrap_or(-1);

        // Parse Claude's JSON output for conversation history
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        let conversation: serde_json::Value =
            serde_json::from_str(&stdout_str).unwrap_or_else(|_| {
                serde_json::json!({
                    "raw_output": stdout_str.to_string(),
                    "parse_error": "could not parse claude output as JSON"
                })
            });

        // Collect git state from workspace
        let git_refs = collect_git_refs(&env.workspace).await;

        Ok(DroneOutput {
            exit_code,
            conversation,
            artifacts: vec![],
            git_refs,
        })
    }

    async fn teardown(&self, env: &DroneEnvironment) {
        environment::cleanup(&env.home).await;
    }
}

/// Read the task prompt. For now the task is stored in a file written during setup,
/// or we can read it from the job spec. Since the job spec isn't available here,
/// we read from a file the setup phase writes.
fn job_task_as_bytes(env: &DroneEnvironment) -> String {
    // The task file is written during setup by the harness.
    // For now, return empty — the actual prompt comes from stdin in the harness.
    // This function exists as a hook for future task file support.
    std::fs::read_to_string(env.home.join(".task"))
        .unwrap_or_default()
}

/// Collect current git branch and any PR URLs from the workspace.
async fn collect_git_refs(workspace: &std::path::Path) -> GitRefs {
    let branch = tokio::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(workspace)
        .output()
        .await
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });

    // Check for recently created PRs via gh CLI
    let pr_url = tokio::process::Command::new("gh")
        .args(["pr", "view", "--json", "url", "-q", ".url"])
        .current_dir(workspace)
        .output()
        .await
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let url = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if url.is_empty() { None } else { Some(url) }
            } else {
                None
            }
        });

    GitRefs { branch, pr_url }
}
```

- [ ] **Step 2: Update setup to write the task file**

In `src/drones/claude/base/src/environment.rs`, add a function:

```rust
/// Write the task prompt to a file in the home directory.
/// The drone reads this during execution.
pub async fn write_task(home: &Path, task: &str) -> anyhow::Result<()> {
    tokio::fs::write(home.join(".task"), task).await?;
    Ok(())
}
```

- [ ] **Step 3: Update ClaudeDrone::setup to write the task**

In `drone.rs`, update the `setup` method to write the task file after creating the environment:

```rust
    async fn setup(&self, job: &JobSpec) -> anyhow::Result<DroneEnvironment> {
        let env = environment::create_home(&job.job_run_id).await?;

        environment::clone_repo(
            &job.repo_url,
            job.branch.as_deref(),
            &env.workspace,
        )
        .await?;

        environment::write_task(&env.home, &job.task).await?;

        Ok(env)
    }
```

- [ ] **Step 4: Wire into main.rs**

Replace `src/drones/claude/base/src/main.rs`:

```rust
mod drone;
mod environment;

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    // Note: tracing goes to stderr so stdout is reserved for the protocol
    drone_sdk::harness::run(drone::ClaudeDrone).await
}
```

- [ ] **Step 5: Verify build**

Run: `cd src/drones/claude/base && cargo check`
Expected: compiles

- [ ] **Step 6: Commit**

```bash
git add src/drones/
git commit -m "feat(claude-drone): implement ClaudeDrone with DroneRunner trait"
```

---

### Task 8: Build verification

**Files:**
- None modified — verification only

- [ ] **Step 1: Run all tests**

Run: `cd src/drone-sdk && cargo test`
Expected: ALL PASS

Run: `cd src/drones/claude/base && cargo test`
Expected: ALL PASS

- [ ] **Step 2: Build with Buck2**

Run: `buck2 build root//src/drone-sdk:drone-sdk`
Expected: BUILD SUCCEEDED

Run: `buck2 build root//src/drones/claude/base:claude-drone`
Expected: BUILD SUCCEEDED

Run: `buck2 build root//...`
Expected: BUILD SUCCEEDED

- [ ] **Step 3: Run clippy**

Run: `cd src/drone-sdk && cargo clippy -- -D warnings`
Run: `cd src/drones/claude/base && cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 4: Run pre-commit hooks**

Run: `buck2 run root//tools:prek -- run --all-files`
Expected: All hooks pass

- [ ] **Step 5: Commit any formatting fixes**

```bash
git add -u
git commit -m "style: apply cargo fmt formatting"
```
