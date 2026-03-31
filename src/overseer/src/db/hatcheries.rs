use chrono::NaiveDateTime;
use sea_query::{Expr, Order, Query, SqliteQueryBuilder};
use sea_query_binder::SqlxBinder;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

pub use super::models::{Hatchery, HatcheryStatus, JobRun};
use super::tables::{Hatcheries, JobRuns};
use crate::error::{OverseerError, Result};

fn row_to_hatchery(row: &sqlx::sqlite::SqliteRow) -> Hatchery {
    let caps_json: String = row.get("capabilities");
    let capabilities: serde_json::Value =
        serde_json::from_str(&caps_json).unwrap_or(serde_json::Value::Object(Default::default()));
    Hatchery {
        id: row.get("id"),
        name: row.get("name"),
        status: row
            .get::<String, _>("status")
            .parse()
            .unwrap_or(HatcheryStatus::Offline),
        capabilities,
        max_concurrency: row.get("max_concurrency"),
        active_drones: row.get("active_drones"),
        last_heartbeat_at: row.get::<NaiveDateTime, _>("last_heartbeat_at").and_utc(),
        created_at: row.get::<NaiveDateTime, _>("created_at").and_utc(),
        updated_at: row.get::<NaiveDateTime, _>("updated_at").and_utc(),
    }
}

pub async fn register_hatchery(
    pool: &SqlitePool,
    name: &str,
    capabilities: serde_json::Value,
    max_concurrency: i32,
) -> Result<Hatchery> {
    let id = Uuid::new_v4().to_string();
    let caps_json =
        serde_json::to_string(&capabilities).map_err(|e| OverseerError::Internal(e.to_string()))?;

    let (sql, values) = Query::insert()
        .into_table(Hatcheries::Table)
        .columns([
            Hatcheries::Id,
            Hatcheries::Name,
            Hatcheries::Capabilities,
            Hatcheries::MaxConcurrency,
        ])
        .values([
            id.into(),
            name.into(),
            caps_json.into(),
            max_concurrency.into(),
        ])
        .map_err(|e| OverseerError::Internal(format!("query build error: {e}")))?
        .returning(Query::returning().columns([
            Hatcheries::Id,
            Hatcheries::Name,
            Hatcheries::Status,
            Hatcheries::Capabilities,
            Hatcheries::MaxConcurrency,
            Hatcheries::ActiveDrones,
            Hatcheries::LastHeartbeatAt,
            Hatcheries::CreatedAt,
            Hatcheries::UpdatedAt,
        ]))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_one(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row_to_hatchery(&row))
}

pub async fn get_hatchery(pool: &SqlitePool, id: &str) -> Result<Option<Hatchery>> {
    let (sql, values) = Query::select()
        .columns([
            Hatcheries::Id,
            Hatcheries::Name,
            Hatcheries::Status,
            Hatcheries::Capabilities,
            Hatcheries::MaxConcurrency,
            Hatcheries::ActiveDrones,
            Hatcheries::LastHeartbeatAt,
            Hatcheries::CreatedAt,
            Hatcheries::UpdatedAt,
        ])
        .from(Hatcheries::Table)
        .and_where(Expr::col(Hatcheries::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row.as_ref().map(row_to_hatchery))
}

pub async fn get_hatchery_by_name(pool: &SqlitePool, name: &str) -> Result<Option<Hatchery>> {
    let (sql, values) = Query::select()
        .columns([
            Hatcheries::Id,
            Hatcheries::Name,
            Hatcheries::Status,
            Hatcheries::Capabilities,
            Hatcheries::MaxConcurrency,
            Hatcheries::ActiveDrones,
            Hatcheries::LastHeartbeatAt,
            Hatcheries::CreatedAt,
            Hatcheries::UpdatedAt,
        ])
        .from(Hatcheries::Table)
        .and_where(Expr::col(Hatcheries::Name).eq(name))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row.as_ref().map(row_to_hatchery))
}

