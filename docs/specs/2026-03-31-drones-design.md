# Drones Design

## Overview

Drones are self-extracting, hermetic agent binaries. Each drone embeds its configuration at build time, creates an isolated environment at runtime, runs an agent CLI as a subprocess, and reports structured results back to Queen via a JSON-line protocol over stdin/stdout.

This spec covers the `drone-sdk` shared crate and the first concrete drone: `claude-drone`.

## Protocol

Bidirectional JSON-line protocol over stdin/stdout between Queen and the drone binary. One JSON object per line.

**Message envelope:**
```json
{"type": "message_type", "payload": { ... }}
```

**Drone -> Queen:**
- `auth_request` — `{ url: String, message: String }` — needs human to visit URL and approve
- `progress` — `{ status: String, detail: String }` — progress update
- `result` — `{ exit_code: i32, conversation: Value, artifacts: [String], git_refs: { branch?: String, pr_url?: String } }` — final structured output with full conversation history
- `error` — `{ message: String }` — fatal error before result

**Queen -> Drone:**
- `job` — `{ job_run_id: String, repo_url: String, branch?: String, task: String, config: Value }` — initial job spec, sent once at start
- `auth_response` — `{ approved: bool }` — human approved/denied auth
- `cancel` — `{}` — stop work

Queen writes `job` immediately after spawning. Drone reads it, does its work, sends messages as needed, finishes with `result` or `error`, then exits.

Full conversation history is required in the result — Evolution Chamber needs the complete transcript (tool calls, messages, context usage) for analysis.

## Drone SDK Crate

Shared library that every drone binary links against. Provides the protocol types, the `DroneRunner` trait, and the harness that handles Queen communication.

### DroneRunner Trait

```rust
#[async_trait]
pub trait DroneRunner: Send + Sync {
    /// Set up the drone environment (create home, extract config, clone repo).
    async fn setup(&self, job: &JobSpec) -> anyhow::Result<DroneEnvironment>;

    /// Run the agent CLI in the prepared environment.
    /// Returns the full conversation history + artifacts.
    async fn execute(&self, env: &DroneEnvironment, channel: &QueenChannel) -> anyhow::Result<DroneOutput>;

    /// Clean up temp directories, processes, etc.
    async fn teardown(&self, env: &DroneEnvironment);
}
```

### Harness

`harness::run(runner)` is the entrypoint every drone binary calls from `main()`:

1. Read `job` message from stdin
2. Call `runner.setup(&job)`
3. Call `runner.execute(&env, &channel)` — runner can send `auth_request` and `progress` messages via `QueenChannel`
4. Call `runner.teardown(&env)`
5. Write `result` message to stdout
6. Exit

### QueenChannel

Handle passed into `execute()` for mid-run communication with Queen:

```rust
pub struct QueenChannel { /* wraps stdout writer + stdin reader */ }

impl QueenChannel {
    /// Request auth from human operator. Blocks until approved/denied.
    pub async fn request_auth(&self, url: &str, message: &str) -> bool;

    /// Send progress update to Queen.
    pub async fn progress(&self, status: &str, detail: &str);
}
```

### Protocol Types

```rust
pub struct JobSpec {
    pub job_run_id: String,
    pub repo_url: String,
    pub branch: Option<String>,
    pub task: String,
    pub config: serde_json::Value,
}

pub struct DroneEnvironment {
    pub home: PathBuf,
    pub workspace: PathBuf,
}

pub struct DroneOutput {
    pub exit_code: i32,
    pub conversation: serde_json::Value,
    pub artifacts: Vec<String>,
    pub git_refs: GitRefs,
}

pub struct GitRefs {
    pub branch: Option<String>,
    pub pr_url: Option<String>,
}
```

## Claude Drone

The first concrete drone. A self-extracting Rust binary that runs Claude Code.

### Setup Phase

1. Create temp home dir (`/tmp/drone-{job_run_id}/`)
2. Write embedded config files into `{home}/.claude/` (settings, skills, MCP configs)
3. Symlink auth paths — `{home}/.claude/auth` -> real `~/.claude/auth` (so Claude Code can reuse existing OAuth tokens)
4. Clone the target repo into `{home}/workspace/`
5. Return `DroneEnvironment { home, workspace }`

### Execute Phase

1. Spawn `claude` CLI as subprocess with `HOME={home}`, working dir = workspace
2. Pass the task from job spec as the initial prompt
3. Monitor the subprocess — forward `auth_request` to Queen if Claude needs login
4. On exit, parse Claude Code's conversation output from session files
5. Collect git state (current branch, any PRs created)
6. Return `DroneOutput` with full conversation history, artifact IDs, git refs

### Teardown Phase

1. Remove temp home dir
2. Clean up any child processes

### Embedded Config

Config files are embedded at compile time via `include_bytes!`:

```rust
const SETTINGS: &[u8] = include_bytes!("../config/settings.json");
const CLAUDE_MD: &[u8] = include_bytes!("../config/CLAUDE.md");
```

Buck2 tracks these files as build inputs — changing a skill file or CLAUDE.md rebuilds the drone binary.

### Config Contents

The `config/` directory for the base Claude drone contains:
- `settings.json` — Claude Code user-level settings (allowed tools, permissions, model preferences)
- `CLAUDE.md` — base instructions for all Claude Code drones
- `skills/` — bundled skill definitions
- `mcp/` — MCP server configurations (Overseer MCP connection, Creep when available)

Subtypes (code-reviewer, feature-builder) embed different config files with specialized instructions and skill sets.

## Buck2 Integration

For v1, drones are plain `rust_binary` targets. Config files are tracked as build inputs so Buck2 rebuilds when they change.

```python
# src/drones/claude/base/BUCK
rust_binary(
    name = "claude-drone",
    srcs = glob(["src/**/*.rs"]),
    crate_root = "src/main.rs",
    deps = ["//src/drone-sdk:drone-sdk"],
    resources = glob(["config/**"]),
    visibility = ["PUBLIC"],
)
```

A custom `drone_package()` rule can be introduced later when bundling becomes more complex (MCP server binaries, multi-file resources).

## Repo Structure

```
src/
  drone-sdk/
    Cargo.toml
    BUCK
    src/
      lib.rs            # Re-exports
      protocol.rs       # Message types, JSON serialization
      harness.rs        # harness::run(runner) entrypoint + QueenChannel
      runner.rs         # DroneRunner trait + DroneEnvironment, DroneOutput
  drones/
    claude/
      base/
        Cargo.toml
        BUCK
        src/
          main.rs       # harness::run(ClaudeDrone)
          drone.rs      # ClaudeDrone implementing DroneRunner
          environment.rs # Temp home setup, symlinks, repo clone
        config/
          settings.json
          CLAUDE.md
          skills/
          mcp/
```

Both `drone-sdk` and `drones/claude/base` are new crates in the Cargo workspace with their own Buck2 targets.

## Future Drone Types

Adding a new agent type (e.g. Gemini) means:
1. Create `src/drones/gemini/base/` with a new binary that links `drone-sdk`
2. Implement `DroneRunner` with Gemini-specific setup (different CLI, different config format, different auth)
3. Embed Gemini-specific config files
4. Different conversation output parsing

The SDK protocol remains the same — Queen doesn't care which agent runs inside.
