pub mod artifacts;
pub mod decisions;
pub mod hatcheries;
pub mod jobs;
pub mod memory;
pub mod models;
pub mod postgres;
pub mod sqlite;
pub mod tables;
mod trait_def;
pub use postgres::PostgresDatabase;
pub use sqlite::SqliteDatabase;
#[allow(unused_imports)]
pub use trait_def::{ArtifactStore, Database, DecisionStore, HatcheryStore, JobStore, MemoryStore};

#[allow(unused_imports)]
pub use models::*;

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::error::OverseerError;

pub async fn create_embedding_table(
    pool: &SqlitePool,
    provider_name: &str,
    dimensions: usize,
) -> Result<(), OverseerError> {
    let sql = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memory_embeddings_{provider_name} USING vec0(embedding float[{dimensions}])"
    );
    sqlx::raw_sql(&sql)
        .execute(pool as &SqlitePool)
        .await
        .map_err(OverseerError::Storage)?;
    Ok(())
}

/// Open a database from a URL, dispatching on the scheme.
pub async fn open_from_url(url: &str) -> std::result::Result<Arc<dyn Database>, OverseerError> {
    if let Some(path) = url.strip_prefix("sqlite://") {
        Ok(Arc::new(SqliteDatabase::open(path).await?))
    } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        Ok(Arc::new(PostgresDatabase::open(url).await?))
    } else {
        Err(OverseerError::Validation(format!(
            "unsupported database URL scheme: {url}"
        )))
    }
}

/// Open (or create) a SQLite database at the given file path.
pub async fn open(path: &str) -> Result<SqlitePool, OverseerError> {
    SqliteDatabase::open(path).await.map(|db| db.pool)
}

/// Open an in-memory SQLite database (useful for tests).
pub async fn open_in_memory() -> Result<SqlitePool, OverseerError> {
    SqliteDatabase::open_in_memory().await.map(|db| db.pool)
}

/// Open a named in-memory SQLite database. Each unique name gets its own
/// isolated in-memory database, which is useful for test isolation.
pub async fn open_in_memory_named(name: &str) -> Result<SqlitePool, OverseerError> {
    SqliteDatabase::open_in_memory_named(name)
        .await
        .map(|db| db.pool)
}