pub async fn heartbeat_hatchery(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    active_drones: i32,
) -> Result<Hatchery> {
    let (sql, values) = Query::update()
        .table(Hatcheries::Table)
        .value(Hatcheries::Status, status)
        .value(Hatcheries::ActiveDrones, active_drones)
        .value(Hatcheries::LastHeartbeatAt, Expr::cust("datetime('now')"))
        .value(Hatcheries::UpdatedAt, Expr::cust("datetime('now')"))
        .and_where(Expr::col(Hatcheries::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    let result = sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    if result.rows_affected() == 0 {
        return Err(OverseerError::NotFound(format!("hatchery {id}")));
    }

    get_hatchery(pool, id)
        .await?
        .ok_or_else(|| OverseerError::NotFound(format!("hatchery {id}")))
}

pub async fn list_hatcheries(pool: &SqlitePool, status: Option<&str>) -> Result<Vec<Hatchery>> {
    let mut query = Query::select();
    query
        .columns([
            Hatcheries::Id,
            Hatcheries::Name,
            Hatcheries::Status,
            Hatcheries::Capabilities,
            Hatcheries::MaxConcurrency,
            Hatcheries::ActiveDrones,
            Hatcheries::LastHeartbeatAt,
            Hatcheries::CreatedAt,
            Hatcheries::UpdatedAt,
        ])
        .from(Hatcheries::Table);

    if let Some(s) = status {
        query.and_where(Expr::col(Hatcheries::Status).eq(s));
    }

    query.order_by(Hatcheries::CreatedAt, Order::Asc);

    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);

    let rows = sqlx::query_with(&sql, values)
        .fetch_all(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(rows.iter().map(row_to_hatchery).collect())
}

pub async fn deregister_hatchery(pool: &SqlitePool, id: &str) -> Result<()> {
    // First check the hatchery exists
    if get_hatchery(pool, id).await?.is_none() {
        return Err(OverseerError::NotFound(format!("hatchery {id}")));
    }

    // Clear hatchery_id on any associated job runs to avoid FK constraint failure
    let (null_sql, null_values) = Query::update()
        .table(JobRuns::Table)
        .value(JobRuns::HatcheryId, Option::<String>::None)
        .and_where(Expr::col(JobRuns::HatcheryId).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    sqlx::query_with(&null_sql, null_values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    let (sql, values) = Query::delete()
        .from_table(Hatcheries::Table)
        .and_where(Expr::col(Hatcheries::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(())
}

pub async fn assign_job_to_hatchery(
    pool: &SqlitePool,
    job_run_id: &str,
    hatchery_id: &str,
) -> Result<JobRun> {
    let (sql, values) = Query::update()
        .table(JobRuns::Table)
        .value(JobRuns::HatcheryId, hatchery_id)
        .and_where(Expr::col(JobRuns::Id).eq(job_run_id))
        .build_sqlx(SqliteQueryBuilder);

    let result = sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    if result.rows_affected() == 0 {
        return Err(OverseerError::NotFound(format!("job_run {job_run_id}")));
    }

    super::jobs::get_job_run(pool, job_run_id)
        .await?
        .ok_or_else(|| OverseerError::NotFound(format!("job_run {job_run_id}")))
}

pub async fn list_hatchery_job_runs(
    pool: &SqlitePool,
    hatchery_id: &str,
    status: Option<&str>,
) -> Result<Vec<JobRun>> {
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
        .from(JobRuns::Table)
        .and_where(Expr::col(JobRuns::HatcheryId).eq(hatchery_id));

    if let Some(s) = status {
        query.and_where(Expr::col(JobRuns::Status).eq(s));
    }

    query.order_by(JobRuns::StartedAt, Order::Asc);

    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);

    let rows = sqlx::query_with(&sql, values)
        .fetch_all(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(rows.iter().map(super::jobs::row_to_job_run).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::jobs::{create_job_definition, start_job_run};
    use crate::db::open_in_memory_named;

    async fn make_pool(name: &str) -> SqlitePool {
        open_in_memory_named(name).await.expect("pool opens")
    }

    async fn make_hatchery(pool: &SqlitePool, name: &str) -> Hatchery {
        register_hatchery(pool, name, serde_json::json!({"arch": "x86_64"}), 4)
            .await
            .expect("register hatchery")
    }

    #[tokio::test]
    async fn test_register_and_get_hatchery() {
        let pool = make_pool("hatcheries_test_register_get").await;

        let h = make_hatchery(&pool, "test-hatchery-rg").await;
        assert!(!h.id.is_empty());
        assert_eq!(h.name, "test-hatchery-rg");
        assert_eq!(h.status, HatcheryStatus::Online);
        assert_eq!(h.max_concurrency, 4);
        assert_eq!(h.active_drones, 0);
        assert_eq!(h.capabilities["arch"], "x86_64");

        let fetched = get_hatchery(&pool, &h.id)
            .await
            .expect("get succeeds")
            .expect("hatchery exists");
        assert_eq!(fetched.id, h.id);
        assert_eq!(fetched.name, h.name);

        let by_name = get_hatchery_by_name(&pool, "test-hatchery-rg")
            .await
            .expect("get by name succeeds")
            .expect("hatchery found by name");
        assert_eq!(by_name.id, h.id);
    }

    #[tokio::test]
    async fn test_heartbeat_updates_status() {
        let pool = make_pool("hatcheries_test_heartbeat").await;

        let h = make_hatchery(&pool, "test-hatchery-hb").await;
        assert_eq!(h.status, HatcheryStatus::Online);
        assert_eq!(h.active_drones, 0);

        let updated = heartbeat_hatchery(&pool, &h.id, "degraded", 2)
            .await
            .expect("heartbeat succeeds");
        assert_eq!(updated.status, HatcheryStatus::Degraded);
        assert_eq!(updated.active_drones, 2);
        assert_eq!(updated.id, h.id);
    }

    #[tokio::test]
    async fn test_list_hatcheries_with_filter() {
        let pool = make_pool("hatcheries_test_list_filter").await;

        let h1 = make_hatchery(&pool, "hatchery-list-online").await;
        let h2 = make_hatchery(&pool, "hatchery-list-offline").await;

        heartbeat_hatchery(&pool, &h2.id, "offline", 0)
            .await
            .expect("set offline");

        let all = list_hatcheries(&pool, None).await.expect("list all");
        assert_eq!(all.len(), 2);

        let online = list_hatcheries(&pool, Some("online"))
            .await
            .expect("list online");
        assert_eq!(online.len(), 1);
        assert_eq!(online[0].id, h1.id);

        let offline = list_hatcheries(&pool, Some("offline"))
            .await
            .expect("list offline");
        assert_eq!(offline.len(), 1);
        assert_eq!(offline[0].id, h2.id);
    }

    #[tokio::test]
    async fn test_deregister_hatchery() {
        let pool = make_pool("hatcheries_test_deregister").await;

        let h = make_hatchery(&pool, "test-hatchery-dereg").await;

        deregister_hatchery(&pool, &h.id)
            .await
            .expect("deregister succeeds");

        let gone = get_hatchery(&pool, &h.id).await.expect("get after delete");
        assert!(gone.is_none());
    }

    #[tokio::test]
    async fn test_assign_job_to_hatchery() {
        let pool = make_pool("hatcheries_test_assign_job").await;

        let h = make_hatchery(&pool, "test-hatchery-assign").await;
        let def = create_job_definition(&pool, "test-job-assign", "a job", serde_json::json!({}))
            .await
            .expect("create job def");
        let run = start_job_run(&pool, &def.id, "agent", None)
            .await
            .expect("start run");

        let assigned = assign_job_to_hatchery(&pool, &run.id, &h.id)
            .await
            .expect("assign job");
        assert_eq!(assigned.id, run.id);

        let h_runs = list_hatchery_job_runs(&pool, &h.id, None)
            .await
            .expect("list hatchery runs");
        assert_eq!(h_runs.len(), 1);
        assert_eq!(h_runs[0].id, run.id);
    }

    #[tokio::test]
    async fn test_heartbeat_nonexistent() {
        let pool = make_pool("hatcheries_test_heartbeat_notfound").await;

        let result = heartbeat_hatchery(&pool, "no-such-id", "online", 0).await;
        assert!(matches!(result, Err(OverseerError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_duplicate_name_rejected() {
        let pool = make_pool("hatcheries_test_dup_name").await;

        make_hatchery(&pool, "duplicate-name").await;

        let result = register_hatchery(&pool, "duplicate-name", serde_json::json!({}), 2).await;
        assert!(result.is_err());
    }
}
