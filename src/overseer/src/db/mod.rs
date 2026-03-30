pub mod artifacts;
pub mod decisions;
pub mod jobs;
pub mod memory;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;

use crate::error::OverseerError;

/// Load the sqlite-vec extension into every new SQLite connection.
///
/// SAFETY: `sqlite3_vec_init` is the standard entry-point exported by the
/// sqlite-vec shared library.  `sqlite3_auto_extension` expects a function
/// pointer with the C signature `int(*)(sqlite3*,char**,const sqlite3_api_routines*)`,
/// but we register it via the opaque-pointer / transmute pattern that SQLite's
/// own extension-loading infrastructure uses.
fn register_vec_extension() {
    unsafe {
        libsqlite3_sys::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    }
}

async fn init_pool(opts: SqliteConnectOptions) -> Result<SqlitePool, OverseerError> {
    register_vec_extension();

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .map_err(OverseerError::Storage)?;

    sqlx::raw_sql(include_str!("schema.sql"))
        .execute(&pool as &SqlitePool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(pool)
}

/// Open (or create) a SQLite database at the given file path.
pub async fn open(path: &str) -> Result<SqlitePool, OverseerError> {
    let opts = SqliteConnectOptions::from_str(&format!("sqlite:{path}"))
        .map_err(OverseerError::Storage)?
        .create_if_missing(true)
        .pragma("journal_mode", "WAL")
        .pragma("foreign_keys", "ON");

    init_pool(opts).await
}

/// Open an in-memory SQLite database (useful for tests).
pub async fn open_in_memory() -> Result<SqlitePool, OverseerError> {
    open_in_memory_named("overseer_test").await
}

/// Open a named in-memory SQLite database. Each unique name gets its own
/// isolated in-memory database, which is useful for test isolation.
pub async fn open_in_memory_named(name: &str) -> Result<SqlitePool, OverseerError> {
    // For an in-memory database shared across pool connections we use a named
    // in-memory URI with cache=shared so all connections see the same data.
    let opts =
        SqliteConnectOptions::from_str(&format!("sqlite:file:{name}?mode=memory&cache=shared"))
            .map_err(OverseerError::Storage)?
            .pragma("journal_mode", "WAL")
            .pragma("foreign_keys", "ON");

    init_pool(opts).await
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
        let pool = open_in_memory().await.expect("pool opens");

        let vtabs: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'memory_embeddings'",
        )
        .fetch_all(&pool)
        .await
        .expect("query succeeds");

        assert!(
            !vtabs.is_empty(),
            "memory_embeddings virtual table not found"
        );
    }
}
