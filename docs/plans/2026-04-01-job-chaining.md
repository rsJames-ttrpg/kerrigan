# Job Chaining Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a pipeline stage completes, Overseer automatically creates the next stage's run (or waits for human approval at gates), enabling the full spec→plan→implement→review dev loop.

**Architecture:** Hardcoded pipeline definition in a new `pipeline.rs` module in Overseer services. Pipeline advancement triggered in `update_job_run` when status becomes `completed`. New `/advance` endpoint for gated transitions. `kerrigan approve` calls advance instead of updating status. `nydus` gains an `advance_run` method.

**Tech Stack:** Rust 2024, axum, serde_json

**Spec:** `docs/specs/2026-04-01-job-chaining-design.md`

---

## File Structure

### New files

| File | Responsibility |
|------|----------------|
| `src/overseer/src/services/pipeline.rs` | Hardcoded pipeline definition, advancement logic, context forwarding |

### Modified files

| File | Change |
|------|--------|
| `src/overseer/src/services/mod.rs` | Add `pub mod pipeline`, add `PipelineService` to `AppState` |
| `src/overseer/src/api/jobs.rs` | Add `POST /runs/{id}/advance` endpoint |
| `src/nydus/src/client.rs` | Add `advance_run` method |
| `src/kerrigan/src/main.rs` | Update `cmd_approve` to call `advance_run`, enhance `cmd_status` with pipeline view |

---

## Task 1: Pipeline definition and advancement logic

**Files:**
- Create: `src/overseer/src/services/pipeline.rs`
- Modify: `src/overseer/src/services/mod.rs`

- [ ] **Step 1: Create `src/overseer/src/services/pipeline.rs`**

