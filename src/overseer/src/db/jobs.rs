use sea_query::{Expr, Order, Query, SqliteQueryBuilder};
use sea_query_binder::SqlxBinder;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

pub use super::models::{JobDefinition, JobRun, Task};
use super::tables::{JobDefinitions, JobRuns, Tasks};
use crate::error::{OverseerError, Result};

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
        serde_json::to_string(&config).map_err(|e| OverseerError::Internal(e.to_string()))?;

    let (sql, values) = Query::insert()
        .into_table(JobDefinitions::Table)
        .columns([
            JobDefinitions::Id,
            JobDefinitions::Name,
            JobDefinitions::Description,
            JobDefinitions::Config,
        ])
        .values_panic([
            id.into(),
            name.into(),
            description.into(),
            config_json.into(),
        ])
        .returning(Query::returning().columns([
            JobDefinitions::Id,
            JobDefinitions::Name,
            JobDefinitions::Description,
            JobDefinitions::Config,
            JobDefinitions::CreatedAt,
            JobDefinitions::UpdatedAt,
        ]))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_one(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row_to_job_definition(&row))
}

pub async fn get_job_definition(pool: &SqlitePool, id: &str) -> Result<Option<JobDefinition>> {
    let (sql, values) = Query::select()
        .columns([
            JobDefinitions::Id,
            JobDefinitions::Name,
            JobDefinitions::Description,
            JobDefinitions::Config,
            JobDefinitions::CreatedAt,
            JobDefinitions::UpdatedAt,
        ])
        .from(JobDefinitions::Table)
        .and_where(Expr::col(JobDefinitions::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row.as_ref().map(row_to_job_definition))
}

pub async fn list_job_definitions(pool: &SqlitePool) -> Result<Vec<JobDefinition>> {
    let (sql, values) = Query::select()
        .columns([
            JobDefinitions::Id,
            JobDefinitions::Name,
            JobDefinitions::Description,
            JobDefinitions::Config,
            JobDefinitions::CreatedAt,
            JobDefinitions::UpdatedAt,
        ])
        .from(JobDefinitions::Table)
        .order_by(JobDefinitions::CreatedAt, Order::Asc)
        .build_sqlx(SqliteQueryBuilder);

    let rows = sqlx::query_with(&sql, values)
        .fetch_all(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(rows.iter().map(row_to_job_definition).collect())
}

pub async fn start_job_run(
    pool: &SqlitePool,
    definition_id: &str,
    triggered_by: &str,
    parent_id: Option<&str>,
) -> Result<JobRun> {
    let id = Uuid::new_v4().to_string();

    let (sql, values) = Query::insert()
        .into_table(JobRuns::Table)
        .columns([
            JobRuns::Id,
            JobRuns::DefinitionId,
            JobRuns::ParentId,
            JobRuns::Status,
            JobRuns::TriggeredBy,
            JobRuns::StartedAt,
        ])
        .values_panic([
            id.into(),
            definition_id.into(),
            parent_id.map(|s| s.to_string()).into(),
            "running".into(),
            triggered_by.into(),
            Expr::cust("datetime('now')"),
        ])
        .returning(Query::returning().columns([
            JobRuns::Id,
            JobRuns::DefinitionId,
            JobRuns::ParentId,
            JobRuns::Status,
            JobRuns::TriggeredBy,
            JobRuns::Result,
            JobRuns::Error,
            JobRuns::StartedAt,
            JobRuns::CompletedAt,
        ]))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_one(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row_to_job_run(&row))
}

pub async fn get_job_run(pool: &SqlitePool, id: &str) -> Result<Option<JobRun>> {
    let (sql, values) = Query::select()
        .columns([
            JobRuns::Id,
            JobRuns::DefinitionId,
            JobRuns::ParentId,
            JobRuns::Status,
            JobRuns::TriggeredBy,
            JobRuns::Result,
            JobRuns::Error,
            JobRuns::StartedAt,
            JobRuns::CompletedAt,
        ])
        .from(JobRuns::Table)
        .and_where(Expr::col(JobRuns::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?;

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
        .map(|v| serde_json::to_string(v).map_err(|e| OverseerError::Internal(e.to_string())))
        .transpose()?;

    let terminal_statuses = ["completed", "failed", "cancelled"];
    let is_terminal = status
        .map(|s| terminal_statuses.contains(&s))
        .unwrap_or(false);

    let mut query = Query::update();
    query.table(JobRuns::Table);

    if let Some(s) = status {
        query.value(JobRuns::Status, s);
    }
    if let Some(ref r) = result_json {
        query.value(JobRuns::Result, r.as_str());
    }
    if let Some(e) = error {
        query.value(JobRuns::Error, e);
    }
    if is_terminal {
        query.value(JobRuns::CompletedAt, Expr::cust("datetime('now')"));
    }

    query.and_where(Expr::col(JobRuns::Id).eq(id));

    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);

    sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    get_job_run(pool, id)
        .await?
        .ok_or_else(|| OverseerError::NotFound(format!("job_run {id}")))
}

pub async fn list_job_runs(pool: &SqlitePool, status: Option<&str>) -> Result<Vec<JobRun>> {
    let mut query = Query::select();
    query
        .columns([
            JobRuns::Id,
            JobRuns::DefinitionId,
            JobRuns::ParentId,
            JobRuns::Status,
            JobRuns::TriggeredBy,
            JobRuns::Result,
            JobRuns::Error,
            JobRuns::StartedAt,
            JobRuns::CompletedAt,
        ])
        .from(JobRuns::Table);

    if let Some(s) = status {
        query.and_where(Expr::col(JobRuns::Status).eq(s));
    }

    query.order_by(JobRuns::StartedAt, Order::Asc);

    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);

    let rows = sqlx::query_with(&sql, values)
        .fetch_all(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(rows.iter().map(row_to_job_run).collect())
}

pub async fn create_task(
    pool: &SqlitePool,
    subject: &str,
    run_id: Option<&str>,
    assigned_to: Option<&str>,
) -> Result<Task> {
    let id = Uuid::new_v4().to_string();

    let (sql, values) = Query::insert()
        .into_table(Tasks::Table)
        .columns([Tasks::Id, Tasks::Subject, Tasks::RunId, Tasks::AssignedTo])
        .values_panic([
            id.into(),
            subject.into(),
            run_id.map(|s| s.to_string()).into(),
            assigned_to.map(|s| s.to_string()).into(),
        ])
        .returning(Query::returning().columns([
            Tasks::Id,
            Tasks::RunId,
            Tasks::Subject,
            Tasks::Status,
            Tasks::AssignedTo,
            Tasks::Output,
            Tasks::CreatedAt,
            Tasks::UpdatedAt,
        ]))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_one(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row_to_task(&row))
}

pub async fn get_task(pool: &SqlitePool, id: &str) -> Result<Option<Task>> {
    let (sql, values) = Query::select()
        .columns([
            Tasks::Id,
            Tasks::RunId,
            Tasks::Subject,
            Tasks::Status,
            Tasks::AssignedTo,
            Tasks::Output,
            Tasks::CreatedAt,
            Tasks::UpdatedAt,
        ])
        .from(Tasks::Table)
        .and_where(Expr::col(Tasks::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?;

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
        .map(|v| serde_json::to_string(v).map_err(|e| OverseerError::Internal(e.to_string())))
        .transpose()?;

    let mut query = Query::update();
    query.table(Tasks::Table);

    if let Some(s) = status {
        query.value(Tasks::Status, s);
    }
    if let Some(a) = assigned_to {
        query.value(Tasks::AssignedTo, a);
    }
    if let Some(ref o) = output_json {
        query.value(Tasks::Output, o.as_str());
    }

    query.value(Tasks::UpdatedAt, Expr::cust("datetime('now')"));
    query.and_where(Expr::col(Tasks::Id).eq(id));

    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);

    sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    get_task(pool, id)
        .await?
        .ok_or_else(|| OverseerError::NotFound(format!("task {id}")))
}

pub async fn list_tasks(
    pool: &SqlitePool,
    status: Option<&str>,
    assigned_to: Option<&str>,
    run_id: Option<&str>,
) -> Result<Vec<Task>> {
    let mut query = Query::select();
    query
        .columns([
            Tasks::Id,
            Tasks::RunId,
            Tasks::Subject,
            Tasks::Status,
            Tasks::AssignedTo,
            Tasks::Output,
            Tasks::CreatedAt,
            Tasks::UpdatedAt,
        ])
        .from(Tasks::Table);

    if let Some(s) = status {
        query.and_where(Expr::col(Tasks::Status).eq(s));
    }
    if let Some(a) = assigned_to {
        query.and_where(Expr::col(Tasks::AssignedTo).eq(a));
    }
    if let Some(r) = run_id {
        query.and_where(Expr::col(Tasks::RunId).eq(r));
    }

    query.order_by(Tasks::CreatedAt, Order::Asc);

    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);

    let rows = sqlx::query_with(&sql, values)
        .fetch_all(pool)
        .await
        .map_err(OverseerError::Storage)?;

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

    #[tokio::test]
    async fn test_update_job_run_nonexistent() {
        let pool = make_pool("jobs_test_update_run_notfound").await;
        let result = update_job_run(&pool, "no-such-id", Some("completed"), None, None).await;
        assert!(matches!(result, Err(OverseerError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_update_task_nonexistent() {
        let pool = make_pool("jobs_test_update_task_notfound").await;
        let result = update_task(&pool, "no-such-id", Some("completed"), None, None).await;
        assert!(matches!(result, Err(OverseerError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_terminal_statuses_set_completed_at() {
        let pool = make_pool("jobs_test_terminal_statuses").await;
        let def = make_definition(&pool, "test-terminal").await;

        for status in ["failed", "cancelled"] {
            let run = start_job_run(&pool, &def.id, "agent", None)
                .await
                .expect("start run");
            assert!(run.completed_at.is_none());

            let updated = update_job_run(&pool, &run.id, Some(status), None, None)
                .await
                .expect("update run");
            assert_eq!(updated.status, status);
            assert!(
                updated.completed_at.is_some(),
                "{status} should set completed_at"
            );
        }
    }

    #[tokio::test]
    async fn test_non_terminal_status_no_completed_at() {
        let pool = make_pool("jobs_test_non_terminal").await;
        let def = make_definition(&pool, "test-non-terminal").await;

        let run = start_job_run(&pool, &def.id, "agent", None)
            .await
            .expect("start run");

        let updated = update_job_run(&pool, &run.id, Some("pending"), None, None)
            .await
            .expect("update run");
        assert_eq!(updated.status, "pending");
        assert!(updated.completed_at.is_none());
    }
}
