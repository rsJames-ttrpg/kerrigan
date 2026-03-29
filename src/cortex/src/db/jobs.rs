use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::{CortexError, Result};

#[derive(Debug, Clone)]
pub struct JobDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub config: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct JobRun {
    pub id: String,
    pub definition_id: String,
    pub parent_id: Option<String>,
    pub status: String,
    pub triggered_by: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub run_id: Option<String>,
    pub subject: String,
    pub status: String,
    pub assigned_to: Option<String>,
    pub output: Option<serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
}

fn row_to_job_definition(row: &sqlx::sqlite::SqliteRow) -> JobDefinition {
    let config_json: String = row.get("config");
    let config: serde_json::Value =
        serde_json::from_str(&config_json).unwrap_or(serde_json::Value::Null);
    JobDefinition {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        config,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn row_to_job_run(row: &sqlx::sqlite::SqliteRow) -> JobRun {
    let result_json: Option<String> = row.get("result");
    let result = result_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    JobRun {
        id: row.get("id"),
        definition_id: row.get("definition_id"),
        parent_id: row.get("parent_id"),
        status: row.get("status"),
        triggered_by: row.get("triggered_by"),
        result,
        error: row.get("error"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
    }
}

fn row_to_task(row: &sqlx::sqlite::SqliteRow) -> Task {
    let output_json: Option<String> = row.get("output");
    let output = output_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    Task {
        id: row.get("id"),
        run_id: row.get("run_id"),
        subject: row.get("subject"),
        status: row.get("status"),
        assigned_to: row.get("assigned_to"),
        output,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

pub async fn create_job_definition(
    pool: &SqlitePool,
    name: &str,
    description: &str,
    config: serde_json::Value,
) -> Result<JobDefinition> {
    let id = Uuid::new_v4().to_string();
    let config_json =
        serde_json::to_string(&config).map_err(|e| CortexError::Internal(e.to_string()))?;

    sqlx::query(
        "INSERT INTO job_definitions (id, name, description, config) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&id)
    .bind(name)
    .bind(description)
    .bind(&config_json)
    .execute(pool)
    .await
    .map_err(CortexError::Storage)?;

    get_job_definition(pool, &id)
        .await?
        .ok_or_else(|| CortexError::NotFound(format!("job_definition {id}")))
}

pub async fn get_job_definition(pool: &SqlitePool, id: &str) -> Result<Option<JobDefinition>> {
    let row = sqlx::query(
        "SELECT id, name, description, config, created_at, updated_at \
         FROM job_definitions WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(CortexError::Storage)?;

    Ok(row.as_ref().map(row_to_job_definition))
}

pub async fn list_job_definitions(pool: &SqlitePool) -> Result<Vec<JobDefinition>> {
    let rows = sqlx::query(
        "SELECT id, name, description, config, created_at, updated_at \
         FROM job_definitions ORDER BY created_at",
    )
    .fetch_all(pool)
    .await
    .map_err(CortexError::Storage)?;

    Ok(rows.iter().map(row_to_job_definition).collect())
}

pub async fn start_job_run(
    pool: &SqlitePool,
    definition_id: &str,
    triggered_by: &str,
    parent_id: Option<&str>,
) -> Result<JobRun> {
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO job_runs (id, definition_id, parent_id, status, triggered_by, started_at) \
         VALUES (?1, ?2, ?3, 'running', ?4, datetime('now'))",
    )
    .bind(&id)
    .bind(definition_id)
    .bind(parent_id)
    .bind(triggered_by)
    .execute(pool)
    .await
    .map_err(CortexError::Storage)?;

    get_job_run(pool, &id)
        .await?
        .ok_or_else(|| CortexError::NotFound(format!("job_run {id}")))
}

pub async fn get_job_run(pool: &SqlitePool, id: &str) -> Result<Option<JobRun>> {
    let row = sqlx::query(
        "SELECT id, definition_id, parent_id, status, triggered_by, result, error, \
         started_at, completed_at FROM job_runs WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(CortexError::Storage)?;

    Ok(row.as_ref().map(row_to_job_run))
}

pub async fn update_job_run(
    pool: &SqlitePool,
    id: &str,
    status: Option<&str>,
    result: Option<serde_json::Value>,
    error: Option<&str>,
) -> Result<JobRun> {
    let result_json = result
        .as_ref()
        .map(|v| serde_json::to_string(v).map_err(|e| CortexError::Internal(e.to_string())))
        .transpose()?;

    let terminal_statuses = ["completed", "failed", "cancelled"];
    let is_terminal = status
        .map(|s| terminal_statuses.contains(&s))
        .unwrap_or(false);

    if is_terminal {
        sqlx::query(
            "UPDATE job_runs SET \
             status = COALESCE(?1, status), \
             result = COALESCE(?2, result), \
             error = COALESCE(?3, error), \
             completed_at = datetime('now') \
             WHERE id = ?4",
        )
        .bind(status)
        .bind(result_json.as_deref())
        .bind(error)
        .bind(id)
        .execute(pool)
        .await
        .map_err(CortexError::Storage)?;
    } else {
        sqlx::query(
            "UPDATE job_runs SET \
             status = COALESCE(?1, status), \
             result = COALESCE(?2, result), \
             error = COALESCE(?3, error) \
             WHERE id = ?4",
        )
        .bind(status)
        .bind(result_json.as_deref())
        .bind(error)
        .bind(id)
        .execute(pool)
        .await
        .map_err(CortexError::Storage)?;
    }

    get_job_run(pool, id)
        .await?
        .ok_or_else(|| CortexError::NotFound(format!("job_run {id}")))
}

pub async fn list_job_runs(pool: &SqlitePool, status: Option<&str>) -> Result<Vec<JobRun>> {
    let rows = sqlx::query(
        "SELECT id, definition_id, parent_id, status, triggered_by, result, error, \
         started_at, completed_at FROM job_runs \
         WHERE (?1 IS NULL OR status = ?1) \
         ORDER BY started_at",
    )
    .bind(status)
    .fetch_all(pool)
    .await
    .map_err(CortexError::Storage)?;

    Ok(rows.iter().map(row_to_job_run).collect())
}

pub async fn create_task(
    pool: &SqlitePool,
    subject: &str,
    run_id: Option<&str>,
    assigned_to: Option<&str>,
) -> Result<Task> {
    let id = Uuid::new_v4().to_string();

    sqlx::query("INSERT INTO tasks (id, subject, run_id, assigned_to) VALUES (?1, ?2, ?3, ?4)")
        .bind(&id)
        .bind(subject)
        .bind(run_id)
        .bind(assigned_to)
        .execute(pool)
        .await
        .map_err(CortexError::Storage)?;

    get_task(pool, &id)
        .await?
        .ok_or_else(|| CortexError::NotFound(format!("task {id}")))
}

pub async fn get_task(pool: &SqlitePool, id: &str) -> Result<Option<Task>> {
    let row = sqlx::query(
        "SELECT id, run_id, subject, status, assigned_to, output, created_at, updated_at \
         FROM tasks WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(CortexError::Storage)?;

    Ok(row.as_ref().map(row_to_task))
}

pub async fn update_task(
    pool: &SqlitePool,
    id: &str,
    status: Option<&str>,
    assigned_to: Option<&str>,
    output: Option<serde_json::Value>,
) -> Result<Task> {
    let output_json = output
        .as_ref()
        .map(|v| serde_json::to_string(v).map_err(|e| CortexError::Internal(e.to_string())))
        .transpose()?;

    sqlx::query(
        "UPDATE tasks SET \
         status = COALESCE(?1, status), \
         assigned_to = COALESCE(?2, assigned_to), \
         output = COALESCE(?3, output), \
         updated_at = datetime('now') \
         WHERE id = ?4",
    )
    .bind(status)
    .bind(assigned_to)
    .bind(output_json.as_deref())
    .bind(id)
    .execute(pool)
    .await
    .map_err(CortexError::Storage)?;

    get_task(pool, id)
        .await?
        .ok_or_else(|| CortexError::NotFound(format!("task {id}")))
}

pub async fn list_tasks(
    pool: &SqlitePool,
    status: Option<&str>,
    assigned_to: Option<&str>,
    run_id: Option<&str>,
) -> Result<Vec<Task>> {
    let rows = sqlx::query(
        "SELECT id, run_id, subject, status, assigned_to, output, created_at, updated_at \
         FROM tasks \
         WHERE (?1 IS NULL OR status = ?1) \
         AND (?2 IS NULL OR assigned_to = ?2) \
         AND (?3 IS NULL OR run_id = ?3) \
         ORDER BY created_at",
    )
    .bind(status)
    .bind(assigned_to)
    .bind(run_id)
    .fetch_all(pool)
    .await
    .map_err(CortexError::Storage)?;

    Ok(rows.iter().map(row_to_task).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory_named;

    async fn make_pool(name: &str) -> SqlitePool {
        open_in_memory_named(name).await.expect("pool opens")
    }

    async fn make_definition(pool: &SqlitePool, job_name: &str) -> JobDefinition {
        create_job_definition(
            pool,
            job_name,
            "A test job",
            serde_json::json!({"key": "val"}),
        )
        .await
        .expect("create definition")
    }

    #[tokio::test]
    async fn test_job_definition_crud() {
        let pool = make_pool("jobs_test_def_crud").await;

        let def = make_definition(&pool, "test-job-def-crud").await;
        assert!(!def.id.is_empty());
        assert_eq!(def.name, "test-job-def-crud");
        assert_eq!(def.description, "A test job");
        assert_eq!(def.config["key"], "val");

        let fetched = get_job_definition(&pool, &def.id)
            .await
            .expect("get succeeds")
            .expect("definition exists");
        assert_eq!(fetched.id, def.id);
        assert_eq!(fetched.name, def.name);

        let all = list_job_definitions(&pool).await.expect("list succeeds");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, def.id);
    }

    #[tokio::test]
    async fn test_job_run_lifecycle() {
        let pool = make_pool("jobs_test_run_lifecycle").await;
        let def = make_definition(&pool, "test-job-run-lifecycle").await;

        let run = start_job_run(&pool, &def.id, "agent-1", None)
            .await
            .expect("start run");
        assert_eq!(run.status, "running");
        assert!(run.started_at.is_some());
        assert!(run.completed_at.is_none());

        let updated = update_job_run(
            &pool,
            &run.id,
            Some("completed"),
            Some(serde_json::json!({"items": 42})),
            None,
        )
        .await
        .expect("update run");
        assert_eq!(updated.status, "completed");
        assert!(updated.completed_at.is_some());
        assert_eq!(updated.result.as_ref().unwrap()["items"], 42);

        let runs = list_job_runs(&pool, Some("completed"))
            .await
            .expect("list runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, run.id);

        let empty = list_job_runs(&pool, Some("running"))
            .await
            .expect("list running");
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn test_sub_job_runs() {
        let pool = make_pool("jobs_test_sub_runs").await;
        let def = make_definition(&pool, "test-job-sub-runs").await;

        let parent = start_job_run(&pool, &def.id, "agent-1", None)
            .await
            .expect("start parent");
        let child = start_job_run(&pool, &def.id, "agent-2", Some(&parent.id))
            .await
            .expect("start child");

        assert_eq!(child.parent_id.as_deref(), Some(parent.id.as_str()));
    }

    #[tokio::test]
    async fn test_task_crud() {
        let pool = make_pool("jobs_test_task_crud").await;
        let def = make_definition(&pool, "test-job-task-crud").await;
        let run = start_job_run(&pool, &def.id, "agent-1", None)
            .await
            .expect("start run");

        let task = create_task(
            &pool,
            "do something",
            Some(&run.id),
            Some("agent-task-crud"),
        )
        .await
        .expect("create task");
        assert_eq!(task.subject, "do something");
        assert_eq!(task.status, "pending");
        assert_eq!(task.run_id.as_deref(), Some(run.id.as_str()));

        let fetched = get_task(&pool, &task.id)
            .await
            .expect("get task")
            .expect("task exists");
        assert_eq!(fetched.id, task.id);

        let updated = update_task(
            &pool,
            &task.id,
            Some("completed"),
            None,
            Some(serde_json::json!({"done": true})),
        )
        .await
        .expect("update task");
        assert_eq!(updated.status, "completed");
        assert_eq!(updated.output.as_ref().unwrap()["done"], true);

        let tasks_by_run = list_tasks(&pool, None, None, Some(&run.id))
            .await
            .expect("list by run");
        assert_eq!(tasks_by_run.len(), 1);

        let tasks_by_status = list_tasks(&pool, Some("completed"), None, None)
            .await
            .expect("list by status");
        assert_eq!(tasks_by_status.len(), 1);

        let tasks_by_agent = list_tasks(&pool, None, Some("agent-task-crud"), None)
            .await
            .expect("list by agent");
        assert_eq!(tasks_by_agent.len(), 1);
    }
}