```rust
use std::sync::Arc;

use crate::db::Database;
use crate::db::models::{JobDefinition, JobRun};
use crate::error::{OverseerError, Result};

/// Hardcoded pipeline: spec → plan → implement → review
const PIPELINE: &[PipelineStage] = &[
    PipelineStage {
        stage: "spec",
        definition_name: "spec-from-problem",
        next: Some("plan"),
        gate_before_next: true,
    },
    PipelineStage {
        stage: "plan",
        definition_name: "plan-from-spec",
        next: Some("implement"),
        gate_before_next: true,
    },
    PipelineStage {
        stage: "implement",
        definition_name: "implement-from-plan",
        next: Some("review"),
        gate_before_next: false,
    },
    PipelineStage {
        stage: "review",
        definition_name: "review-pr",
        next: None,
        gate_before_next: false,
    },
];

struct PipelineStage {
    stage: &'static str,
    definition_name: &'static str,
    next: Option<&'static str>,
    gate_before_next: bool,
}

fn find_stage(stage: &str) -> Option<&'static PipelineStage> {
    PIPELINE.iter().find(|s| s.stage == stage)
}

fn find_stage_by_def_name(name: &str) -> Option<&'static PipelineStage> {
    PIPELINE.iter().find(|s| s.definition_name == name)
}

pub struct PipelineService {
    db: Arc<dyn Database>,
}

impl PipelineService {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }

    /// Check if a completed run should trigger the next pipeline stage.
    /// Returns the new run if one was created (non-gated transition).
    /// Returns None if the stage is gated (awaiting approval) or not a pipeline run.
    pub async fn check_advancement(
        &self,
        completed_run: &JobRun,
        completed_def: &JobDefinition,
    ) -> Result<Option<JobRun>> {
        let stage_name = match completed_def.config.get("stage").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return Ok(None), // Not a pipeline run
        };

        let current_stage = match find_stage(stage_name) {
            Some(s) => s,
            None => return Ok(None), // Unknown stage
        };

        if current_stage.gate_before_next {
            tracing::info!(
                run_id = %completed_run.id,
                stage = stage_name,
                "pipeline stage completed — awaiting approval before next stage"
            );
            return Ok(None);
        }

        match current_stage.next {
            Some(next_stage) => self.create_next_run(completed_run, next_stage).await.map(Some),
            None => {
                tracing::info!(
                    run_id = %completed_run.id,
                    stage = stage_name,
                    "pipeline complete — no more stages"
                );
                Ok(None)
            }
        }
    }

    /// Advance a gated pipeline stage to the next stage. Called by `kerrigan approve`.
    pub async fn advance(
        &self,
        run_id: &str,
    ) -> Result<JobRun> {
        let run = self
            .db
            .get_job_run(run_id)
            .await?
            .ok_or_else(|| OverseerError::NotFound(format!("job run {run_id}")))?;

        if run.status.to_string() != "completed" {
            return Err(OverseerError::Validation(format!(
                "run {run_id} is not completed (status: {})",
                run.status
            )));
        }

        let def = self
            .db
            .get_job_definition(&run.definition_id)
            .await?
            .ok_or_else(|| {
                OverseerError::NotFound(format!("job definition {}", run.definition_id))
            })?;

        let stage_name = def
            .config
            .get("stage")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                OverseerError::Validation(format!("run {run_id} is not a pipeline stage"))
            })?;

        let current_stage = find_stage(stage_name).ok_or_else(|| {
            OverseerError::Validation(format!("unknown pipeline stage: {stage_name}"))
        })?;

        let next_stage_name = current_stage.next.ok_or_else(|| {
            OverseerError::Validation(format!(
                "stage '{stage_name}' is the last stage — nothing to advance to"
            ))
        })?;

        self.create_next_run(&run, next_stage_name).await
    }

    async fn create_next_run(
        &self,
        parent_run: &JobRun,
        next_stage_name: &str,
    ) -> Result<JobRun> {
        let next_stage = find_stage(next_stage_name).ok_or_else(|| {
            OverseerError::Internal(format!("unknown next stage: {next_stage_name}"))
        })?;

        // Find the next stage's definition
        let definitions = self.db.list_job_definitions().await?;
        let next_def = definitions
            .iter()
            .find(|d| d.name == next_stage.definition_name)
            .ok_or_else(|| {
                OverseerError::Internal(format!(
                    "pipeline definition '{}' not found — was it seeded?",
                    next_stage.definition_name
                ))
            })?;

        // Build config overrides with context from the parent run
        let overrides = self.build_context_overrides(parent_run, next_stage_name);

        let new_run = self
            .db
            .start_job_run(
                &next_def.id,
                "pipeline",
                Some(&parent_run.id),
                Some(overrides),
            )
            .await?;

        tracing::info!(
            parent_run_id = %parent_run.id,
            new_run_id = %new_run.id,
            stage = next_stage_name,
            "pipeline advanced to next stage"
        );

        Ok(new_run)
    }

    fn build_context_overrides(
        &self,
        parent_run: &JobRun,
        next_stage: &str,
    ) -> serde_json::Value {
        let mut overrides = serde_json::json!({});

        // Copy repo_url and secrets from parent's config_overrides or result
        if let Some(ref parent_overrides) = parent_run.config_overrides {
            if let Some(repo_url) = parent_overrides.get("repo_url") {
                overrides["repo_url"] = repo_url.clone();
            }
            if let Some(secrets) = parent_overrides.get("secrets") {
                overrides["secrets"] = secrets.clone();
            }
            if let Some(task) = parent_overrides.get("task") {
                overrides["task"] = task.clone();
            }
        }

        // Extract git_refs from parent's result for stage-specific context
        if let Some(ref result) = parent_run.result {
            let git_refs = result.get("git_refs");

            match next_stage {
                "review" => {
                    // Review needs the PR URL and branch
                    if let Some(refs) = git_refs {
                        if let Some(pr_url) = refs.get("pr_url") {
                            overrides["pr_url"] = pr_url.clone();
                        }
                        if let Some(branch) = refs.get("branch") {
                            overrides["branch"] = branch.clone();
                        }
                    }
                }
                _ => {
                    // Gated stages (plan, implement) clone main — PR was merged
                    // No branch needed
                }
            }
        }

        overrides
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDatabase;

    #[tokio::test]
    async fn test_advance_non_pipeline_run_returns_none() {
        let db = SqliteDatabase::open_in_memory_named("pipeline_test_non_pipeline")
            .await
            .expect("db opens");
        let db = Arc::new(db);
        let svc = PipelineService::new(db.clone());

        // Create a definition without a stage
        let def = db
            .create_job_definition("no-stage", "test", serde_json::json!({}))
            .await
            .expect("create def");
        let run = db
            .start_job_run(&def.id, "test", None, None)
            .await
            .expect("start run");
        let run = db
            .update_job_run(&run.id, Some("completed"), None, None)
            .await
            .expect("complete run");

        let result = svc.check_advancement(&run, &def).await.expect("check");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_advance_gated_stage_returns_none() {
        let db = SqliteDatabase::open_in_memory_named("pipeline_test_gated")
            .await
            .expect("db opens");
        let db = Arc::new(db);
        let svc = PipelineService::new(db.clone());

        let def = db
            .create_job_definition(
                "spec-def",
                "test",
                serde_json::json!({"stage": "spec"}),
            )
            .await
            .expect("create def");
        let run = db
            .start_job_run(&def.id, "test", None, None)
            .await
            .expect("start run");
        let run = db
            .update_job_run(&run.id, Some("completed"), None, None)
            .await
            .expect("complete run");

        let result = svc.check_advancement(&run, &def).await.expect("check");
        assert!(result.is_none(), "gated stage should not auto-advance");
    }

    #[tokio::test]
    async fn test_advance_non_gated_creates_next_run() {
        let db = SqliteDatabase::open_in_memory_named("pipeline_test_auto_advance")
            .await
            .expect("db opens");
        let db = Arc::new(db);
        let svc = PipelineService::new(db.clone());

        // Seed the review definition (next stage after implement)
        let _ = db
            .create_job_definition(
                "review-pr",
                "Review a PR",
                serde_json::json!({"drone_type": "claude-drone", "stage": "review"}),
            )
            .await
            .expect("seed review def");

        let impl_def = db
            .create_job_definition(
                "impl-def",
                "test",
                serde_json::json!({"stage": "implement"}),
            )
            .await
            .expect("create impl def");

        let overrides = serde_json::json!({
            "repo_url": "https://github.com/test/repo.git",
            "secrets": {"github_pat": "ghp_test"}
        });
        let run = db
            .start_job_run(&impl_def.id, "test", None, Some(overrides))
            .await
            .expect("start run");
        let result_data = serde_json::json!({
            "exit_code": 0,
            "git_refs": {"branch": "feat/test", "pr_url": "https://github.com/test/repo/pull/1"}
        });
        let run = db
            .update_job_run(&run.id, Some("completed"), Some(result_data), None)
            .await
            .expect("complete run");

        let next = svc
            .check_advancement(&run, &impl_def)
            .await
            .expect("check")
            .expect("should create next run");

        assert_eq!(next.parent_id.as_deref(), Some(run.id.as_str()));
        assert_eq!(next.triggered_by, "pipeline");
        // Check context was forwarded
        let next_overrides = next.config_overrides.expect("overrides should exist");
        assert_eq!(
            next_overrides.get("repo_url").and_then(|v| v.as_str()),
            Some("https://github.com/test/repo.git")
        );
        assert_eq!(
            next_overrides
                .get("pr_url")
                .and_then(|v| v.as_str()),
            Some("https://github.com/test/repo/pull/1")
        );
    }

    #[tokio::test]
    async fn test_advance_explicit_gated_stage() {
        let db = SqliteDatabase::open_in_memory_named("pipeline_test_explicit_advance")
            .await
            .expect("db opens");
        let db = Arc::new(db);
        let svc = PipelineService::new(db.clone());

        // Seed the plan definition
        let _ = db
            .create_job_definition(
                "plan-from-spec",
                "Write a plan",
                serde_json::json!({"drone_type": "claude-drone", "stage": "plan"}),
            )
            .await
            .expect("seed plan def");

        let spec_def = db
            .create_job_definition(
                "spec-def-2",
                "test",
                serde_json::json!({"stage": "spec"}),
            )
            .await
            .expect("create spec def");

        let overrides = serde_json::json!({
            "repo_url": "https://github.com/test/repo.git",
            "task": "fix the bug"
        });
        let run = db
            .start_job_run(&spec_def.id, "operator", None, Some(overrides))
            .await
            .expect("start run");
        let run = db
            .update_job_run(&run.id, Some("completed"), None, None)
            .await
            .expect("complete run");

        // Explicit advance (simulating kerrigan approve)
        let next = svc.advance(&run.id).await.expect("advance");
        assert_eq!(next.parent_id.as_deref(), Some(run.id.as_str()));
        assert_eq!(next.triggered_by, "pipeline");
    }

    #[tokio::test]
    async fn test_advance_terminal_stage_fails() {
        let db = SqliteDatabase::open_in_memory_named("pipeline_test_terminal")
            .await
            .expect("db opens");
        let db = Arc::new(db);
        let svc = PipelineService::new(db.clone());

        let review_def = db
            .create_job_definition(
                "review-def",
                "test",
                serde_json::json!({"stage": "review"}),
            )
            .await
            .expect("create review def");

        let run = db
            .start_job_run(&review_def.id, "test", None, None)
            .await
            .expect("start run");
        let run = db
            .update_job_run(&run.id, Some("completed"), None, None)
            .await
            .expect("complete run");

        let result = svc.advance(&run.id).await;
        assert!(result.is_err(), "should fail — review is terminal");
    }

    #[tokio::test]
    async fn test_advance_non_completed_run_fails() {
        let db = SqliteDatabase::open_in_memory_named("pipeline_test_not_completed")
            .await
            .expect("db opens");
        let db = Arc::new(db);
        let svc = PipelineService::new(db.clone());

        let def = db
            .create_job_definition(
                "spec-def-3",
                "test",
                serde_json::json!({"stage": "spec"}),
            )
            .await
            .expect("create def");

        let run = db
            .start_job_run(&def.id, "test", None, None)
            .await
            .expect("start run");

        let result = svc.advance(&run.id).await;
        assert!(result.is_err(), "should fail — run is pending, not completed");
    }
}
```

