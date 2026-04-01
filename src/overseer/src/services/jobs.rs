use std::sync::Arc;

use crate::db::Database;
use crate::db::models::{JobDefinition, JobRun, Task};
use crate::error::Result;

pub struct JobService {
    db: Arc<dyn Database>,
}

impl JobService {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }

    pub async fn create_job_definition(
        &self,
        name: &str,
        description: &str,
        config: serde_json::Value,
    ) -> Result<JobDefinition> {
        self.db
            .create_job_definition(name, description, config)
            .await
    }

    pub async fn get_job_definition(&self, id: &str) -> Result<Option<JobDefinition>> {
        self.db.get_job_definition(id).await
    }

    pub async fn list_job_definitions(&self) -> Result<Vec<JobDefinition>> {
        self.db.list_job_definitions().await
    }

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

    pub async fn get_job_run(&self, id: &str) -> Result<Option<JobRun>> {
        self.db.get_job_run(id).await
    }

    pub async fn update_job_run(
        &self,
        id: &str,
        status: Option<&str>,
        result: Option<serde_json::Value>,
        error: Option<&str>,
    ) -> Result<JobRun> {
        self.db.update_job_run(id, status, result, error).await
    }

    pub async fn list_job_runs(&self, status: Option<&str>) -> Result<Vec<JobRun>> {
        self.db.list_job_runs(status).await
    }

    pub async fn create_task(
        &self,
        subject: &str,
        run_id: Option<&str>,
        assigned_to: Option<&str>,
    ) -> Result<Task> {
        self.db.create_task(subject, run_id, assigned_to).await
    }

    pub async fn get_task(&self, id: &str) -> Result<Option<Task>> {
        self.db.get_task(id).await
    }

    pub async fn update_task(
        &self,
        id: &str,
        status: Option<&str>,
        assigned_to: Option<&str>,
        output: Option<serde_json::Value>,
    ) -> Result<Task> {
        self.db.update_task(id, status, assigned_to, output).await
    }

    pub async fn list_tasks(
        &self,
        status: Option<&str>,
        assigned_to: Option<&str>,
        run_id: Option<&str>,
    ) -> Result<Vec<Task>> {
        self.db.list_tasks(status, assigned_to, run_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDatabase;

    #[tokio::test]
    async fn test_job_service_definition_lifecycle() {
        let sqlite_db = SqliteDatabase::open_in_memory_named("svc_jobs_test_def")
            .await
            .expect("db opens");
        let svc = JobService::new(Arc::new(sqlite_db));

        let def = svc
            .create_job_definition("my-job-svc-def", "desc", serde_json::json!({}))
            .await
            .expect("create");

        let fetched = svc
            .get_job_definition(&def.id)
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(fetched.name, "my-job-svc-def");

        let all = svc.list_job_definitions().await.expect("list");
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn test_job_service_run_lifecycle() {
        let sqlite_db = SqliteDatabase::open_in_memory_named("svc_jobs_test_run")
            .await
            .expect("db opens");
        let svc = JobService::new(Arc::new(sqlite_db));

        let def = svc
            .create_job_definition("run-job-svc", "desc", serde_json::json!({}))
            .await
            .expect("create def");

        let run = svc
            .start_job_run(&def.id, "agent-1", None, None)
            .await
            .expect("start run");
        assert_eq!(run.status, crate::db::models::JobRunStatus::Pending);

        let updated = svc
            .update_job_run(
                &run.id,
                Some("completed"),
                Some(serde_json::json!({"ok": true})),
                None,
            )
            .await
            .expect("update run");
        assert_eq!(updated.status, crate::db::models::JobRunStatus::Completed);
        assert!(updated.completed_at.is_some());

        let runs = svc
            .list_job_runs(Some("completed"))
            .await
            .expect("list completed");
        assert_eq!(runs.len(), 1);
    }

    #[tokio::test]
    async fn test_job_service_task_lifecycle() {
        let sqlite_db = SqliteDatabase::open_in_memory_named("svc_jobs_test_task")
            .await
            .expect("db opens");
        let svc = JobService::new(Arc::new(sqlite_db));

        let def = svc
            .create_job_definition("task-job-svc", "desc", serde_json::json!({}))
            .await
            .expect("create def");
        let run = svc
            .start_job_run(&def.id, "agent-svc-task", None, None)
            .await
            .expect("start run");

        let task = svc
            .create_task("do the thing", Some(&run.id), Some("agent-svc-task"))
            .await
            .expect("create task");
        assert_eq!(task.status, crate::db::models::TaskStatus::Pending);

        let updated = svc
            .update_task(
                &task.id,
                Some("completed"),
                None,
                Some(serde_json::json!({"result": "done"})),
            )
            .await
            .expect("update task");
        assert_eq!(updated.status, crate::db::models::TaskStatus::Completed);

        let tasks = svc
            .list_tasks(None, None, Some(&run.id))
            .await
            .expect("list tasks");
        assert_eq!(tasks.len(), 1);
    }

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
                serde_json::json!({"repo_url": "https://github.com/test/repo", "task": "do stuff"}),
            )
            .await
            .expect("create def");

        let overrides = serde_json::json!({"branch": "feat/override", "extra": "value"});
        let run = svc
            .start_job_run(&def.id, "operator", None, Some(overrides.clone()))
            .await
            .expect("start run");

        assert_eq!(run.status, crate::db::models::JobRunStatus::Pending);
        assert_eq!(run.config_overrides, Some(overrides));
    }
}
