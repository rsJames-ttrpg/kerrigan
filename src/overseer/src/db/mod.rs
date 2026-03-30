pub mod artifacts;
pub mod decisions;
pub mod jobs;
pub mod memory;
pub mod models;
pub mod sqlite;
mod trait_def;
pub use sqlite::SqliteDatabase;
#[allow(unused_imports)]
pub use trait_def::Database;

#[allow(unused_imports)]
pub use models::*;

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
}
