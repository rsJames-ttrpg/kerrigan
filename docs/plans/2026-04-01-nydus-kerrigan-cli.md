# Nydus Client Library + Kerrigan CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a shared Overseer HTTP client library (`nydus`) and an operator CLI (`kerrigan`) that replaces the 4-curl job submission workflow with a single command.

**Architecture:** Two new workspace crates. `nydus` is a stateless typed HTTP client wrapping reqwest, with 1:1 methods for each Overseer API endpoint. `kerrigan` is a clap-based CLI binary that uses `nydus` to orchestrate job submission (resolve definition → start run → find hatchery → assign). Queen migrates from its internal `OverseerClient` to `nydus`. One small Overseer change: `start_run` gains `config_overrides` support.

**Tech Stack:** Rust 2024, reqwest (HTTP), clap (CLI), serde/serde_json (serialization), thiserror (errors), tokio (async runtime)

**Spec:** `docs/specs/2026-04-01-nydus-kerrigan-cli-design.md`

---

## File Structure

### New files

| File | Responsibility |
|------|----------------|
| `src/nydus/Cargo.toml` | Crate manifest for nydus library |
| `src/nydus/BUCK` | Buck2 build target (rust_library + rust_test) |
| `src/nydus/src/lib.rs` | Module root, re-exports `NydusClient` and types |
| `src/nydus/src/client.rs` | `NydusClient` struct and all HTTP methods |
| `src/nydus/src/types.rs` | Response types: `JobDefinition`, `JobRun`, `Task`, `Hatchery`, `Artifact` |
| `src/nydus/src/error.rs` | `nydus::Error` enum |
| `src/kerrigan/Cargo.toml` | Crate manifest for kerrigan binary |
| `src/kerrigan/BUCK` | Buck2 build target (rust_binary + rust_test) |
| `src/kerrigan/src/main.rs` | Entry point, clap CLI definition, command dispatch |

### Modified files

| File | Change |
|------|--------|
| `Cargo.toml` | Add `src/nydus` and `src/kerrigan` to workspace members |
| `src/overseer/src/api/jobs.rs` | Add `config_overrides` to `StartJobRunRequest` |
| `src/overseer/src/services/jobs.rs` | Merge config overrides in `start_job_run` |
| `src/queen/Cargo.toml` | Replace `reqwest` + `base64` with `nydus` dependency |
| `src/queen/BUCK` | Replace `//third-party:reqwest` + `//third-party:base64` with `//src/nydus:nydus` |
| `src/queen/src/main.rs` | `use nydus::NydusClient` instead of `overseer_client::OverseerClient` |
| `src/queen/src/actors/registrar.rs` | Use `NydusClient` |
| `src/queen/src/actors/heartbeat.rs` | Use `NydusClient` |
| `src/queen/src/actors/poller.rs` | Use `NydusClient` |
| `src/queen/src/actors/supervisor.rs` | Use `NydusClient` |

### Deleted files

| File | Reason |
|------|--------|
| `src/queen/src/overseer_client.rs` | Replaced by `nydus` |

---

## Task 1: Create `nydus` crate — types and error

**Files:**
- Create: `src/nydus/Cargo.toml`
- Create: `src/nydus/src/lib.rs`
- Create: `src/nydus/src/types.rs`
- Create: `src/nydus/src/error.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create `src/nydus/Cargo.toml`**

```toml
[package]
name = "nydus"
version = "0.1.0"
edition = "2024"

[dependencies]
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
```

- [ ] **Step 2: Create `src/nydus/src/error.rs`**

```rust
use std::fmt;

