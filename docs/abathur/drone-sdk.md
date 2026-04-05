---
title: Drone SDK
slug: drone-sdk
description: Shared library for drone binaries — DroneRunner trait, JSONL protocol, harness entrypoint
lastmod: 2026-04-05
tags: [drone-sdk, protocol, drones]
sources:
  - path: src/drone-sdk/src/runner.rs
    hash: ""
  - path: src/drone-sdk/src/protocol.rs
    hash: ""
  - path: src/drone-sdk/src/harness.rs
    hash: ""
sections: [drone-runner-trait, protocol, harness, types]
---

# Drone SDK

## Drone Runner Trait

```rust
#[async_trait]
pub trait DroneRunner: Send + Sync {
    async fn setup(&self, job: &JobSpec) -> anyhow::Result<DroneEnvironment>;
    async fn execute(&self, env: &DroneEnvironment, channel: &mut QueenChannel) -> anyhow::Result<DroneOutput>;
    async fn teardown(&self, env: &DroneEnvironment);
}
```

Lifecycle: `setup` (create isolated env, clone repo) → `execute` (run agent, communicate with Queen) → `teardown` (cleanup temp dirs).

## Protocol

Queen ↔ Drone communication uses single-line JSON (JSONL) on stdin/stdout. Each message has a `type` tag and nested `payload`.

**Queen → Drone:**

```rust
pub enum QueenMessage {
    Job(JobSpec),                    // initial task assignment
    AuthResponse(AuthResponse),      // human approval + OAuth code
    Cancel {},                       // abort signal
}

pub struct JobSpec {
    pub job_run_id: String,
    pub repo_url: String,
    pub branch: Option<String>,
    pub task: String,
    pub config: Value,
}

pub struct AuthResponse {
    pub approved: bool,
    pub code: Option<String>,
}
```

**Drone → Queen:**

```rust
pub enum DroneMessage {
    AuthRequest(AuthRequest),        // request human to visit OAuth URL
    Progress(Progress),              // status update
    Result(DroneOutput),             // terminal success
    Error(DroneError),               // terminal failure
}

pub struct DroneOutput {
    pub exit_code: i32,
    pub conversation: Value,         // full chat history JSON
    pub artifacts: Vec<String>,
    pub git_refs: GitRefs,
    pub session_jsonl_gz: Option<String>,  // gzipped + base64
}

pub struct GitRefs {
    pub branch: Option<String>,
    pub pr_url: Option<String>,
    pub pr_required: bool,           // default true
}
```

## Harness

```rust
pub async fn run(runner: impl DroneRunner) -> anyhow::Result<()>
```

1. Create `QueenChannel` (stdin/stdout)
2. Receive `Job` message
3. `runner.setup(job)` → `DroneEnvironment`
4. Send `Progress("setup_complete")`
5. `runner.execute(env, channel)` → `DroneOutput`
6. Send `Result` message
7. `runner.teardown(env)`

On setup failure: sends `Error`, returns. On execute failure: sends `Error`, calls teardown, returns. Teardown always runs.

`QueenChannel` methods: `send(msg)`, `recv() -> QueenMessage`, `request_auth(url, message) -> AuthResponse`, `progress(status, detail)`.

## Types

```rust
pub struct DroneEnvironment {
    pub home: PathBuf,       // temporary isolated home
    pub workspace: PathBuf,  // cloned repo directory
}
```