/// Shared conformance test that runs against any Database implementation.
/// Verifies that both SQLite and Postgres backends behave identically.
pub(crate) async fn trait_conformance_suite(db: Arc<dyn Database>) {
    // Artifacts
    let artifact = db
        .insert_artifact("test-id", "test.txt", "text/plain", 42, None)
        .await
        .expect("insert artifact");
    assert_eq!(artifact.name, "test.txt");

    let fetched = db.get_artifact("test-id").await.expect("get artifact");
    assert!(fetched.is_some());

    let listed = db.list_artifacts(None).await.expect("list artifacts");
    assert!(!listed.is_empty());

    // Job definitions
    let def = db
        .create_job_definition("conformance-job", "test", serde_json::json!({}))
        .await
        .expect("create job def");
    assert_eq!(def.name, "conformance-job");

    let fetched_def = db.get_job_definition(&def.id).await.expect("get def");
    assert!(fetched_def.is_some());

    let defs = db.list_job_definitions().await.expect("list defs");
    assert!(!defs.is_empty());

    // Job runs
    let run = db
        .start_job_run(&def.id, "test-agent", None, None)
        .await
        .expect("start run");
    assert_eq!(run.status, models::JobRunStatus::Pending);

    let updated = db
        .update_job_run(
            &run.id,
            Some("completed"),
            Some(serde_json::json!({"ok": true})),
            None,
        )
        .await
        .expect("update run");
    assert_eq!(updated.status, models::JobRunStatus::Completed);
    assert!(updated.completed_at.is_some());

    // Tasks
    let task = db
        .create_task("do something", Some(&run.id), Some("agent"))
        .await
        .expect("create task");
    assert_eq!(task.status, models::TaskStatus::Pending);

    let updated_task = db
        .update_task(&task.id, Some("completed"), None, None)
        .await
        .expect("update task");
    assert_eq!(updated_task.status, models::TaskStatus::Completed);

    // Decisions
    let dec = db
        .log_decision(
            "agent",
            "context",
            "decision",
            "reasoning",
            &["tag".to_string()],
            None,
        )
        .await
        .expect("log decision");
    assert_eq!(dec.agent, "agent");
    assert_eq!(dec.tags, vec!["tag"]);

    let queried = db
        .query_decisions(Some("agent"), None, 10)
        .await
        .expect("query decisions");
    assert!(!queried.is_empty());

    // Hatcheries
    let hatchery = db
        .register_hatchery(
            "conformance-hatchery",
            serde_json::json!({"arch": "x86_64"}),
            4,
        )
        .await
        .expect("register hatchery");
    assert_eq!(hatchery.name, "conformance-hatchery");
    assert_eq!(hatchery.status, models::HatcheryStatus::Online);

    let fetched_h = db.get_hatchery(&hatchery.id).await.expect("get hatchery");
    assert!(fetched_h.is_some());

    let by_name = db
        .get_hatchery_by_name("conformance-hatchery")
        .await
        .expect("get by name");
    assert!(by_name.is_some());

    let heartbeated = db
        .heartbeat_hatchery(&hatchery.id, "degraded", 2)
        .await
        .expect("heartbeat");
    assert_eq!(heartbeated.status, models::HatcheryStatus::Degraded);

    let hatcheries = db.list_hatcheries(None).await.expect("list hatcheries");
    assert!(!hatcheries.is_empty());

    let assigned = db
        .assign_job_to_hatchery(&run.id, &hatchery.id)
        .await
        .expect("assign job");
    assert_eq!(assigned.id, run.id);

    let h_runs = db
        .list_hatchery_job_runs(&hatchery.id, None)
        .await
        .expect("list hatchery runs");
    assert!(!h_runs.is_empty());

    db.deregister_hatchery(&hatchery.id)
        .await
        .expect("deregister");
    let gone = db
        .get_hatchery(&hatchery.id)
        .await
        .expect("get after delete");
    assert!(gone.is_none());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_database_opens_and_creates_schema() {
        let pool = open_in_memory().await.expect("pool opens");

        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type IN ('table', 'shadow') ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .expect("query succeeds");

        let expected = [
            "artifacts",
            "decisions",
            "job_definitions",
            "job_runs",
            "memory_links",
            "memories",
            "tasks",
        ];

        for name in &expected {
            assert!(
                tables.iter().any(|t| t == name),
                "expected table '{name}' not found; got: {tables:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_vec0_extension_loaded() {
        let pool = open_in_memory_named("db_test_vec0_loaded")
            .await
            .expect("pool opens");
        create_embedding_table(&pool, "vec0test", 128)
            .await
            .expect("vec0 table creation should work");
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'memory_embeddings_vec0test'",
        )
        .fetch_all(&pool)
        .await
        .expect("query");
        assert!(!tables.is_empty(), "vec0 virtual table should exist");
    }

    #[tokio::test]
    async fn test_create_embedding_table() {
        let pool = open_in_memory_named("db_test_create_emb_table")
            .await
            .expect("pool opens");
        create_embedding_table(&pool, "test_provider", 512)
            .await
            .expect("create table");
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'memory_embeddings_test_provider'",
        )
        .fetch_all(&pool)
        .await
        .expect("query");
        assert_eq!(tables.len(), 1);
    }

    #[tokio::test]
    async fn test_create_embedding_table_idempotent() {
        let pool = open_in_memory_named("db_test_emb_idempotent")
            .await
            .expect("pool opens");
        create_embedding_table(&pool, "dup", 384)
            .await
            .expect("first create");
        create_embedding_table(&pool, "dup", 384)
            .await
            .expect("second create should be ok");
    }

    #[tokio::test]
    async fn test_open_from_url_sqlite() {
        let db = open_from_url("sqlite://:memory:")
            .await
            .expect("sqlite URL should work");
        // Verify we can use it
        let defs = db.list_job_definitions().await.expect("query works");
        assert!(defs.is_empty());
    }

    #[tokio::test]
    async fn test_open_from_url_unknown_scheme() {
        let result = open_from_url("unknown://foo").await;
        assert!(matches!(result, Err(OverseerError::Validation(_))));
    }

    #[tokio::test]
    #[ignore] // requires a network round-trip to a dead host (~30s TCP timeout)
    async fn test_open_from_url_postgres_stub() {
        let result = open_from_url("postgres://127.0.0.1:1/nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_sqlite_trait_conformance() {
        let db: Arc<dyn Database> = Arc::new(
            SqliteDatabase::open_in_memory_named("trait_conformance")
                .await
                .expect("open"),
        );
        super::trait_conformance_suite(db).await;
    }
}
