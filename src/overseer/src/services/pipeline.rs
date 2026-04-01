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
            None => return Ok(None),
        };

        let current_stage = match find_stage(stage_name) {
            Some(s) => s,
            None => return Ok(None),
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
            Some(next_stage) => self
                .create_next_run(completed_run, next_stage)
                .await
                .map(Some),
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
    pub async fn advance(&self, run_id: &str) -> Result<JobRun> {
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

        if !current_stage.gate_before_next {
            return Err(OverseerError::Validation(format!(
                "stage '{stage_name}' is not gated — it auto-advances on completion"
            )));
        }

        let next_stage_name = current_stage.next.ok_or_else(|| {
            OverseerError::Validation(format!(
                "stage '{stage_name}' is the last stage — nothing to advance to"
            ))
        })?;

        self.create_next_run(&run, next_stage_name).await
    }

    async fn create_next_run(&self, parent_run: &JobRun, next_stage_name: &str) -> Result<JobRun> {
        let next_stage = find_stage(next_stage_name).ok_or_else(|| {
            OverseerError::Internal(format!("unknown next stage: {next_stage_name}"))
        })?;

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

    fn build_context_overrides(&self, parent_run: &JobRun, _next_stage: &str) -> serde_json::Value {
        let mut overrides = serde_json::json!({});

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
            // Forward branch if explicitly set (partial pipeline, working on a branch)
            if let Some(branch) = parent_overrides.get("branch") {
                overrides["branch"] = branch.clone();
            }
        }

        // Extract git_refs from parent's result
        if let Some(ref result) = parent_run.result
            && let Some(refs) = result.get("git_refs")
        {
            // Always forward PR URL if available
            if let Some(pr_url) = refs.get("pr_url") {
                overrides["pr_url"] = pr_url.clone();
            }
            // Forward branch from git_refs (overrides explicit branch if drone created one)
            if let Some(branch) = refs.get("branch") {
                overrides["branch"] = branch.clone();
            }
        }

        overrides
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::JobStore;
    use crate::db::SqliteDatabase;

    #[tokio::test]
    async fn test_advance_non_pipeline_run_returns_none() {
        let db = SqliteDatabase::open_in_memory_named("pipeline_test_non_pipeline")
            .await
            .expect("db opens");
        let db = Arc::new(db);
        let svc = PipelineService::new(db.clone());

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
            .create_job_definition("spec-def", "test", serde_json::json!({"stage": "spec"}))
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
        let next_overrides = next.config_overrides.expect("overrides should exist");
        assert_eq!(
            next_overrides.get("repo_url").and_then(|v| v.as_str()),
            Some("https://github.com/test/repo.git")
        );
        assert_eq!(
            next_overrides.get("pr_url").and_then(|v| v.as_str()),
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

        let _ = db
            .create_job_definition(
                "plan-from-spec",
                "Write a plan",
                serde_json::json!({"drone_type": "claude-drone", "stage": "plan"}),
            )
            .await
            .expect("seed plan def");

        let spec_def = db
            .create_job_definition("spec-def-2", "test", serde_json::json!({"stage": "spec"}))
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
            .create_job_definition("review-def", "test", serde_json::json!({"stage": "review"}))
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
            .create_job_definition("spec-def-3", "test", serde_json::json!({"stage": "spec"}))
            .await
            .expect("create def");

        let run = db
            .start_job_run(&def.id, "test", None, None)
            .await
            .expect("start run");

        let result = svc.advance(&run.id).await;
        assert!(
            result.is_err(),
            "should fail — run is pending, not completed"
        );
    }
}