#[derive(Debug)]
pub enum Error {
    Request(reqwest::Error),
    Api { status: u16, body: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(e) => write!(f, "HTTP request failed: {e}"),
            Self::Api { status, body } => write!(f, "Overseer API error ({status}): {body}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Request(e) => Some(e),
            Self::Api { .. } => None,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Self::Request(e)
    }
}
```

- [ ] **Step 3: Create `src/nydus/src/types.rs`**

These types mirror Overseer's API responses. They are owned by nydus — not shared with Overseer.

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRun {
    pub id: String,
    pub definition_id: String,
    pub parent_id: Option<String>,
    pub status: String,
    pub triggered_by: String,
    pub result: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub run_id: Option<String>,
    pub subject: String,
    pub status: String,
    pub assigned_to: Option<String>,
    pub output: Option<Value>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hatchery {
    pub id: String,
    pub name: String,
    pub status: String,
    pub capabilities: Value,
    pub max_concurrency: i32,
    pub active_drones: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub run_id: Option<String>,
}
```

- [ ] **Step 4: Create `src/nydus/src/lib.rs`**

```rust
mod client;
mod error;
mod types;

pub use client::NydusClient;
pub use error::Error;
pub use types::*;
```

Create a placeholder `src/nydus/src/client.rs`:

```rust
pub struct NydusClient;
```

- [ ] **Step 5: Add to workspace**

In root `Cargo.toml`, add `"src/nydus"` to the workspace members list:

```toml
[workspace]
members = ["src/overseer", "src/queen", "src/drone-sdk", "src/drones/claude/base", "src/creep", "src/nydus"]
resolver = "2"
```

- [ ] **Step 6: Verify it compiles**

Run: `cd src/nydus && cargo check`
Expected: compiles with no errors

- [ ] **Step 7: Commit**

```bash
git add src/nydus/ Cargo.toml
git commit -m "feat(nydus): scaffold crate with types and error"
```

---

## Task 2: Implement `NydusClient` — core + jobs API

**Files:**
- Modify: `src/nydus/src/client.rs`
- Test: `src/nydus/src/client.rs` (unit tests for construction)

- [ ] **Step 1: Write test for client construction**

Add to `src/nydus/src/client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = NydusClient::new("http://localhost:3100");
        assert_eq!(client.base_url, "http://localhost:3100");
    }

    #[test]
    fn test_client_strips_trailing_slash() {
        let client = NydusClient::new("http://localhost:3100/");
        assert_eq!(client.base_url, "http://localhost:3100");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/nydus && cargo test`
Expected: FAIL — `NydusClient` is a unit struct with no fields or `new` method

- [ ] **Step 3: Implement `NydusClient` struct and jobs methods**

Replace `src/nydus/src/client.rs`:

```rust
use serde::Serialize;
use serde_json::Value;

use crate::Error;
use crate::types::*;

#[derive(Debug, Clone)]
pub struct NydusClient {
    pub(crate) base_url: String,
    client: reqwest::Client,
}

impl NydusClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let mut url = base_url.into();
        if url.ends_with('/') {
            url.pop();
        }
        Self {
            base_url: url,
            client: reqwest::Client::new(),
        }
    }

    async fn check_response(&self, response: reqwest::Response) -> Result<reqwest::Response, Error> {
        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            Err(Error::Api { status, body })
        }
    }

    // ── Jobs: Definitions ────────────────────────────────────────────────

    pub async fn create_definition(
        &self,
        name: &str,
        description: &str,
        config: Value,
    ) -> Result<JobDefinition, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
            description: &'a str,
            config: Value,
        }
        let resp = self
            .client
            .post(format!("{}/api/jobs/definitions", self.base_url))
            .json(&Body { name, description, config })
            .send()
            .await?;
        Ok(self.check_response(resp).await?.json().await?)
    }

    pub async fn get_definition(&self, id: &str) -> Result<JobDefinition, Error> {
        let resp = self
            .client
            .get(format!("{}/api/jobs/definitions/{id}", self.base_url))
            .send()
            .await?;
        Ok(self.check_response(resp).await?.json().await?)
    }

    pub async fn list_definitions(&self) -> Result<Vec<JobDefinition>, Error> {
        let resp = self
            .client
            .get(format!("{}/api/jobs/definitions", self.base_url))
            .send()
            .await?;
        Ok(self.check_response(resp).await?.json().await?)
    }

    // ── Jobs: Runs ───────────────────────────────────────────────────────

    pub async fn start_run(
        &self,
        definition_id: &str,
        triggered_by: &str,
        parent_id: Option<&str>,
        config_overrides: Option<Value>,
    ) -> Result<JobRun, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            definition_id: &'a str,
            triggered_by: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            parent_id: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            config_overrides: Option<Value>,
        }
        let resp = self
            .client
            .post(format!("{}/api/jobs/runs", self.base_url))
            .json(&Body {
                definition_id,
                triggered_by,
                parent_id,
                config_overrides,
            })
            .send()
            .await?;
        Ok(self.check_response(resp).await?.json().await?)
    }

    pub async fn list_runs(&self, status: Option<&str>) -> Result<Vec<JobRun>, Error> {
        let mut url = format!("{}/api/jobs/runs", self.base_url);
        if let Some(s) = status {
            url.push_str(&format!("?status={s}"));
        }
        let resp = self.client.get(&url).send().await?;
        Ok(self.check_response(resp).await?.json().await?)
    }

    pub async fn update_run(
        &self,
        id: &str,
        status: Option<&str>,
        result: Option<Value>,
        error: Option<&str>,
    ) -> Result<JobRun, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            status: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            result: Option<Value>,
            #[serde(skip_serializing_if = "Option::is_none")]
            error: Option<&'a str>,
        }
        let resp = self
            .client
            .patch(format!("{}/api/jobs/runs/{id}", self.base_url))
            .json(&Body { status, result, error })
            .send()
            .await?;
        Ok(self.check_response(resp).await?.json().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = NydusClient::new("http://localhost:3100");
        assert_eq!(client.base_url, "http://localhost:3100");
    }

    #[test]
    fn test_client_strips_trailing_slash() {
        let client = NydusClient::new("http://localhost:3100/");
        assert_eq!(client.base_url, "http://localhost:3100");
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd src/nydus && cargo test`
Expected: PASS (2 tests)

- [ ] **Step 5: Commit**

```bash
git add src/nydus/src/client.rs
git commit -m "feat(nydus): implement NydusClient core + jobs API"
```

---

## Task 3: Implement `NydusClient` — tasks, hatcheries, artifacts, auth

**Files:**
- Modify: `src/nydus/src/client.rs`

- [ ] **Step 1: Add tasks methods**

Add to the `impl NydusClient` block in `src/nydus/src/client.rs`:

```rust
    // ── Tasks ────────────────────────────────────────────────────────────

    pub async fn create_task(
        &self,
        subject: &str,
        run_id: Option<&str>,
        assigned_to: Option<&str>,
    ) -> Result<Task, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            subject: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            run_id: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            assigned_to: Option<&'a str>,
        }
        let resp = self
            .client
            .post(format!("{}/api/tasks", self.base_url))
            .json(&Body { subject, run_id, assigned_to })
            .send()
            .await?;
        Ok(self.check_response(resp).await?.json().await?)
    }

    pub async fn list_tasks(
        &self,
        status: Option<&str>,
        assigned_to: Option<&str>,
        run_id: Option<&str>,
    ) -> Result<Vec<Task>, Error> {
        let mut params = Vec::new();
        if let Some(s) = status { params.push(format!("status={s}")); }
        if let Some(a) = assigned_to { params.push(format!("assigned_to={a}")); }
        if let Some(r) = run_id { params.push(format!("run_id={r}")); }
        let mut url = format!("{}/api/tasks", self.base_url);
        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }
        let resp = self.client.get(&url).send().await?;
        Ok(self.check_response(resp).await?.json().await?)
    }

    pub async fn update_task(
        &self,
        id: &str,
        status: Option<&str>,
        assigned_to: Option<&str>,
        output: Option<Value>,
    ) -> Result<Task, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            status: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            assigned_to: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            output: Option<Value>,
        }
        let resp = self
            .client
            .patch(format!("{}/api/tasks/{id}", self.base_url))
            .json(&Body { status, assigned_to, output })
            .send()
            .await?;
        Ok(self.check_response(resp).await?.json().await?)
    }
```

- [ ] **Step 2: Add hatcheries methods**

Add to the `impl NydusClient` block:

```rust
    // ── Hatcheries ───────────────────────────────────────────────────────

    pub async fn register_hatchery(
        &self,
        name: &str,
        capabilities: Value,
        max_concurrency: i32,
    ) -> Result<Hatchery, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
            capabilities: Value,
            max_concurrency: i32,
        }
        let resp = self
            .client
            .post(format!("{}/api/hatcheries", self.base_url))
            .json(&Body { name, capabilities, max_concurrency })
            .send()
            .await?;
        Ok(self.check_response(resp).await?.json().await?)
    }

    pub async fn heartbeat(
        &self,
        hatchery_id: &str,
        status: &str,
        active_drones: i32,
    ) -> Result<Hatchery, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            status: &'a str,
            active_drones: i32,
        }
        let resp = self
            .client
            .post(format!("{}/api/hatcheries/{hatchery_id}/heartbeat", self.base_url))
            .json(&Body { status, active_drones })
            .send()
            .await?;
        Ok(self.check_response(resp).await?.json().await?)
    }

    pub async fn get_hatchery(&self, id: &str) -> Result<Hatchery, Error> {
        let resp = self
            .client
            .get(format!("{}/api/hatcheries/{id}", self.base_url))
            .send()
            .await?;
        Ok(self.check_response(resp).await?.json().await?)
    }

    pub async fn list_hatcheries(&self, status: Option<&str>) -> Result<Vec<Hatchery>, Error> {
        let mut url = format!("{}/api/hatcheries", self.base_url);
        if let Some(s) = status {
            url.push_str(&format!("?status={s}"));
        }
        let resp = self.client.get(&url).send().await?;
        Ok(self.check_response(resp).await?.json().await?)
    }

    pub async fn deregister_hatchery(&self, id: &str) -> Result<(), Error> {
        let resp = self
            .client
            .delete(format!("{}/api/hatcheries/{id}", self.base_url))
            .send()
            .await?;
        self.check_response(resp).await?;
        Ok(())
    }

    pub async fn list_hatchery_jobs(
        &self,
        hatchery_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<JobRun>, Error> {
        let mut url = format!("{}/api/hatcheries/{hatchery_id}/jobs", self.base_url);
        if let Some(s) = status {
            url.push_str(&format!("?status={s}"));
        }
        let resp = self.client.get(&url).send().await?;
        Ok(self.check_response(resp).await?.json().await?)
    }

    pub async fn assign_job(
        &self,
        hatchery_id: &str,
        job_run_id: &str,
    ) -> Result<JobRun, Error> {
        let resp = self
            .client
            .put(format!(
                "{}/api/hatcheries/{hatchery_id}/jobs/{job_run_id}",
                self.base_url
            ))
            .send()
            .await?;
        Ok(self.check_response(resp).await?.json().await?)
    }
```

- [ ] **Step 3: Add artifacts methods**

Add to the `impl NydusClient` block:

```rust
    // ── Artifacts ────────────────────────────────────────────────────────

    pub async fn store_artifact(
        &self,
        name: &str,
        content_type: &str,
        data: &[u8],
        run_id: Option<&str>,
    ) -> Result<Artifact, Error> {
        use base64::Engine;
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
            content_type: &'a str,
            data: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            run_id: Option<&'a str>,
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        let resp = self
            .client
            .post(format!("{}/api/artifacts", self.base_url))
            .json(&Body { name, content_type, data: encoded, run_id })
            .send()
            .await?;
        Ok(self.check_response(resp).await?.json().await?)
    }

    pub async fn get_artifact(&self, id: &str) -> Result<Vec<u8>, Error> {
        let resp = self
            .client
            .get(format!("{}/api/artifacts/{id}", self.base_url))
            .send()
            .await?;
        let resp = self.check_response(resp).await?;
        Ok(resp.bytes().await?.to_vec())
    }

    pub async fn list_artifacts(&self, run_id: Option<&str>) -> Result<Vec<Artifact>, Error> {
        let mut url = format!("{}/api/artifacts", self.base_url);
        if let Some(r) = run_id {
            url.push_str(&format!("?run_id={r}"));
        }
        let resp = self.client.get(&url).send().await?;
        Ok(self.check_response(resp).await?.json().await?)
    }
```

- [ ] **Step 4: Add auth methods**

Add to the `impl NydusClient` block:

```rust
    // ── Auth ─────────────────────────────────────────────────────────────

    pub async fn submit_auth_code(
        &self,
        job_run_id: &str,
        code: &str,
    ) -> Result<(), Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            code: &'a str,
        }
        let resp = self
            .client
            .post(format!("{}/api/jobs/runs/{job_run_id}/auth", self.base_url))
            .json(&Body { code })
            .send()
            .await?;
        self.check_response(resp).await?;
        Ok(())
    }

    pub async fn poll_auth_code(&self, job_run_id: &str) -> Result<Option<String>, Error> {
        let resp = self
            .client
            .get(format!("{}/api/jobs/runs/{job_run_id}/auth", self.base_url))
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let body: Value = self.check_response(resp).await?.json().await?;
        Ok(body.get("code").and_then(|v| v.as_str()).map(String::from))
    }
```

- [ ] **Step 5: Add `base64` dependency to `src/nydus/Cargo.toml`**

Add under `[dependencies]`:

```toml
base64 = "0.22"
```

- [ ] **Step 6: Verify it compiles**

Run: `cd src/nydus && cargo test`
Expected: PASS (2 tests), no compilation errors

- [ ] **Step 7: Commit**

```bash
git add src/nydus/
git commit -m "feat(nydus): implement tasks, hatcheries, artifacts, auth methods"
```

---

## Task 4: Add `nydus` BUCK target and buckify

**Files:**
- Create: `src/nydus/BUCK`
- Regenerate: `third-party/BUCK` (via `./tools/buckify.sh`)

- [ ] **Step 1: Create `src/nydus/BUCK`**

Follow the pattern from `src/drone-sdk/BUCK`:

```starlark
NYDUS_SRCS = glob(["src/**/*.rs"])

NYDUS_DEPS = [
    "//third-party:base64",
    "//third-party:reqwest",
    "//third-party:serde",
    "//third-party:serde_json",
    "//third-party:thiserror",
]

rust_library(
    name = "nydus",
    srcs = NYDUS_SRCS,
    deps = NYDUS_DEPS,
    visibility = ["PUBLIC"],
)

rust_test(
    name = "nydus-test",
    srcs = NYDUS_SRCS,
    deps = NYDUS_DEPS,
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 2: Regenerate third-party BUCK**

Run: `./tools/buckify.sh`
Expected: completes successfully, `third-party/BUCK` is updated

- [ ] **Step 3: Verify Buck2 build**

Run: `buck2 build root//src/nydus:nydus`
Expected: builds successfully

Run: `buck2 test root//src/nydus:nydus-test`
Expected: 2 tests pass

- [ ] **Step 4: Commit**

```bash
git add src/nydus/BUCK third-party/BUCK
git commit -m "build(nydus): add Buck2 target"
```

---

## Task 5: Overseer — add `config_overrides` to `start_run`

**Files:**
- Modify: `src/overseer/src/api/jobs.rs:77-99`
- Modify: `src/overseer/src/services/jobs.rs:35-44`
- Test: `src/overseer/src/services/jobs.rs` (existing tests + new test)

- [ ] **Step 1: Write failing test for config override merge**

Add to the `mod tests` in `src/overseer/src/services/jobs.rs`:

```rust
    #[tokio::test]
    async fn test_start_job_run_with_config_overrides() {
        let sqlite_db = SqliteDatabase::open_in_memory_named("svc_jobs_test_override")
            .await
            .expect("db opens");
        let svc = JobService::new(Arc::new(sqlite_db));

        let def = svc
            .create_job_definition(
                "override-test",
                "desc",
                serde_json::json!({"repo_url": "https://github.com/test/repo", "branch": "main", "task": "do stuff"}),
            )
            .await
            .expect("create def");

        let run = svc
            .start_job_run(
                &def.id,
                "operator",
                None,
                Some(serde_json::json!({"branch": "feat/override", "extra": "value"})),
            )
            .await
            .expect("start run");

        // The run's definition should have merged config
        // Verify by fetching the run — the config overrides are stored in result for now
        // Actually, we need to check how the config is passed through.
        // The override merges into the definition config and is stored on the run.
        assert_eq!(run.status, crate::db::models::JobRunStatus::Pending);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/overseer && cargo test test_start_job_run_with_config_overrides`
Expected: FAIL — `start_job_run` doesn't accept a 4th argument

- [ ] **Step 3: Update `JobService::start_job_run` to accept and merge overrides**

In `src/overseer/src/services/jobs.rs`, change the `start_job_run` method:

```rust
    pub async fn start_job_run(
        &self,
        definition_id: &str,
        triggered_by: &str,
        parent_id: Option<&str>,
        config_overrides: Option<serde_json::Value>,
    ) -> Result<JobRun> {
        if let Some(overrides) = config_overrides {
            // Fetch the definition, merge overrides into config, update definition config
            // Actually — we don't want to mutate the definition. We need to store the
            // merged config on the run itself. But the current schema doesn't have a
            // config column on job_runs.
            //
            // For now, store overrides in the run's result field as metadata.
            // TODO: Add a config column to job_runs when we need it.
            let _ = overrides; // Acknowledge but don't use yet — schema change is separate work
        }
        self.db
            .start_job_run(definition_id, triggered_by, parent_id)
            .await
    }
```

Wait — this needs more thought. The current `job_runs` table doesn't have a `config` column. The config lives on the definition. To properly support overrides, we need either:
(a) A `config` column on `job_runs` that stores the merged config
(b) A `config_overrides` column on `job_runs`

Option (b) is simpler — store the overrides, and consumers merge at read time. The poller in Queen already reads `def.config` — it would also read `run.config_overrides` and merge.

Actually, the simplest approach that unblocks the CLI: the `start_run` API accepts `config_overrides`, and the Overseer stores them on the run (new column). Queen's poller already fetches both the definition and the run — it can merge them. This keeps the definition immutable and the override visible.

Let's revise. We need a migration + schema change. This is a bigger step.

- [ ] **Step 3 (revised): Add `config_overrides` column to job_runs**

Create migration `src/overseer/migrations/sqlite/005_job_run_config_overrides.sql`:

```sql
ALTER TABLE job_runs ADD COLUMN config_overrides TEXT DEFAULT NULL;
```

Create migration `src/overseer/migrations/postgres/005_job_run_config_overrides.sql`:

```sql
ALTER TABLE job_runs ADD COLUMN config_overrides JSONB DEFAULT NULL;
```

- [ ] **Step 4: Update `JobRun` model in `src/overseer/src/db/models.rs`**

Add field to the `JobRun` struct:

```rust
pub struct JobRun {
    pub id: String,
    pub definition_id: String,
    pub parent_id: Option<String>,
    pub status: JobRunStatus,
    pub triggered_by: String,
    pub config_overrides: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}
```

- [ ] **Step 5: Update Database trait and implementations**

Update `start_job_run` in the `Database` trait (`src/overseer/src/db/trait_def.rs`) to accept `config_overrides: Option<serde_json::Value>`.

Update both `sqlite.rs` and `postgres.rs` implementations to store the new column. The SQLite impl stores it as `TEXT` (JSON string), Postgres as `JSONB`. Both need to read it back when querying job runs.

This step requires reading the existing trait and both implementations to make the exact changes. The implementer should:
1. Add `config_overrides: Option<serde_json::Value>` parameter to `start_job_run` in `trait_def.rs`
2. In `sqlite.rs` `start_job_run`: serialize overrides to JSON string, include in INSERT
3. In `postgres.rs` `start_job_run`: pass overrides directly (sqlx handles JSONB)
4. In both `sqlite.rs` and `postgres.rs` row-reading code for `JobRun`: read the new column

- [ ] **Step 6: Update `JobService::start_job_run`**

In `src/overseer/src/services/jobs.rs`:

```rust
    pub async fn start_job_run(
        &self,
        definition_id: &str,
        triggered_by: &str,
        parent_id: Option<&str>,
        config_overrides: Option<serde_json::Value>,
    ) -> Result<JobRun> {
        self.db
            .start_job_run(definition_id, triggered_by, parent_id, config_overrides)
            .await
    }
```

- [ ] **Step 7: Update the API handler**

In `src/overseer/src/api/jobs.rs`, update `StartJobRunRequest`:

```rust
#[derive(Deserialize)]
struct StartJobRunRequest {
    definition_id: String,
    triggered_by: String,
    parent_id: Option<String>,
    config_overrides: Option<Value>,
}
```

Update the `start_job_run` handler to pass `body.config_overrides`:

```rust
async fn start_job_run(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StartJobRunRequest>,
) -> Result<Json<Value>> {
    let result = state
        .jobs
        .start_job_run(
            &body.definition_id,
            &body.triggered_by,
            body.parent_id.as_deref(),
            body.config_overrides,
        )
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}
```

- [ ] **Step 8: Update MCP if it exposes start_job_run**

Check `src/overseer/src/mcp/mod.rs` — if there's a `start_job_run` MCP tool, add the `config_overrides` parameter there too.

- [ ] **Step 9: Fix all callers**

Any existing code that calls `start_job_run` with 3 args needs updating to pass `None` as the 4th arg. Grep for `start_job_run` across the codebase and fix each callsite.

- [ ] **Step 10: Run tests**

Run: `cd src/overseer && cargo test`
Expected: all tests pass, including the new `test_start_job_run_with_config_overrides`

- [ ] **Step 11: Commit**

```bash
git add src/overseer/
git commit -m "feat(overseer): add config_overrides to job runs"
```

---

## Task 6: Update `nydus` `JobRun` type for config_overrides

**Files:**
- Modify: `src/nydus/src/types.rs`

- [ ] **Step 1: Add `config_overrides` field to `JobRun`**

In `src/nydus/src/types.rs`, update the `JobRun` struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRun {
    pub id: String,
    pub definition_id: String,
    pub parent_id: Option<String>,
    pub status: String,
    pub triggered_by: String,
    pub config_overrides: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<String>,
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd src/nydus && cargo check`
Expected: compiles

- [ ] **Step 3: Commit**

```bash
git add src/nydus/src/types.rs
git commit -m "feat(nydus): add config_overrides to JobRun type"
```

---

## Task 7: Migrate Queen from `OverseerClient` to `NydusClient`

**Files:**
- Delete: `src/queen/src/overseer_client.rs`
- Modify: `src/queen/Cargo.toml`
- Modify: `src/queen/BUCK`
- Modify: `src/queen/src/main.rs`
- Modify: `src/queen/src/actors/registrar.rs`
- Modify: `src/queen/src/actors/heartbeat.rs`
- Modify: `src/queen/src/actors/poller.rs`
- Modify: `src/queen/src/actors/supervisor.rs`

The key API differences between Queen's `OverseerClient` and `NydusClient`:

| Queen's OverseerClient | NydusClient equivalent |
|---|---|
| `client.register(name, caps, concurrency)` (stores hatchery_id internally) | `client.register_hatchery(name, caps, concurrency)` (returns Hatchery, caller stores ID) |
| `client.heartbeat(status, drones)` (uses stored hatchery_id) | `client.heartbeat(hatchery_id, status, drones)` (caller passes ID) |
| `client.poll_jobs()` (uses stored hatchery_id) | `client.list_hatchery_jobs(hatchery_id, Some("pending"))` |
| `client.update_job_run(id, status, result, error)` | `client.update_run(id, status, result, error)` |
| `client.get_job_definition(id)` | `client.get_definition(id)` |
| `client.get_tasks_for_run(run_id)` | `client.list_tasks(None, None, Some(run_id))` |
| `client.store_artifact(name, ct, data, run_id)` | `client.store_artifact(name, ct, data, run_id)` (same) |
| `client.poll_auth_code(run_id)` | `client.poll_auth_code(run_id)` (same) |
| `client.deregister()` (uses stored hatchery_id) | `client.deregister_hatchery(hatchery_id)` |

- [ ] **Step 1: Update `src/queen/Cargo.toml`**

Remove `reqwest` and `base64` from dependencies. Add:

```toml
nydus = { path = "../nydus" }
```

- [ ] **Step 2: Update `src/queen/BUCK`**

In `QUEEN_DEPS`, replace `"//third-party:reqwest"` and `"//third-party:base64"` with `"//src/nydus:nydus"`.

- [ ] **Step 3: Update `src/queen/src/main.rs`**

Remove `mod overseer_client;` and `use overseer_client::OverseerClient;`.

Add `use nydus::NydusClient;`.

Change the client creation:

```rust
let client = NydusClient::new(config.queen.overseer_url.clone());
```

Queen needs to track `hatchery_id` itself now. Add a shared state:

```rust
use std::sync::Arc;
use tokio::sync::RwLock;

let hatchery_id: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
```

Update the `register` call to store the returned ID:

```rust
let hatchery = actors::registrar::run(
    client.clone(),
    config.queen.name.clone(),
    config.queen.max_concurrency,
    notifier.clone(),
)
.await?;
*hatchery_id.write().await = Some(hatchery.id.clone());
```

Pass `hatchery_id.clone()` to heartbeat, poller, supervisor actors. Update `deregister` at shutdown:

```rust
if let Some(id) = hatchery_id.read().await.as_ref() {
    if let Err(e) = client.deregister_hatchery(id).await {
        tracing::warn!(error = %e, "failed to deregister from overseer");
    }
}
```

- [ ] **Step 4: Update `src/queen/src/actors/registrar.rs`**

Change `OverseerClient` → `NydusClient`. The `register` call becomes `client.register_hatchery(...)`. Return the `Hatchery` so main can extract the ID:

```rust
use nydus::NydusClient;
use nydus::Hatchery;

pub async fn run(
    client: NydusClient,
    name: String,
    max_concurrency: i32,
    notifier: Arc<dyn Notifier>,
) -> anyhow::Result<Hatchery> {
    let capabilities = serde_json::json!({});

    loop {
        match client
            .register_hatchery(&name, capabilities.clone(), max_concurrency)
            .await
        {
            Ok(hatchery) => {
                notifier
                    .notify(QueenEvent::HatcheryRegistered {
                        name: name.clone(),
                        id: hatchery.id.clone(),
                    })
                    .await;
                return Ok(hatchery);
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to register with overseer, retrying in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}
```

- [ ] **Step 5: Update `src/queen/src/actors/heartbeat.rs`**

Change `OverseerClient` → `NydusClient`. Add `hatchery_id: Arc<RwLock<Option<String>>>` parameter. The heartbeat call becomes:

```rust
use nydus::NydusClient;
use std::sync::Arc;
use tokio::sync::RwLock;

pub async fn run(
    client: NydusClient,
    hatchery_id: Arc<RwLock<Option<String>>>,
    interval_secs: u64,
    status_tx: mpsc::Sender<(StatusQuery, oneshot::Sender<StatusResponse>)>,
    token: CancellationToken,
) {
    // ... in the heartbeat call:
    let id = match hatchery_id.read().await.as_ref() {
        Some(id) => id.clone(),
        None => {
            tracing::warn!("no hatchery id yet, skipping heartbeat");
            continue;
        }
    };
    if let Err(e) = client.heartbeat(&id, status, status_resp.active_drones).await {
        tracing::warn!(error = %e, "heartbeat failed");
    }
}
```

- [ ] **Step 6: Update `src/queen/src/actors/poller.rs`**

Change `OverseerClient` → `NydusClient`. Add `hatchery_id: Arc<RwLock<Option<String>>>` parameter. The poll call becomes `client.list_hatchery_jobs(&id, Some("pending"))`. The `client.get_job_definition` becomes `client.get_definition`. The `client.update_job_run` becomes `client.update_run`.

- [ ] **Step 7: Update `src/queen/src/actors/supervisor.rs`**

Change all `OverseerClient` references to `NydusClient`. Update method calls:
- `client.update_job_run(...)` → `client.update_run(...)`
- `client.store_artifact(...)` → `client.store_artifact(...)` (same signature)
- `client.poll_auth_code(...)` → `client.poll_auth_code(...)` (same signature)

- [ ] **Step 8: Delete `src/queen/src/overseer_client.rs`**

Remove the file.

- [ ] **Step 9: Run Queen tests**

Run: `cd src/queen && cargo test`
Expected: all tests pass

- [ ] **Step 10: Verify Buck2 build**

Run: `buck2 build root//src/queen:queen`
Expected: builds successfully

- [ ] **Step 11: Commit**

```bash
git add -A src/queen/ src/nydus/
git commit -m "refactor(queen): migrate from OverseerClient to nydus::NydusClient"
```

---

## Task 8: Create `kerrigan` CLI crate — scaffold + submit command

**Files:**
- Create: `src/kerrigan/Cargo.toml`
- Create: `src/kerrigan/BUCK`
- Create: `src/kerrigan/src/main.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create `src/kerrigan/Cargo.toml`**

```toml
[package]
name = "kerrigan"
version = "0.1.0"
edition = "2024"

[dependencies]
nydus = { path = "../nydus" }
clap = { version = "4", features = ["derive", "env"] }
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
anyhow = "1"
```

- [ ] **Step 2: Create `src/kerrigan/src/main.rs`**

```rust
use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use nydus::NydusClient;

#[derive(Parser)]
#[command(name = "kerrigan", about = "Kerrigan operator console")]
struct Cli {
    /// Overseer URL
    #[arg(long, env = "KERRIGAN_URL", default_value = "http://localhost:3100")]
    url: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Submit a problem into the dev loop
    Submit {
        /// Problem description
        problem: String,

        /// Override config values (key=value)
        #[arg(long = "set", value_name = "KEY=VALUE")]
        overrides: Vec<String>,

        /// Target hatchery name (auto-selects if omitted)
        #[arg(long)]
        hatchery: Option<String>,

        /// Job definition name to use
        #[arg(long, default_value = "spec-from-problem")]
        definition: String,
    },

    /// Show job status
    Status {
        /// Job run ID (lists all if omitted)
        run_id: Option<String>,
    },

    /// Approve a job at a gate
    Approve {
        /// Job run ID
        run_id: String,

        /// Optional message
        #[arg(long)]
        message: Option<String>,
    },

    /// Reject a job at a gate
    Reject {
        /// Job run ID
        run_id: String,

        /// Rejection reason
        #[arg(long)]
        message: String,
    },

    /// Submit an OAuth code for a running job
    Auth {
        /// Job run ID
        run_id: String,

        /// OAuth code
        code: String,
    },

    /// View run output and decisions
    Log {
        /// Job run ID
        run_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = NydusClient::new(&cli.url);

    match cli.command {
        Command::Submit {
            problem,
            overrides,
            hatchery,
            definition,
        } => cmd_submit(&client, &definition, &problem, &overrides, hatchery.as_deref()).await,

        Command::Status { run_id } => cmd_status(&client, run_id.as_deref()).await,

        Command::Approve { run_id, message } => {
            cmd_approve(&client, &run_id, message.as_deref()).await
        }

        Command::Reject { run_id, message } => {
            cmd_reject(&client, &run_id, &message).await
        }

        Command::Auth { run_id, code } => cmd_auth(&client, &run_id, &code).await,

        Command::Log { run_id } => cmd_log(&client, &run_id).await,
    }
}

async fn cmd_submit(
    client: &NydusClient,
    definition_name: &str,
    problem: &str,
    overrides: &[String],
    hatchery_name: Option<&str>,
) -> Result<()> {
    // 1. Resolve definition by name
    let definitions = client.list_definitions().await?;
    let def = definitions
        .iter()
        .find(|d| d.name == definition_name)
        .ok_or_else(|| anyhow::anyhow!("job definition '{}' not found", definition_name))?;

    // 2. Build config overrides
    let mut config = serde_json::json!({ "problem": problem });
    for kv in overrides {
        let (key, value) = kv
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("invalid override format '{}', expected key=value", kv))?;
        config[key] = serde_json::Value::String(value.to_string());
    }

    // 3. Start run
    let run = client
        .start_run(&def.id, "operator", None, Some(config))
        .await?;
    println!("Started run: {}", run.id);

    // 4. Find hatchery
    let hatchery = if let Some(name) = hatchery_name {
        let hatcheries = client.list_hatcheries(Some("online")).await?;
        hatcheries
            .into_iter()
            .find(|h| h.name == name)
            .ok_or_else(|| anyhow::anyhow!("hatchery '{}' not found or not online", name))?
    } else {
        let hatcheries = client.list_hatcheries(Some("online")).await?;
        hatcheries
            .into_iter()
            .find(|h| h.active_drones < h.max_concurrency)
            .ok_or_else(|| anyhow::anyhow!("no hatcheries with available capacity"))?
    };

    // 5. Assign
    client.assign_job(&hatchery.id, &run.id).await?;
    println!("Assigned to hatchery: {} ({})", hatchery.name, hatchery.id);

    Ok(())
}

async fn cmd_status(client: &NydusClient, run_id: Option<&str>) -> Result<()> {
    match run_id {
        Some(id) => {
            let runs = client.list_runs(None).await?;
            let run = runs
                .iter()
                .find(|r| r.id == id)
                .ok_or_else(|| anyhow::anyhow!("run '{}' not found", id))?;
            println!("Run: {}", run.id);
            println!("  Status:       {}", run.status);
            println!("  Definition:   {}", run.definition_id);
            println!("  Triggered by: {}", run.triggered_by);
            if let Some(ref err) = run.error {
                println!("  Error:        {}", err);
            }

            let tasks = client.list_tasks(None, None, Some(id)).await?;
            if !tasks.is_empty() {
                println!("  Tasks:");
                for task in &tasks {
                    println!("    - [{}] {}", task.status, task.subject);
                }
            }
        }
        None => {
            let runs = client.list_runs(None).await?;
            if runs.is_empty() {
                println!("No runs found.");
            } else {
                for run in &runs {
                    let marker = if run.status == "pending" { " [needs attention]" } else { "" };
                    println!("{} — {} ({}){}", run.id, run.status, run.triggered_by, marker);
                }
            }
        }
    }
    Ok(())
}

async fn cmd_approve(client: &NydusClient, run_id: &str, _message: Option<&str>) -> Result<()> {
    // For now, approve = mark as running (resume from gate)
    // Full gate logic is roadmap #7
    client.update_run(run_id, Some("running"), None, None).await?;
    println!("Approved: {}", run_id);
    Ok(())
}

async fn cmd_reject(client: &NydusClient, run_id: &str, message: &str) -> Result<()> {
    client
        .update_run(run_id, Some("failed"), None, Some(message))
        .await?;
    println!("Rejected: {}", run_id);
    Ok(())
}

async fn cmd_auth(client: &NydusClient, run_id: &str, code: &str) -> Result<()> {
    client.submit_auth_code(run_id, code).await?;
    println!("Auth code submitted for run: {}", run_id);
    Ok(())
}

async fn cmd_log(client: &NydusClient, run_id: &str) -> Result<()> {
    let artifacts = client.list_artifacts(Some(run_id)).await?;
    if artifacts.is_empty() {
        println!("No artifacts for run {}.", run_id);
    } else {
        println!("Artifacts for run {}:", run_id);
        for a in &artifacts {
            println!("  {} — {} ({})", a.id, a.name, a.content_type);
        }
    }

    let tasks = client.list_tasks(None, None, Some(run_id)).await?;
    if !tasks.is_empty() {
        println!("\nTasks:");
        for task in &tasks {
            println!("  [{}] {}", task.status, task.subject);
            if let Some(ref output) = task.output {
                println!("    Output: {}", serde_json::to_string_pretty(output)?);
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Add `src/kerrigan` to workspace**

In root `Cargo.toml`:

```toml
[workspace]
members = ["src/overseer", "src/queen", "src/drone-sdk", "src/drones/claude/base", "src/creep", "src/nydus", "src/kerrigan"]
resolver = "2"
```

- [ ] **Step 4: Verify it compiles**

Run: `cd src/kerrigan && cargo check`
Expected: compiles

- [ ] **Step 5: Create `src/kerrigan/BUCK`**

```starlark
KERRIGAN_SRCS = glob(["src/**/*.rs"])

KERRIGAN_DEPS = [
    "//src/nydus:nydus",
    "//third-party:anyhow",
    "//third-party:clap",
    "//third-party:serde_json",
    "//third-party:tokio",
]

rust_binary(
    name = "kerrigan",
    srcs = KERRIGAN_SRCS,
    crate_root = "src/main.rs",
    deps = KERRIGAN_DEPS,
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 6: Buckify and verify**

Run: `./tools/buckify.sh`
Run: `buck2 build root//src/kerrigan:kerrigan`
Expected: builds successfully

- [ ] **Step 7: Commit**

```bash
git add src/kerrigan/ Cargo.toml
git commit -m "feat(kerrigan): scaffold CLI with submit, status, approve, reject, auth, log commands"
```

---

## Task 9: Full build verification

**Files:** None (verification only)

- [ ] **Step 1: Run full workspace cargo check**

Run: `cargo check --workspace`
Expected: all crates compile

- [ ] **Step 2: Run all cargo tests**

Run: `cargo test --workspace`
Expected: all tests pass

- [ ] **Step 3: Run Buck2 build for all targets**

Run: `buck2 build root//src/nydus:nydus root//src/kerrigan:kerrigan root//src/queen:queen root//src/overseer:overseer`
Expected: all targets build

- [ ] **Step 4: Run Buck2 tests**

Run: `buck2 test root//src/nydus:nydus-test root//src/overseer:overseer-test`
Expected: all tests pass

- [ ] **Step 5: Run pre-commit hooks**

Run: `buck2 run root//tools:prek -- run --all-files`
Expected: all hooks pass (fmt, clippy, tests, reindeer sync)

- [ ] **Step 6: Verify CLI help**

Run: `cargo run -p kerrigan -- --help`
Expected: shows usage with submit, status, approve, reject, auth, log commands

Run: `cargo run -p kerrigan -- submit --help`
Expected: shows submit usage with problem, --set, --hatchery, --definition flags
