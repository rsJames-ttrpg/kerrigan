# Queen Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build Queen, the Hatchery process manager that registers with Overseer, manages drone lifecycles, and provides operator notifications.

**Architecture:** Actor model on tokio tasks + mpsc channels. Actors: Registrar (boot), Heartbeat (periodic), Poller (jobs), Supervisor (drones), CreepManager (sidecar). Shared OverseerClient for HTTP. Notifier trait with log impl.

**Tech Stack:** Rust (edition 2024), tokio, reqwest, clap, serde, toml, tracing, async-trait, chrono

---

## File Structure

```
src/queen/
  Cargo.toml                # New crate manifest
  BUCK                      # Buck2 build target
  src/
    main.rs                 # Entry: load config, boot actors, await shutdown
    config.rs               # hatchery.toml + clap + env
    overseer_client.rs      # HTTP client wrapping Overseer API
    actors/
      mod.rs                # Actor re-exports
      registrar.rs          # One-shot: register with Overseer
      heartbeat.rs          # Periodic: send heartbeat
      poller.rs             # Periodic: fetch jobs, send spawn requests
      supervisor.rs         # Core: manage drone processes
      creep_manager.rs      # Manage Creep sidecar process
    notifier/
      mod.rs                # Notifier trait + QueenEvent enum
      log.rs                # LogNotifier (tracing-based)
    messages.rs             # SpawnRequest, StatusQuery, StatusResponse
```

---

### Task 1: Crate scaffolding and workspace setup

**Files:**
- Create: `src/queen/Cargo.toml`
- Create: `src/queen/BUCK`
- Create: `src/queen/src/main.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create Cargo.toml for queen crate**

Create `src/queen/Cargo.toml`:

```toml
[package]
name = "queen"
version = "0.1.0"
edition = "2024"

[dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal", "process", "time"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
clap = { version = "4", features = ["derive", "env"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
async-trait = "0.1"
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
```

- [ ] **Step 2: Add queen to workspace members**

In root `Cargo.toml`, change:
```toml
members = ["src/overseer"]
```
to:
```toml
members = ["src/overseer", "src/queen"]
```

- [ ] **Step 3: Create minimal main.rs**

Create `src/queen/src/main.rs`:

```rust
fn main() {
    println!("queen placeholder");
}
```

- [ ] **Step 4: Create BUCK file**

Create `src/queen/BUCK`:

```python
QUEEN_SRCS = glob(["src/**/*.rs"])

QUEEN_DEPS = [
    "//third-party:anyhow",
    "//third-party:async-trait",
    "//third-party:chrono",
    "//third-party:clap",
    "//third-party:reqwest",
    "//third-party:serde",
    "//third-party:serde_json",
    "//third-party:tokio",
    "//third-party:toml",
    "//third-party:tracing",
    "//third-party:tracing-subscriber",
]

rust_binary(
    name = "queen",
    srcs = QUEEN_SRCS,
    crate_root = "src/main.rs",
    deps = QUEEN_DEPS,
    visibility = ["PUBLIC"],
)

rust_test(
    name = "queen-test",
    srcs = QUEEN_SRCS,
    crate_root = "src/main.rs",
    deps = QUEEN_DEPS,
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 5: Run buckify to regenerate third-party BUCK with new deps**

Run: `cd /home/jackm/repos/kerrigan && cargo add reqwest --features json,rustls-tls -p queen && cargo add clap --features derive,env -p queen && ./tools/buckify.sh`

Note: `reqwest` and `clap` are new to the workspace — reindeer needs to see them. Other deps (tokio, serde, etc.) are already present from overseer.

- [ ] **Step 6: Verify build**

Run: `buck2 build root//src/queen:queen`
Expected: BUILD SUCCEEDED

Run: `cd src/queen && cargo check`
Expected: compiles

- [ ] **Step 7: Commit**

```bash
git add src/queen/ Cargo.toml Cargo.lock third-party/
git commit -m "feat(queen): scaffold queen crate with workspace and Buck2 setup"
```

---

### Task 2: Configuration (config.rs)

**Files:**
- Create: `src/queen/src/config.rs`
- Modify: `src/queen/src/main.rs`

- [ ] **Step 1: Write failing test for config loading**

Create `src/queen/src/config.rs`:

```rust
use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "queen", about = "Kerrigan Hatchery process manager")]
pub struct Cli {
    /// Path to hatchery.toml config file
    #[arg(default_value = "hatchery.toml")]
    pub config: PathBuf,

    /// Hatchery name (overrides config file)
    #[arg(long, env = "QUEEN_NAME")]
    pub name: Option<String>,

    /// Overseer URL (overrides config file)
    #[arg(long, env = "QUEEN_OVERSEER_URL")]
    pub overseer_url: Option<String>,

    /// Max concurrent drones (overrides config file)
    #[arg(long, env = "QUEEN_MAX_CONCURRENCY")]
    pub max_concurrency: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub queen: QueenConfig,
    #[serde(default)]
    pub creep: CreepConfig,
    #[serde(default)]
    pub notifications: NotificationConfig,
}

#[derive(Debug, Deserialize)]
pub struct QueenConfig {
    pub name: String,
    #[serde(default = "default_overseer_url")]
    pub overseer_url: String,
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval: u64,
    #[serde(default = "default_poll_interval")]
    pub poll_interval: u64,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: i32,
    #[serde(default = "default_drone_timeout")]
    pub drone_timeout: String,
    #[serde(default = "default_stall_threshold")]
    pub stall_threshold: u64,
}

#[derive(Debug, Deserialize)]
pub struct CreepConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_creep_binary")]
    pub binary: String,
    #[serde(default = "default_health_port")]
    pub health_port: u16,
    #[serde(default = "default_restart_delay")]
    pub restart_delay: u64,
}

impl Default for CreepConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            binary: "./creep".to_string(),
            health_port: 9090,
            restart_delay: 5,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct NotificationConfig {
    #[serde(default = "default_backend")]
    pub backend: String,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            backend: "log".to_string(),
        }
    }
}

fn default_overseer_url() -> String { "http://localhost:3100".to_string() }
fn default_heartbeat_interval() -> u64 { 30 }
fn default_poll_interval() -> u64 { 10 }
fn default_max_concurrency() -> i32 { 4 }
fn default_drone_timeout() -> String { "2h".to_string() }
fn default_stall_threshold() -> u64 { 300 }
fn default_true() -> bool { true }
fn default_creep_binary() -> String { "./creep".to_string() }
fn default_health_port() -> u16 { 9090 }
fn default_restart_delay() -> u64 { 5 }
fn default_backend() -> String { "log".to_string() }

impl Config {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Apply CLI overrides to the loaded config.
    pub fn apply_overrides(&mut self, cli: &Cli) {
        if let Some(ref name) = cli.name {
            self.queen.name = name.clone();
        }
        if let Some(ref url) = cli.overseer_url {
            self.queen.overseer_url = url.clone();
        }
        if let Some(max) = cli.max_concurrency {
            self.queen.max_concurrency = max;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let toml_str = r#"
[queen]
name = "test-hatchery"
"#;
        let config: Config = toml::from_str(toml_str).expect("parse");
        assert_eq!(config.queen.name, "test-hatchery");
        assert_eq!(config.queen.overseer_url, "http://localhost:3100");
        assert_eq!(config.queen.heartbeat_interval, 30);
        assert_eq!(config.queen.max_concurrency, 4);
        assert!(config.creep.enabled);
        assert_eq!(config.notifications.backend, "log");
    }

    #[test]
    fn test_parse_full_config() {
        let toml_str = r#"
[queen]
name = "rpi-1"
overseer_url = "http://overseer:3100"
heartbeat_interval = 15
poll_interval = 5
max_concurrency = 2
drone_timeout = "1h"
stall_threshold = 120

[creep]
enabled = false
binary = "/opt/creep"
health_port = 8080
restart_delay = 10

[notifications]
backend = "log"
"#;
        let config: Config = toml::from_str(toml_str).expect("parse");
        assert_eq!(config.queen.name, "rpi-1");
        assert_eq!(config.queen.overseer_url, "http://overseer:3100");
        assert_eq!(config.queen.heartbeat_interval, 15);
        assert_eq!(config.queen.max_concurrency, 2);
        assert!(!config.creep.enabled);
        assert_eq!(config.creep.binary, "/opt/creep");
    }

    #[test]
    fn test_cli_overrides() {
        let toml_str = r#"
[queen]
name = "original"
overseer_url = "http://original:3100"
max_concurrency = 1
"#;
        let mut config: Config = toml::from_str(toml_str).expect("parse");
        let cli = Cli {
            config: PathBuf::from("unused"),
            name: Some("overridden".to_string()),
            overseer_url: Some("http://new:3100".to_string()),
            max_concurrency: Some(8),
        };
        config.apply_overrides(&cli);
        assert_eq!(config.queen.name, "overridden");
        assert_eq!(config.queen.overseer_url, "http://new:3100");
        assert_eq!(config.queen.max_concurrency, 8);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd src/queen && cargo test config::tests`
Expected: ALL PASS (3 tests)

- [ ] **Step 3: Update main.rs to use config**

Replace `src/queen/src/main.rs`:

```rust
mod config;

use clap::Parser;
use config::{Cli, Config};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let mut config = Config::load(&cli.config)?;
    config.apply_overrides(&cli);

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!(name = %config.queen.name, "queen starting");

    // Actors will be started here in later tasks.
    // For now, just await Ctrl+C.
    tokio::signal::ctrl_c().await?;
    tracing::info!("queen shutting down");

    Ok(())
}
```

- [ ] **Step 4: Verify build**

Run: `cd src/queen && cargo check`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add src/queen/src/
git commit -m "feat(queen): add configuration with TOML + clap CLI + env var overrides"
```

---

### Task 3: Messages and Notifier

**Files:**
- Create: `src/queen/src/messages.rs`
- Create: `src/queen/src/notifier/mod.rs`
- Create: `src/queen/src/notifier/log.rs`
- Modify: `src/queen/src/main.rs`

- [ ] **Step 1: Create messages.rs**

Create `src/queen/src/messages.rs`:

```rust
use serde_json::Value;

/// Request from Poller to Supervisor to spawn a drone.
#[derive(Debug, Clone)]
pub struct SpawnRequest {
    pub job_run_id: String,
    pub drone_type: String,
    pub job_config: Value,
}

/// Query from Heartbeat to Supervisor for current status.
#[derive(Debug)]
pub struct StatusQuery;

/// Response from Supervisor to Heartbeat with current counts.
#[derive(Debug, Clone)]
pub struct StatusResponse {
    pub active_drones: i32,
    pub queued_jobs: i32,
}
```

- [ ] **Step 2: Create notifier trait and QueenEvent**

Create `src/queen/src/notifier/mod.rs`:

```rust
pub mod log;

use async_trait::async_trait;

#[derive(Debug, Clone)]
pub enum QueenEvent {
    HatcheryRegistered { name: String, id: String },
    DroneSpawned { job_run_id: String, drone_type: String },
    DroneCompleted { job_run_id: String, exit_code: i32 },
    DroneFailed { job_run_id: String, error: String },
    DroneStalled { job_run_id: String, last_activity_secs: u64 },
    DroneTimedOut { job_run_id: String },
    CreepStarted,
    CreepDied { restart_in_secs: u64 },
    ShuttingDown,
}

#[async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, event: QueenEvent);
}
```

- [ ] **Step 3: Create LogNotifier**

Create `src/queen/src/notifier/log.rs`:

```rust
use async_trait::async_trait;
use super::{Notifier, QueenEvent};

pub struct LogNotifier;

#[async_trait]
impl Notifier for LogNotifier {
    async fn notify(&self, event: QueenEvent) {
        match event {
            QueenEvent::HatcheryRegistered { name, id } => {
                tracing::info!(%name, %id, "hatchery registered with overseer");
            }
            QueenEvent::DroneSpawned { job_run_id, drone_type } => {
                tracing::info!(%job_run_id, %drone_type, "drone spawned");
            }
            QueenEvent::DroneCompleted { job_run_id, exit_code } => {
                tracing::info!(%job_run_id, %exit_code, "drone completed");
            }
            QueenEvent::DroneFailed { job_run_id, error } => {
                tracing::warn!(%job_run_id, %error, "drone failed");
            }
            QueenEvent::DroneStalled { job_run_id, last_activity_secs } => {
                tracing::warn!(%job_run_id, %last_activity_secs, "drone stalled");
            }
            QueenEvent::DroneTimedOut { job_run_id } => {
                tracing::warn!(%job_run_id, "drone timed out");
            }
            QueenEvent::CreepStarted => {
                tracing::info!("creep sidecar started");
            }
            QueenEvent::CreepDied { restart_in_secs } => {
                tracing::warn!(%restart_in_secs, "creep sidecar died, restarting");
            }
            QueenEvent::ShuttingDown => {
                tracing::info!("queen shutting down");
            }
        }
    }
}
```

- [ ] **Step 4: Wire modules into main.rs**

Add to the top of `src/queen/src/main.rs` after `mod config;`:

```rust
mod messages;
mod notifier;
```

- [ ] **Step 5: Verify build**

Run: `cd src/queen && cargo check`
Expected: compiles

- [ ] **Step 6: Commit**

```bash
git add src/queen/src/
git commit -m "feat(queen): add messages, Notifier trait, and LogNotifier"
```

---

### Task 4: OverseerClient

**Files:**
- Create: `src/queen/src/overseer_client.rs`
- Modify: `src/queen/src/main.rs`

- [ ] **Step 1: Create OverseerClient**

Create `src/queen/src/overseer_client.rs`:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

/// HTTP client for the Overseer API.
///
/// `hatchery_id` is set after registration and read by other actors.
#[derive(Clone)]
pub struct OverseerClient {
    base_url: String,
    client: reqwest::Client,
    hatchery_id: Arc<RwLock<Option<String>>>,
}

#[derive(Debug, Deserialize)]
pub struct HatcheryResponse {
    pub id: String,
    pub name: String,
    pub status: String,
    pub capabilities: Value,
    pub max_concurrency: i32,
    pub active_drones: i32,
    pub last_heartbeat_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct JobRunResponse {
    pub id: String,
    pub definition_id: String,
    pub parent_id: Option<String>,
    pub status: String,
    pub triggered_by: String,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TaskResponse {
    pub id: String,
    pub run_id: Option<String>,
    pub subject: String,
    pub status: String,
    pub assigned_to: Option<String>,
    pub output: Option<Value>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
struct RegisterRequest {
    name: String,
    capabilities: Value,
    max_concurrency: i32,
}

#[derive(Debug, Serialize)]
struct HeartbeatRequest {
    status: String,
    active_drones: i32,
}

#[derive(Debug, Serialize)]
struct UpdateJobRunRequest {
    status: Option<String>,
    result: Option<Value>,
    error: Option<String>,
}

impl OverseerClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
            hatchery_id: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn hatchery_id(&self) -> Option<String> {
        self.hatchery_id.read().await.clone()
    }

    /// Register this hatchery with Overseer. Sets `hatchery_id` on success.
    pub async fn register(
        &self,
        name: &str,
        capabilities: Value,
        max_concurrency: i32,
    ) -> anyhow::Result<HatcheryResponse> {
        let url = format!("{}/api/hatcheries", self.base_url);
        let body = RegisterRequest {
            name: name.to_string(),
            capabilities,
            max_concurrency,
        };
        let resp = self.client.post(&url).json(&body).send().await?;
        let resp = resp.error_for_status()?;
        let hatchery: HatcheryResponse = resp.json().await?;
        *self.hatchery_id.write().await = Some(hatchery.id.clone());
        Ok(hatchery)
    }

    /// Send a heartbeat to Overseer.
    pub async fn heartbeat(
        &self,
        status: &str,
        active_drones: i32,
    ) -> anyhow::Result<HatcheryResponse> {
        let id = self.require_hatchery_id().await?;
        let url = format!("{}/api/hatcheries/{}/heartbeat", self.base_url, id);
        let body = HeartbeatRequest {
            status: status.to_string(),
            active_drones,
        };
        let resp = self.client.post(&url).json(&body).send().await?;
        let resp = resp.error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Poll for job runs assigned to this hatchery.
    pub async fn poll_jobs(&self) -> anyhow::Result<Vec<JobRunResponse>> {
        let id = self.require_hatchery_id().await?;
        let url = format!(
            "{}/api/hatcheries/{}/jobs?status=running",
            self.base_url, id
        );
        let resp = self.client.get(&url).send().await?;
        let resp = resp.error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Update a job run's status in Overseer.
    pub async fn update_job_run(
        &self,
        job_run_id: &str,
        status: Option<&str>,
        result: Option<Value>,
        error: Option<&str>,
    ) -> anyhow::Result<JobRunResponse> {
        let url = format!("{}/api/jobs/runs/{}", self.base_url, job_run_id);
        let body = UpdateJobRunRequest {
            status: status.map(String::from),
            result,
            error: error.map(String::from),
        };
        let resp = self.client.patch(&url).json(&body).send().await?;
        let resp = resp.error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Get recent tasks for a job run (to check drone activity).
    pub async fn get_tasks_for_run(
        &self,
        run_id: &str,
    ) -> anyhow::Result<Vec<TaskResponse>> {
        let url = format!("{}/api/tasks?run_id={}", self.base_url, run_id);
        let resp = self.client.get(&url).send().await?;
        let resp = resp.error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Deregister this hatchery from Overseer.
    pub async fn deregister(&self) -> anyhow::Result<()> {
        let id = self.require_hatchery_id().await?;
        let url = format!("{}/api/hatcheries/{}", self.base_url, id);
        let resp = self.client.delete(&url).send().await?;
        resp.error_for_status()?;
        Ok(())
    }

    async fn require_hatchery_id(&self) -> anyhow::Result<String> {
        self.hatchery_id
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("hatchery not registered yet"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = OverseerClient::new("http://localhost:3100".to_string());
        assert_eq!(client.base_url, "http://localhost:3100");
    }

    #[tokio::test]
    async fn test_hatchery_id_initially_none() {
        let client = OverseerClient::new("http://localhost:3100".to_string());
        assert!(client.hatchery_id().await.is_none());
    }

    #[tokio::test]
    async fn test_require_hatchery_id_fails_before_register() {
        let client = OverseerClient::new("http://localhost:3100".to_string());
        let result = client.require_hatchery_id().await;
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Wire into main.rs**

Add `mod overseer_client;` to `src/queen/src/main.rs`.

- [ ] **Step 3: Run tests**

Run: `cd src/queen && cargo test overseer_client::tests`
Expected: ALL PASS (3 tests)

- [ ] **Step 4: Commit**

```bash
git add src/queen/src/
git commit -m "feat(queen): add OverseerClient HTTP client for Overseer API"
```

---

### Task 5: Registrar actor

**Files:**
- Create: `src/queen/src/actors/mod.rs`
- Create: `src/queen/src/actors/registrar.rs`
- Modify: `src/queen/src/main.rs`

- [ ] **Step 1: Create actors/mod.rs**

Create `src/queen/src/actors/mod.rs`:

```rust
pub mod registrar;
```

- [ ] **Step 2: Create registrar actor**

Create `src/queen/src/actors/registrar.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;

use crate::notifier::{Notifier, QueenEvent};
use crate::overseer_client::OverseerClient;

/// One-shot actor: registers this hatchery with Overseer, retries on failure.
pub async fn run(
    client: OverseerClient,
    name: String,
    max_concurrency: i32,
    notifier: Arc<dyn Notifier>,
) -> anyhow::Result<()> {
    let capabilities = serde_json::json!({});

    loop {
        match client.register(&name, capabilities.clone(), max_concurrency).await {
            Ok(hatchery) => {
                notifier
                    .notify(QueenEvent::HatcheryRegistered {
                        name: name.clone(),
                        id: hatchery.id.clone(),
                    })
                    .await;
                return Ok(());
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to register with overseer, retrying in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}
```

- [ ] **Step 3: Wire into main.rs**

Update `src/queen/src/main.rs`:

```rust
mod actors;
mod config;
mod messages;
mod notifier;
mod overseer_client;

use std::sync::Arc;

use clap::Parser;
use config::{Cli, Config};
use notifier::log::LogNotifier;
use overseer_client::OverseerClient;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let mut config = Config::load(&cli.config)?;
    config.apply_overrides(&cli);

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!(name = %config.queen.name, "queen starting");

    let client = OverseerClient::new(config.queen.overseer_url.clone());
    let notifier: Arc<dyn notifier::Notifier> = Arc::new(LogNotifier);

    // Register with Overseer (blocks until successful)
    actors::registrar::run(
        client.clone(),
        config.queen.name.clone(),
        config.queen.max_concurrency,
        notifier.clone(),
    )
    .await?;

    // Await Ctrl+C (other actors will be added in later tasks)
    tokio::signal::ctrl_c().await?;

    notifier.notify(notifier::QueenEvent::ShuttingDown).await;

    // Deregister from Overseer
    if let Err(e) = client.deregister().await {
        tracing::warn!(error = %e, "failed to deregister from overseer");
    }

    Ok(())
}
```

- [ ] **Step 4: Verify build**

Run: `cd src/queen && cargo check`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add src/queen/src/
git commit -m "feat(queen): add Registrar actor with retry and main boot sequence"
```

---

### Task 6: Heartbeat actor

**Files:**
- Create: `src/queen/src/actors/heartbeat.rs`
- Modify: `src/queen/src/actors/mod.rs`
- Modify: `src/queen/src/main.rs`

- [ ] **Step 1: Create heartbeat actor**

Create `src/queen/src/actors/heartbeat.rs`:

```rust
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::messages::{StatusQuery, StatusResponse};
use crate::overseer_client::OverseerClient;

/// Periodic actor: sends heartbeats to Overseer with current drone status.
///
/// Queries the Supervisor for active/queued counts via a channel, then
/// sends the heartbeat to Overseer.
pub async fn run(
    client: OverseerClient,
    interval_secs: u64,
    status_tx: mpsc::Sender<(StatusQuery, oneshot::Sender<StatusResponse>)>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));

    loop {
        ticker.tick().await;

        // Ask Supervisor for current status
        let (resp_tx, resp_rx) = oneshot::channel();
        if status_tx.send((StatusQuery, resp_tx)).await.is_err() {
            tracing::warn!("supervisor channel closed, stopping heartbeat");
            return;
        }

        let status_resp = match resp_rx.await {
            Ok(resp) => resp,
            Err(_) => {
                tracing::warn!("supervisor did not respond to status query");
                continue;
            }
        };

        let status = "online";
        if let Err(e) = client.heartbeat(status, status_resp.active_drones).await {
            tracing::warn!(error = %e, "heartbeat failed");
        }
    }
}
```

- [ ] **Step 2: Add to actors/mod.rs**

Add to `src/queen/src/actors/mod.rs`:

```rust
pub mod heartbeat;
```

- [ ] **Step 3: Wire into main.rs**

After the registrar call in `main.rs`, add:

```rust
    // Channel for Heartbeat -> Supervisor status queries
    let (status_query_tx, status_query_rx) = tokio::sync::mpsc::channel(8);

    // Start Heartbeat actor
    let heartbeat_client = client.clone();
    let heartbeat_interval = config.queen.heartbeat_interval;
    tokio::spawn(async move {
        actors::heartbeat::run(heartbeat_client, heartbeat_interval, status_query_tx).await;
    });
```

Note: `status_query_rx` will be consumed by the Supervisor in a later task. For now, store it with `let _status_query_rx = status_query_rx;` so it compiles. The heartbeat will log warnings about the channel until the Supervisor is connected.

Actually, a simpler approach: don't spawn heartbeat until the Supervisor exists. Instead, just add the import and verify the module compiles. The full wiring happens in Task 9 (main.rs integration).

- [ ] **Step 4: Verify build**

Run: `cd src/queen && cargo check`
Expected: compiles

- [ ] **Step 5: Commit**

```bash
git add src/queen/src/
git commit -m "feat(queen): add Heartbeat actor with periodic Overseer updates"
```

---

### Task 7: Poller actor

**Files:**
- Create: `src/queen/src/actors/poller.rs`
- Modify: `src/queen/src/actors/mod.rs`

- [ ] **Step 1: Create poller actor**

Create `src/queen/src/actors/poller.rs`:

```rust
use std::collections::HashSet;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::messages::SpawnRequest;
use crate::overseer_client::OverseerClient;

/// Periodic actor: polls Overseer for jobs assigned to this hatchery,
/// sends spawn requests to the Supervisor for new ones.
pub async fn run(
    client: OverseerClient,
    interval_secs: u64,
    spawn_tx: mpsc::Sender<SpawnRequest>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
    let mut known_runs: HashSet<String> = HashSet::new();

    loop {
        ticker.tick().await;

        let runs = match client.poll_jobs().await {
            Ok(runs) => runs,
            Err(e) => {
                tracing::warn!(error = %e, "failed to poll jobs from overseer");
                continue;
            }
        };

        for run in runs {
            if known_runs.contains(&run.id) {
                continue;
            }

            // Extract drone_type from the job definition config.
            // For now, the drone_type is expected in the triggered_by field
            // or job config. This will evolve when drone registry is built.
            let drone_type = run.triggered_by.clone();

            let request = SpawnRequest {
                job_run_id: run.id.clone(),
                drone_type,
                job_config: run.result.unwrap_or(serde_json::json!({})),
            };

            if spawn_tx.send(request).await.is_err() {
                tracing::warn!("supervisor channel closed, stopping poller");
                return;
            }

            known_runs.insert(run.id);
        }
    }
}
```

- [ ] **Step 2: Add to actors/mod.rs**

Add to `src/queen/src/actors/mod.rs`:

```rust
pub mod poller;
```

- [ ] **Step 3: Verify build**

Run: `cd src/queen && cargo check`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add src/queen/src/
git commit -m "feat(queen): add Poller actor to fetch jobs from Overseer"
```

---

### Task 8: Supervisor actor

**Files:**
- Create: `src/queen/src/actors/supervisor.rs`
- Modify: `src/queen/src/actors/mod.rs`

- [ ] **Step 1: Create supervisor actor**

Create `src/queen/src/actors/supervisor.rs`:

```rust
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};

use crate::messages::{SpawnRequest, StatusQuery, StatusResponse};
use crate::notifier::{Notifier, QueenEvent};
use crate::overseer_client::OverseerClient;

struct DroneHandle {
    job_run_id: String,
    drone_type: String,
    process: Child,
    started_at: Instant,
    timeout: Duration,
    last_activity: Instant,
}

/// Core actor: manages drone processes.
///
/// Receives spawn requests from the Poller, monitors running drones,
/// enforces timeouts, and reports results to Overseer.
pub async fn run(
    client: OverseerClient,
    notifier: Arc<dyn Notifier>,
    max_concurrency: i32,
    default_timeout: Duration,
    stall_threshold: Duration,
    mut spawn_rx: mpsc::Receiver<SpawnRequest>,
    mut status_rx: mpsc::Receiver<(StatusQuery, oneshot::Sender<StatusResponse>)>,
) {
    let mut active: HashMap<String, DroneHandle> = HashMap::new();
    let mut queue: VecDeque<SpawnRequest> = VecDeque::new();
    let health_interval = Duration::from_secs(30);
    let mut health_ticker = tokio::time::interval(health_interval);

    loop {
        tokio::select! {
            // New spawn request from Poller
            Some(request) = spawn_rx.recv() => {
                if (active.len() as i32) < max_concurrency {
                    spawn_drone(&client, &notifier, &mut active, request, default_timeout).await;
                } else {
                    tracing::info!(
                        job_run_id = %request.job_run_id,
                        "concurrency limit reached, queueing"
                    );
                    queue.push_back(request);
                }
            }

            // Status query from Heartbeat
            Some((_, resp_tx)) = status_rx.recv() => {
                let _ = resp_tx.send(StatusResponse {
                    active_drones: active.len() as i32,
                    queued_jobs: queue.len() as i32,
                });
            }

            // Periodic health check
            _ = health_ticker.tick() => {
                check_drones(
                    &client,
                    &notifier,
                    &mut active,
                    stall_threshold,
                ).await;

                // Drain queue if slots opened
                while (active.len() as i32) < max_concurrency {
                    if let Some(request) = queue.pop_front() {
                        spawn_drone(&client, &notifier, &mut active, request, default_timeout).await;
                    } else {
                        break;
                    }
                }
            }

            // All channels closed — shut down
            else => {
                tracing::info!("all channels closed, supervisor exiting");
                break;
            }
        }
    }

    // Kill remaining drones
    shutdown_all(&client, &notifier, &mut active).await;
}

async fn spawn_drone(
    client: &OverseerClient,
    notifier: &Arc<dyn Notifier>,
    active: &mut HashMap<String, DroneHandle>,
    request: SpawnRequest,
    default_timeout: Duration,
) {
    tracing::info!(
        job_run_id = %request.job_run_id,
        drone_type = %request.drone_type,
        "spawning drone"
    );

    // Placeholder: spawn a no-op process. Real drone launching comes from the
    // Drone trait in src/drones/ — not implemented yet. For now we use a sleep
    // process so the supervisor has something to manage.
    let process = match tokio::process::Command::new("sleep")
        .arg("infinity")
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            tracing::error!(
                job_run_id = %request.job_run_id,
                error = %e,
                "failed to spawn drone process"
            );
            let _ = client
                .update_job_run(
                    &request.job_run_id,
                    Some("failed"),
                    None,
                    Some(&format!("failed to spawn: {e}")),
                )
                .await;
            notifier
                .notify(QueenEvent::DroneFailed {
                    job_run_id: request.job_run_id,
                    error: e.to_string(),
                })
                .await;
            return;
        }
    };

    let now = Instant::now();
    let handle = DroneHandle {
        job_run_id: request.job_run_id.clone(),
        drone_type: request.drone_type.clone(),
        process,
        started_at: now,
        timeout: default_timeout,
        last_activity: now,
    };

    notifier
        .notify(QueenEvent::DroneSpawned {
            job_run_id: request.job_run_id.clone(),
            drone_type: request.drone_type,
        })
        .await;

    active.insert(request.job_run_id, handle);
}

async fn check_drones(
    client: &OverseerClient,
    notifier: &Arc<dyn Notifier>,
    active: &mut HashMap<String, DroneHandle>,
    stall_threshold: Duration,
) {
    let now = Instant::now();
    let mut completed = Vec::new();

    for (id, handle) in active.iter_mut() {
        // Check if process exited
        match handle.process.try_wait() {
            Ok(Some(status)) => {
                let exit_code = status.code().unwrap_or(-1);
                if status.success() {
                    tracing::info!(job_run_id = %id, "drone process exited successfully");
                    let _ = client
                        .update_job_run(id, Some("completed"), None, None)
                        .await;
                    notifier
                        .notify(QueenEvent::DroneCompleted {
                            job_run_id: id.clone(),
                            exit_code,
                        })
                        .await;
                } else {
                    tracing::warn!(job_run_id = %id, exit_code, "drone process failed");
                    let _ = client
                        .update_job_run(
                            id,
                            Some("failed"),
                            None,
                            Some(&format!("process exited with code {exit_code}")),
                        )
                        .await;
                    notifier
                        .notify(QueenEvent::DroneFailed {
                            job_run_id: id.clone(),
                            error: format!("exit code {exit_code}"),
                        })
                        .await;
                }
                completed.push(id.clone());
                continue;
            }
            Ok(None) => {} // Still running
            Err(e) => {
                tracing::error!(job_run_id = %id, error = %e, "failed to check drone status");
                continue;
            }
        }

        // Check timeout
        if now.duration_since(handle.started_at) > handle.timeout {
            tracing::warn!(job_run_id = %id, "drone timed out, killing");
            let _ = handle.process.kill().await;
            let _ = client
                .update_job_run(id, Some("failed"), None, Some("timed out"))
                .await;
            notifier
                .notify(QueenEvent::DroneTimedOut {
                    job_run_id: id.clone(),
                })
                .await;
            completed.push(id.clone());
            continue;
        }

        // Check stall (activity-based)
        if now.duration_since(handle.last_activity) > stall_threshold {
            notifier
                .notify(QueenEvent::DroneStalled {
                    job_run_id: id.clone(),
                    last_activity_secs: now.duration_since(handle.last_activity).as_secs(),
                })
                .await;
            // Don't kill yet — stall is a warning. A second stall check will
            // still exceed the threshold and the operator can intervene.
        }

        // Update last_activity from Overseer task data
        if let Ok(tasks) = client.get_tasks_for_run(id).await {
            if let Some(latest) = tasks.last() {
                if let Ok(updated) = chrono::DateTime::parse_from_rfc3339(&latest.updated_at) {
                    let age = chrono::Utc::now() - updated.to_utc();
                    if age.num_seconds() < stall_threshold.as_secs() as i64 {
                        handle.last_activity = now;
                    }
                }
            }
        }
    }

    for id in completed {
        active.remove(&id);
    }
}

async fn shutdown_all(
    client: &OverseerClient,
    notifier: &Arc<dyn Notifier>,
    active: &mut HashMap<String, DroneHandle>,
) {
    for (id, mut handle) in active.drain() {
        tracing::info!(job_run_id = %id, "killing drone for shutdown");
        let _ = handle.process.kill().await;
        let _ = client
            .update_job_run(&id, Some("cancelled"), None, Some("queen shutting down"))
            .await;
    }
    notifier.notify(QueenEvent::ShuttingDown).await;
}
```

- [ ] **Step 2: Add to actors/mod.rs**

Add to `src/queen/src/actors/mod.rs`:

```rust
pub mod supervisor;
```

- [ ] **Step 3: Verify build**

Run: `cd src/queen && cargo check`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add src/queen/src/
git commit -m "feat(queen): add Supervisor actor with drone lifecycle management"
```

---

### Task 9: CreepManager actor

**Files:**
- Create: `src/queen/src/actors/creep_manager.rs`
- Modify: `src/queen/src/actors/mod.rs`

- [ ] **Step 1: Create creep_manager actor**

Create `src/queen/src/actors/creep_manager.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;

use crate::config::CreepConfig;
use crate::notifier::{Notifier, QueenEvent};

/// Background actor: manages the Creep sidecar process.
///
/// Spawns Creep, monitors it, restarts on crash. Non-blocking — drones
/// degrade gracefully if Creep is not available.
pub async fn run(
    config: CreepConfig,
    notifier: Arc<dyn Notifier>,
) {
    if !config.enabled {
        tracing::info!("creep sidecar disabled in config");
        return;
    }

    loop {
        tracing::info!(binary = %config.binary, "starting creep sidecar");

        let mut child = match tokio::process::Command::new(&config.binary)
            .arg("--health-port")
            .arg(config.health_port.to_string())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                tracing::error!(error = %e, "failed to start creep, retrying in {}s", config.restart_delay);
                notifier
                    .notify(QueenEvent::CreepDied {
                        restart_in_secs: config.restart_delay,
                    })
                    .await;
                tokio::time::sleep(Duration::from_secs(config.restart_delay)).await;
                continue;
            }
        };

        notifier.notify(QueenEvent::CreepStarted).await;

        // Wait for creep to exit
        match child.wait().await {
            Ok(status) => {
                tracing::warn!(
                    exit_code = status.code().unwrap_or(-1),
                    "creep exited, restarting in {}s",
                    config.restart_delay
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "error waiting for creep");
            }
        }

        notifier
            .notify(QueenEvent::CreepDied {
                restart_in_secs: config.restart_delay,
            })
            .await;

        tokio::time::sleep(Duration::from_secs(config.restart_delay)).await;
    }
}
```

- [ ] **Step 2: Add to actors/mod.rs**

Add to `src/queen/src/actors/mod.rs`:

```rust
pub mod creep_manager;
```

- [ ] **Step 3: Verify build**

Run: `cd src/queen && cargo check`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add src/queen/src/
git commit -m "feat(queen): add CreepManager actor for sidecar process management"
```

---

### Task 10: Full main.rs integration

**Files:**
- Modify: `src/queen/src/main.rs`

- [ ] **Step 1: Wire all actors into main.rs**

Replace `src/queen/src/main.rs` with the full integrated version:

```rust
mod actors;
mod config;
mod messages;
mod notifier;
mod overseer_client;

use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use config::{Cli, Config};
use notifier::log::LogNotifier;
use overseer_client::OverseerClient;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let mut config = Config::load(&cli.config)?;
    config.apply_overrides(&cli);

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!(name = %config.queen.name, "queen starting");

    let client = OverseerClient::new(config.queen.overseer_url.clone());
    let notifier: Arc<dyn notifier::Notifier> = Arc::new(LogNotifier);

    // 1. Register with Overseer (blocks until successful)
    actors::registrar::run(
        client.clone(),
        config.queen.name.clone(),
        config.queen.max_concurrency,
        notifier.clone(),
    )
    .await?;

    // 2. Start Creep in background (non-blocking)
    let creep_notifier = notifier.clone();
    let creep_config = config.creep;
    tokio::spawn(async move {
        actors::creep_manager::run(creep_config, creep_notifier).await;
    });

    // 3. Channels
    let (spawn_tx, spawn_rx) = tokio::sync::mpsc::channel(32);
    let (status_query_tx, status_query_rx) = tokio::sync::mpsc::channel(8);

    // 4. Start Heartbeat actor
    let heartbeat_client = client.clone();
    let heartbeat_interval = config.queen.heartbeat_interval;
    tokio::spawn(async move {
        actors::heartbeat::run(heartbeat_client, heartbeat_interval, status_query_tx).await;
    });

    // 5. Start Poller actor
    let poller_client = client.clone();
    let poll_interval = config.queen.poll_interval;
    tokio::spawn(async move {
        actors::poller::run(poller_client, poll_interval, spawn_tx).await;
    });

    // 6. Parse timeout duration
    let default_timeout = parse_duration(&config.queen.drone_timeout)?;
    let stall_threshold = Duration::from_secs(config.queen.stall_threshold);

    // 7. Start Supervisor actor (runs until channels close)
    let supervisor_client = client.clone();
    let supervisor_notifier = notifier.clone();
    let max_concurrency = config.queen.max_concurrency;
    let supervisor_handle = tokio::spawn(async move {
        actors::supervisor::run(
            supervisor_client,
            supervisor_notifier,
            max_concurrency,
            default_timeout,
            stall_threshold,
            spawn_rx,
            status_query_rx,
        )
        .await;
    });

    // 8. Await Ctrl+C
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutdown signal received");

    // Dropping the spawn_tx and status_query_tx will close channels,
    // causing Poller and Heartbeat to stop, which closes Supervisor channels.
    // Supervisor will then kill drones and exit.
    // We just need to wait for it.
    drop(supervisor_handle);

    // Deregister from Overseer
    if let Err(e) = client.deregister().await {
        tracing::warn!(error = %e, "failed to deregister from overseer");
    }

    notifier.notify(notifier::QueenEvent::ShuttingDown).await;

    Ok(())
}

fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    let s = s.trim();
    if let Some(hours) = s.strip_suffix('h') {
        Ok(Duration::from_secs(hours.parse::<u64>()? * 3600))
    } else if let Some(mins) = s.strip_suffix('m') {
        Ok(Duration::from_secs(mins.parse::<u64>()? * 60))
    } else if let Some(secs) = s.strip_suffix('s') {
        Ok(Duration::from_secs(secs.parse::<u64>()?))
    } else {
        // Default: treat as seconds
        Ok(Duration::from_secs(s.parse::<u64>()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
    }

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("300s").unwrap(), Duration::from_secs(300));
    }

    #[test]
    fn test_parse_duration_bare_number() {
        assert_eq!(parse_duration("60").unwrap(), Duration::from_secs(60));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd src/queen && cargo test`
Expected: ALL PASS

- [ ] **Step 3: Verify build**

Run: `cd src/queen && cargo check`
Expected: compiles

Run: `buck2 build root//src/queen:queen`
Expected: BUILD SUCCEEDED

- [ ] **Step 4: Commit**

```bash
git add src/queen/src/
git commit -m "feat(queen): integrate all actors in main.rs with full boot and shutdown sequence"
```

---

### Task 11: Build verification and pre-commit

**Files:**
- None modified — verification only

- [ ] **Step 1: Run all cargo tests**

Run: `cd src/queen && cargo test`
Expected: ALL PASS

- [ ] **Step 2: Build with Buck2**

Run: `buck2 build root//src/queen:queen`
Expected: BUILD SUCCEEDED

- [ ] **Step 3: Build entire project**

Run: `buck2 build root//...`
Expected: BUILD SUCCEEDED (both overseer and queen)

- [ ] **Step 4: Run clippy**

Run: `cd src/queen && cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 5: Run pre-commit hooks**

Run: `buck2 run root//tools:prek -- run --all-files`
Expected: All hooks pass

- [ ] **Step 6: Commit any formatting fixes**

If `cargo fmt` made changes:

```bash
git add -u
git commit -m "style: apply cargo fmt formatting"
```
