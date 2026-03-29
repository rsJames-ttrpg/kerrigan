use sqlx::SqlitePool;

use crate::db::jobs as db;
use crate::error::Result;

pub struct JobService {
    pool: SqlitePool,
}

impl JobService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_job_definition(
        &self,
        name: &str,
        description: &str,
        config: serde_json::Value,
    ) -> Result<db::JobDefinition> {
        db::create_job_definition(&self.pool, name, description, config).await
    }

    pub async fn get_job_definition(&self, id: &str) -> Result<Option<db::JobDefinition>> {
        db::get_job_definition(&self.pool, id).await
    }

    pub async fn list_job_definitions(&self) -> Result<Vec<db::JobDefinition>> {
        db::list_job_definitions(&self.pool).await
    }

    pub async fn start_job_run(
        &self,
        definition_id: &str,
        triggered_by: &str,
        parent_id: Option<&str>,
    ) -> Result<db::JobRun> {
        db::start_job_run(&self.pool, definition_id, triggered_by, parent_id).await
    }

    pub async fn get_job_run(&self, id: &str) -> Result<Option<db::JobRun>> {
        db::get_job_run(&self.pool, id).await
    }

    pub async fn update_job_run(
        &self,
        id: &str,
        status: Option<&str>,
        result: Option<serde_json::Value>,
        error: Option<&str>,
    ) -> Result<db::JobRun> {
        db::update_job_run(&self.pool, id, status, result, error).await
    }

    pub async fn list_job_runs(&self, status: Option<&str>) -> Result<Vec<db::JobRun>> {
        db::list_job_runs(&self.pool, status).await
    }

    pub async fn create_task(
        &self,
        subject: &str,
        run_id: Option<&str>,
        assigned_to: Option<&str>,
    ) -> Result<db::Task> {
        db::create_task(&self.pool, subject, run_id, assigned_to).await
    }

    pub async fn get_task(&self, id: &str) -> Result<Option<db::Task>> {
        db::get_task(&self.pool, id).await
    }

    pub async fn update_task(
        &self,
        id: &str,
        status: Option<&str>,
        assigned_to: Option<&str>,
        output: Option<serde_json::Value>,
    ) -> Result<db::Task> {
        db::update_task(&self.pool, id, status, assigned_to, output).await
    }

    pub async fn list_tasks(
        &self,
        status: Option<&str>,
        assigned_to: Option<&str>,
        run_id: Option<&str>,
    ) -> Result<Vec<db::Task>> {
        db::list_tasks(&self.pool, status, assigned_to, run_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory_named;

    #[tokio::test]
    async fn test_job_service_definition_lifecycle() {
        let pool = open_in_memory_named("svc_jobs_test_def")
            .await
            .expect("pool opens");
        let svc = JobService::new(pool);

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
        let pool = open_in_memory_named("svc_jobs_test_run")
            .await
            .expect("pool opens");
        let svc = JobService::new(pool);

        let def = svc
            .create_job_definition("run-job-svc", "desc", serde_json::json!({}))
            .await
            .expect("create def");

        let run = svc
            .start_job_run(&def.id, "agent-1", None)
            .await
            .expect("start run");
        assert_eq!(run.status, "running");

        let updated = svc
            .update_job_run(
                &run.id,
                Some("completed"),
                Some(serde_json::json!({"ok": true})),
                None,
            )
            .await
            .expect("update run");
        assert_eq!(updated.status, "completed");
        assert!(updated.completed_at.is_some());

        let runs = svc
            .list_job_runs(Some("completed"))
            .await
            .expect("list completed");
        assert_eq!(runs.len(), 1);
    }

    #[tokio::test]
    async fn test_job_service_task_lifecycle() {
        let pool = open_in_memory_named("svc_jobs_test_task")
            .await
            .expect("pool opens");
        let svc = JobService::new(pool);

        let def = svc
            .create_job_definition("task-job-svc", "desc", serde_json::json!({}))
            .await
            .expect("create def");
        let run = svc
            .start_job_run(&def.id, "agent-svc-task", None)
            .await
            .expect("start run");

        let task = svc
            .create_task("do the thing", Some(&run.id), Some("agent-svc-task"))
            .await
            .expect("create task");
        assert_eq!(task.status, "pending");

        let updated = svc
            .update_task(
                &task.id,
                Some("completed"),
                None,
                Some(serde_json::json!({"result": "done"})),
            )
            .await
            .expect("update task");
        assert_eq!(updated.status, "completed");

        let tasks = svc
            .list_tasks(None, None, Some(&run.id))
            .await
            .expect("list tasks");
        assert_eq!(tasks.len(), 1);
    }
}
