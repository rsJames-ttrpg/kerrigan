# Plan 00: Crate Scaffolding

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the two new crate skeletons (`src/runtime/` and `src/drones/native/`) with proper Cargo.toml, BUCK files, and workspace integration. No real logic yet — just the scaffolding that all subsequent plans build on.

**Architecture:** `runtime` is a library crate (no main.rs). `native-drone` is a binary crate that depends on `runtime` and `drone-sdk`.

**Tech Stack:** Rust 2024 edition, Buck2, tokio, serde, drone-sdk

---

### Task 1: Create runtime library crate

**Files:**
- Create: `src/runtime/Cargo.toml`
- Create: `src/runtime/src/lib.rs`
- Create: `src/runtime/BUCK`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "runtime"
version = "0.1.0"
edition = "2024"

[dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time", "process", "io-util"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
anyhow = "1"
tracing = "0.1"
```

- [ ] **Step 2: Create src/lib.rs with module stubs**

```rust
pub mod api;
pub mod tools;
pub mod conversation;
pub mod config;
pub mod event;
pub mod permission;
```

Create empty module files so it compiles:

`src/runtime/src/api/mod.rs`:
```rust
// Multi-provider LLM API client
```

`src/runtime/src/tools/mod.rs`:
```rust
// Tool registry and execution
```

`src/runtime/src/conversation/mod.rs`:
```rust
// Agent loop, session, compaction
```

`src/runtime/src/config.rs`:
```rust
// Runtime configuration types
```

`src/runtime/src/event.rs`:
```rust
// Runtime event types and EventSink trait
```

`src/runtime/src/permission.rs`:
```rust
// Permission policy types
```

- [ ] **Step 3: Create BUCK file**

```python
RUNTIME_SRCS = glob(["src/**/*.rs"])

RUNTIME_DEPS = [
    "//third-party:tokio",
    "//third-party:serde",
    "//third-party:serde_json",
    "//third-party:async-trait",
    "//third-party:anyhow",
    "//third-party:tracing",
]

rust_library(
    name = "runtime",
    srcs = RUNTIME_SRCS,
    deps = RUNTIME_DEPS,
    visibility = ["PUBLIC"],
)

rust_test(
    name = "runtime-test",
    srcs = RUNTIME_SRCS,
    deps = RUNTIME_DEPS,
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 4: Add to workspace**

In root `Cargo.toml`, add `"src/runtime"` to workspace members:

```toml
members = ["src/overseer", "src/queen", "src/drone-sdk", "src/drones/claude/base",
           "src/creep", "src/creep-cli", "src/nydus", "src/kerrigan", "src/evolution",
           "src/runtime"]
```

- [ ] **Step 5: Regenerate third-party BUCK**

Run: `./tools/buckify.sh`

- [ ] **Step 6: Verify build**

Run: `cd src/runtime && cargo check`
Expected: compiles with no errors

Run: `buck2 build root//src/runtime:runtime`
Expected: builds successfully

- [ ] **Step 7: Commit**

```bash
git add src/runtime/ Cargo.toml Cargo.lock third-party/BUCK
git commit -m "scaffold runtime library crate"
```

---

### Task 2: Create native-drone binary crate

**Files:**
- Create: `src/drones/native/Cargo.toml`
- Create: `src/drones/native/src/main.rs`
- Create: `src/drones/native/BUCK`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "native-drone"
version = "0.1.0"
edition = "2024"

[dependencies]
runtime = { path = "../../runtime" }
drone-sdk = { path = "../../drone-sdk" }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time", "process", "io-util"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
toml = "0.8"
```

- [ ] **Step 2: Create src/main.rs with placeholder drone**

```rust
mod drone;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tracing::info!("Native drone starting");
    drone_sdk::harness::run(drone::NativeDrone).await
}
```

Create `src/drones/native/src/drone.rs`:

```rust
use async_trait::async_trait;
use drone_sdk::{
    harness::QueenChannel,
    protocol::{DroneEnvironment, DroneOutput, GitRefs, JobSpec},
    runner::DroneRunner,
};
use serde_json::json;

pub struct NativeDrone;

#[async_trait]
impl DroneRunner for NativeDrone {
    async fn setup(&self, job: &JobSpec) -> anyhow::Result<DroneEnvironment> {
        tracing::info!(run_id = %job.job_run_id, "Setting up native drone");
        let home = std::path::PathBuf::from(format!("/tmp/drone-{}", job.job_run_id));
        tokio::fs::create_dir_all(&home).await?;
        Ok(DroneEnvironment {
            home: home.clone(),
            workspace: home.join("workspace"),
        })
    }

    async fn execute(
        &self,
        env: &DroneEnvironment,
        channel: &mut QueenChannel,
    ) -> anyhow::Result<DroneOutput> {
        channel.progress("started", "native drone placeholder")?;
        tracing::info!("Native drone execute — placeholder");
        Ok(DroneOutput {
            exit_code: 0,
            conversation: json!({}),
            artifacts: vec![],
            git_refs: GitRefs {
                branch: None,
                pr_url: None,
                pr_required: false,
            },
            session_jsonl_gz: None,
        })
    }

    async fn teardown(&self, env: &DroneEnvironment) {
        let _ = tokio::fs::remove_dir_all(&env.home).await;
    }
}
```

**drone-sdk type reference** (from `src/drone-sdk/src/protocol.rs`):
- `DroneEnvironment { home: PathBuf, workspace: PathBuf }`
- `DroneOutput { exit_code: i32, conversation: serde_json::Value, artifacts: Vec<String>, git_refs: GitRefs, session_jsonl_gz: Option<String> }`
- `DroneMessage::Progress(Progress { status: String, detail: Option<String> })`
- `DroneMessage::Error(DroneError { message: String })`
- `QueenChannel.progress(&mut self, &str, &str) -> Result<()>` (sync, not async)
- `QueenChannel.send(&mut self, &DroneMessage) -> Result<()>` (sync, not async)
- `JobSpec.config` is `serde_json::Value`, not `HashMap<String, String>`

- [ ] **Step 3: Create BUCK file**

```python
NATIVE_DRONE_SRCS = glob(["src/**/*.rs"])

NATIVE_DRONE_DEPS = [
    "//src/runtime:runtime",
    "//src/drone-sdk:drone-sdk",
    "//third-party:tokio",
    "//third-party:serde",
    "//third-party:serde_json",
    "//third-party:async-trait",
    "//third-party:anyhow",
    "//third-party:tracing",
    "//third-party:tracing-subscriber",
    "//third-party:toml",
]

rust_binary(
    name = "native-drone",
    srcs = NATIVE_DRONE_SRCS,
    crate_root = "src/main.rs",
    deps = NATIVE_DRONE_DEPS,
    visibility = ["PUBLIC"],
)

rust_test(
    name = "native-drone-test",
    srcs = NATIVE_DRONE_SRCS,
    crate_root = "src/main.rs",
    deps = NATIVE_DRONE_DEPS,
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 4: Add to workspace**

In root `Cargo.toml`, add `"src/drones/native"` to workspace members.

- [ ] **Step 5: Regenerate third-party BUCK**

Run: `./tools/buckify.sh`

- [ ] **Step 6: Verify build**

Run: `cd src/drones/native && cargo check`
Expected: compiles with no errors

Run: `buck2 build root//src/drones/native:native-drone`
Expected: builds successfully

- [ ] **Step 7: Commit**

```bash
git add src/drones/native/ Cargo.toml Cargo.lock third-party/BUCK
git commit -m "scaffold native-drone binary crate"
```