- [ ] **Step 2: Add pipeline module and service to AppState**

In `src/overseer/src/services/mod.rs`, add `pub mod pipeline;` after the existing module declarations:

```rust
pub mod artifacts;
pub mod auth;
pub mod decisions;
pub mod hatchery;
pub mod jobs;
pub mod memory;
pub mod pipeline;
```

Add `PipelineService` to `AppState`:

```rust
pub struct AppState {
    pub memory: memory::MemoryService,
    pub jobs: jobs::JobService,
    pub decisions: decisions::DecisionService,
    pub artifacts: artifacts::ArtifactService,
    pub hatchery: hatchery::HatcheryService,
    pub auth: auth::AuthService,
    pub pipeline: pipeline::PipelineService,
}
```

In the `AppState::new` method, add:

```rust
            pipeline: pipeline::PipelineService::new(db.clone()),
```

before the closing brace, passing `db.clone()`.

- [ ] **Step 3: Run tests**

Run: `cd src/overseer && cargo test pipeline`
Expected: 6 tests pass

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/services/pipeline.rs src/overseer/src/services/mod.rs
git commit -m "feat(overseer): add pipeline service with hardcoded stage advancement"
```

---

## Task 2: Advance API endpoint

**Files:**
- Modify: `src/overseer/src/api/jobs.rs`

- [ ] **Step 1: Add advance endpoint to the runs router**

In `src/overseer/src/api/jobs.rs`, add the route to the existing `router()` function:

```rust
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/definitions", post(create_job_definition))
        .route("/definitions", get(list_job_definitions))
        .route("/definitions/{id}", get(get_job_definition))
        .route("/runs", post(start_job_run))
        .route("/runs", get(list_job_runs))
        .route("/runs/{id}", patch(update_job_run))
        .route("/runs/{id}/advance", post(advance_job_run))
}
```

- [ ] **Step 2: Add the handler**

Add at the end of the file (before the tasks section):

```rust
async fn advance_job_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let new_run = state.pipeline.advance(&id).await?;

    // Try to assign to an available hatchery
    let hatcheries = state.hatchery.list(Some("online")).await?;
    if let Some(hatchery) = hatcheries
        .iter()
        .find(|h| h.active_drones < h.max_concurrency)
    {
        let _ = state.hatchery.assign_job(&new_run.id, &hatchery.id).await;
        tracing::info!(
            run_id = %new_run.id,
            hatchery_id = %hatchery.id,
            "auto-assigned advanced run to hatchery"
        );
    }

    Ok(Json(serde_json::to_value(new_run).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}
```

- [ ] **Step 3: Verify compilation and tests**

Run: `cd src/overseer && cargo check && cargo test`
Expected: compiles, all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/api/jobs.rs
git commit -m "feat(overseer): add POST /runs/{id}/advance endpoint"
```

---

## Task 3: Auto-advancement on non-gated completion

**Files:**
- Modify: `src/overseer/src/services/jobs.rs`

When `update_job_run` sets status to `completed`, check if the pipeline should auto-advance.

- [ ] **Step 1: Add pipeline check to update_job_run**

In `src/overseer/src/services/jobs.rs`, the `JobService` needs access to the pipeline service. But services shouldn't reference each other directly. Instead, add a method that the API handler can call after updating.

Add a new method to `JobService`:

```rust
    /// After a run is marked completed, check if the pipeline should auto-advance.
    /// Returns the new run if one was created.
    pub async fn check_pipeline_after_completion(
        &self,
        run: &JobRun,
        pipeline: &super::pipeline::PipelineService,
    ) -> Result<Option<JobRun>> {
        if run.status != crate::db::models::JobRunStatus::Completed {
            return Ok(None);
        }

        let def = self.db.get_job_definition(&run.definition_id).await?;
        match def {
            Some(def) => pipeline.check_advancement(run, &def).await,
            None => Ok(None),
        }
    }
```

- [ ] **Step 2: Call it from the API handler**

In `src/overseer/src/api/jobs.rs`, in the `update_job_run` handler, after the existing `state.jobs.update_job_run(...)` call, add pipeline advancement check:

Find:

```rust
async fn update_job_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateJobRunRequest>,
) -> Result<Json<Value>> {
    let result = state
        .jobs
        .update_job_run(
            &id,
            body.status.as_deref(),
            body.result,
            body.error.as_deref(),
        )
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}
```

Replace with:

```rust
async fn update_job_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateJobRunRequest>,
) -> Result<Json<Value>> {
    let result = state
        .jobs
        .update_job_run(
            &id,
            body.status.as_deref(),
            body.result,
            body.error.as_deref(),
        )
        .await?;

    // Check if pipeline should auto-advance
    if let Ok(Some(next_run)) = state
        .jobs
        .check_pipeline_after_completion(&result, &state.pipeline)
        .await
    {
        // Try to assign the new run to a hatchery
        let hatcheries = state.hatchery.list(Some("online")).await.unwrap_or_default();
        if let Some(hatchery) = hatcheries
            .iter()
            .find(|h| h.active_drones < h.max_concurrency)
        {
            let _ = state.hatchery.assign_job(&next_run.id, &hatchery.id).await;
            tracing::info!(
                next_run_id = %next_run.id,
                hatchery_id = %hatchery.id,
                "auto-assigned pipeline run to hatchery"
            );
        }
    }

    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}
```

- [ ] **Step 3: Verify compilation and tests**

Run: `cd src/overseer && cargo check && cargo test`
Expected: compiles, all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/services/jobs.rs src/overseer/src/api/jobs.rs
git commit -m "feat(overseer): auto-advance pipeline on non-gated stage completion"
```

---

## Task 4: `nydus` advance_run method

**Files:**
- Modify: `src/nydus/src/client.rs`

- [ ] **Step 1: Add advance_run method**

Add to the `impl NydusClient` block in `src/nydus/src/client.rs`, in the Jobs — Runs section:

```rust
    pub async fn advance_run(&self, id: &str) -> Result<JobRun, Error> {
        let resp = self
            .client
            .post(format!("{}/api/jobs/runs/{id}/advance", self.base_url))
            .send()
            .await?;
        Ok(self.check_response(resp).await?.json().await?)
    }
```

- [ ] **Step 2: Verify compilation**

Run: `cd src/nydus && cargo check && cargo test`
Expected: compiles, 2 tests pass

- [ ] **Step 3: Commit**

```bash
git add src/nydus/src/client.rs
git commit -m "feat(nydus): add advance_run method"
```

---

## Task 5: Update `kerrigan approve` and `status`

**Files:**
- Modify: `src/kerrigan/src/main.rs`

- [ ] **Step 1: Update cmd_approve to call advance**

Find the existing `cmd_approve`:

```rust
async fn cmd_approve(client: &NydusClient, run_id: &str, _message: Option<&str>) -> Result<()> {
    client.update_run(run_id, Some("running"), None, None).await?;
    println!("Approved: {}", run_id);
    Ok(())
}
```

Replace with:

```rust
async fn cmd_approve(client: &NydusClient, run_id: &str, _message: Option<&str>) -> Result<()> {
    let next_run = client.advance_run(run_id).await?;
    println!("Approved: {}", run_id);
    println!("Next stage started: {} (definition: {})", next_run.id, next_run.definition_id);
    Ok(())
}
```

- [ ] **Step 2: Enhance cmd_status with pipeline chain**

Find the existing single-run status display in `cmd_status` (the `Some(id)` branch). After displaying the run details and tasks, add pipeline chain display:

After the tasks section, add:

```rust
            // Show pipeline chain
            let all_runs = client.list_runs(None).await?;

            // Walk up to find root
            let mut root_id = id.to_string();
            loop {
                let r = all_runs.iter().find(|r| r.id == root_id);
                match r.and_then(|r| r.parent_id.as_ref()) {
                    Some(pid) => root_id = pid.clone(),
                    None => break,
                }
            }

            // Walk down from root to collect chain
            let mut chain = Vec::new();
            let mut current_id = Some(root_id);
            while let Some(cid) = current_id {
                if let Some(r) = all_runs.iter().find(|r| r.id == cid) {
                    chain.push(r);
                    // Find child
                    current_id = all_runs
                        .iter()
                        .find(|r| r.parent_id.as_deref() == Some(&cid))
                        .map(|r| r.id.clone());
                } else {
                    break;
                }
            }

            if chain.len() > 1 {
                println!("\n  Pipeline:");
                for r in &chain {
                    let marker = if r.id == id {
                        "→"
                    } else if r.status == "completed" {
                        "✓"
                    } else if r.status == "failed" {
                        "✗"
                    } else {
                        " "
                    };
                    println!("    {} {} — {}", marker, r.id, r.status);
                }
            }
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p kerrigan`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add src/kerrigan/src/main.rs
git commit -m "feat(kerrigan): approve advances pipeline, status shows chain"
```

---

## Task 6: Full verification

**Files:** None (verification only)

- [ ] **Step 1: Run all tests**

Run: `cargo test -p overseer -p queen -p nydus -p drone-sdk`
Expected: all pass

- [ ] **Step 2: Run pre-commit hooks**

Run: `buck2 run root//tools:prek -- run --all-files`
Expected: all hooks pass

- [ ] **Step 3: Verify advance endpoint manually**

Start Overseer: `buck2 run root//src/overseer:overseer` (in another terminal)

Create a spec run and complete it:
```bash
# Create a run
curl -s -X POST http://localhost:3100/api/jobs/runs \
  -H 'Content-Type: application/json' \
  -d '{"definition_id":"<spec-from-problem-id>","triggered_by":"test"}' | python3 -m json.tool

# Complete it
curl -s -X PATCH http://localhost:3100/api/jobs/runs/<run-id> \
  -H 'Content-Type: application/json' \
  -d '{"status":"completed"}' | python3 -m json.tool

# Advance (should create plan run)
curl -s -X POST http://localhost:3100/api/jobs/runs/<run-id>/advance | python3 -m json.tool
```
Expected: advance returns a new run with `parent_id` set and `triggered_by: "pipeline"`
