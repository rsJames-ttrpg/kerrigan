# Overseer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build overseer — a persistent memory, job orchestration, decision logging, and artifact storage service exposed via HTTP REST and MCP.

**Architecture:** Single Rust binary, layered monolith. SQLite with sqlite-vec for vector search. axum for HTTP. rmcp for MCP. Both transports share a common service layer with no business logic in transport adapters.

**Tech Stack:** Rust (edition 2024), axum, sqlx (SQLite), sqlite-vec, rmcp, tokio, serde, uuid, tracing

**Spec:** `docs/specs/2026-03-29-overseer-design.md`

**Note on sqlx:** All database code uses `sqlx::SqlitePool` (async, pooled). The pattern across all tasks:
- DB functions are `async fn` taking `&SqlitePool`
- Queries use `sqlx::query()`/`sqlx::query_as()` with `.fetch_one()`/`.fetch_all()`/`.execute()`
- No `Mutex` — sqlx handles connection pooling
- Services hold `SqlitePool` (it's `Clone + Send + Sync`)
- Tests use `#[tokio::test]` and `db::open_in_memory().await`
- For sqlite-vec vector queries, raw SQL with `sqlx::query()` and bind params as `&[u8]` (from `zerocopy::IntoBytes` on `&[f32]`)

---

## File Structure

```
src/overseer/src/
  main.rs                 -- Entrypoint: config loading, DB init, start HTTP + MCP
  config.rs               -- Config struct, TOML parsing
  error.rs                -- OverseerError enum, HTTP/MCP conversions
  db/
    mod.rs                -- SqlitePool init, run migrations, sqlite-vec loading
    schema.sql            -- All CREATE TABLE / CREATE VIRTUAL TABLE statements
    memory.rs             -- Memory + memory_links CRUD, vector search queries
    jobs.rs               -- Job definitions, runs, tasks CRUD
    decisions.rs          -- Decision insert + query
    artifacts.rs          -- Artifact metadata CRUD
  services/
    mod.rs                -- AppState struct holding all services
    memory.rs             -- MemoryService (orchestrates embedding + db)
    jobs.rs               -- JobService
    decisions.rs          -- DecisionService
    artifacts.rs          -- ArtifactService (metadata + filesystem blobs)
  embedding/
    mod.rs                -- EmbeddingProvider trait definition
    stub.rs               -- StubEmbedding: returns zero vectors
  api/
    mod.rs                -- axum Router construction
    memory.rs             -- POST/GET/DELETE /api/memories
    jobs.rs               -- CRUD for definitions, runs, tasks
    decisions.rs          -- POST/GET /api/decisions
    artifacts.rs          -- POST/GET /api/artifacts
  mcp/
    mod.rs                -- MCP server struct, tool_router, ServerHandler impl
```

---

### Task 1: Project scaffolding — deps, config, errors

**Files:**
- Modify: `src/overseer/Cargo.toml`
- Modify: `Cargo.toml` (workspace)
- Create: `src/overseer/src/config.rs`
- Create: `src/overseer/src/error.rs`
- Modify: `src/overseer/src/main.rs`

- [ ] **Step 1: Add dependencies to `src/overseer/Cargo.toml`**

```toml
[package]
name = "overseer"
version = "0.1.0"
edition = "2024"

[dependencies]
axum = "0.8"
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "uuid"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sqlite-vec = "0.0.1-alpha.7"
rmcp = { version = "0.16", features = ["server", "macros", "transport-io", "transport-streamable-http-server"] }
schemars = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "io-std", "signal"] }
tokio-util = "0.7"
toml = "0.8"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
uuid = { version = "1", features = ["v4", "serde"] }
zerocopy = { version = "0.8", features = ["derive"] }
anyhow = "1"
base64 = "0.22"
thiserror = "2"
```

- [ ] **Step 2: Run reindeer to generate third-party BUCK**

```bash
buck2 run root//tools:reindeer -- buckify
```

Fix any fixups needed for crates with build scripts. Expected: several new fixups for rusqlite (bundled sqlite), proc-macro crates, etc. Create `third-party/fixups/<crate>/fixups.toml` with `[buildscript]\nrun = true` and `cargo_env = true` as needed until buckify succeeds cleanly.

- [ ] **Step 3: Update overseer BUCK deps**

Modify `src/overseer/BUCK` to add all third-party deps:

```python
rust_binary(
    name = "overseer",
    srcs = glob(["src/**/*.rs"]),
    crate_root = "src/main.rs",
    deps = [
        "//third-party:axum",
        "//third-party:sqlx",
        "//third-party:serde",
        "//third-party:serde_json",
        "//third-party:sqlite-vec",
        "//third-party:rmcp",
        "//third-party:schemars",
        "//third-party:tokio",
        "//third-party:tokio-util",
        "//third-party:toml",
        "//third-party:tracing",
        "//third-party:tracing-subscriber",
        "//third-party:uuid",
        "//third-party:zerocopy",
        "//third-party:anyhow",
        "//third-party:base64",
        "//third-party:thiserror",
    ],
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 4: Write `src/overseer/src/error.rs`**

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum OverseerError {
    #[error("storage error: {0}")]
    Storage(#[from] rusqlite::Error),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("embedding error: {0}")]
    Embedding(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Internal(String),
}

impl IntoResponse for OverseerError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            OverseerError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            OverseerError::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            OverseerError::Storage(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            OverseerError::Embedding(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            OverseerError::Io(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            OverseerError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
        };
        let body = axum::Json(json!({ "error": message }));
        (status, body).into_response()
    }
}

pub type Result<T> = std::result::Result<T, OverseerError>;
```

Note: add `thiserror = "2"` to Cargo.toml dependencies.

- [ ] **Step 5: Write `src/overseer/src/config.rs`**

```rust
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default = "default_mcp_transport")]
    pub mcp_transport: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            http_port: default_http_port(),
            mcp_transport: default_mcp_transport(),
        }
    }
}

fn default_http_port() -> u16 { 3100 }
fn default_mcp_transport() -> String { "stdio".to_string() }

#[derive(Debug, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_db_path")]
    pub database_path: PathBuf,
    #[serde(default = "default_artifact_path")]
    pub artifact_path: PathBuf,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            database_path: default_db_path(),
            artifact_path: default_artifact_path(),
        }
    }
}

fn default_db_path() -> PathBuf { PathBuf::from("data/overseer.db") }
fn default_artifact_path() -> PathBuf { PathBuf::from("data/artifacts") }

#[derive(Debug, Deserialize)]
pub struct EmbeddingConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self { provider: default_provider() }
    }
}

fn default_provider() -> String { "stub".to_string() }

#[derive(Debug, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self { level: default_log_level() }
    }
}

fn default_log_level() -> String { "info".to_string() }

impl Config {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(toml::from_str("")?)
        }
    }
}
```

- [ ] **Step 6: Update `src/overseer/src/main.rs` to wire config and logging**

```rust
mod config;
mod error;

use config::Config;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("overseer.toml"));

    let config = Config::load(&config_path)?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.logging.level)),
        )
        .init();

    tracing::info!("overseer starting");
    tracing::info!("config loaded from {:?}", config_path);

    // TODO: DB init, services, HTTP + MCP servers will be wired in subsequent tasks

    Ok(())
}
```

- [ ] **Step 7: Verify it compiles**

```bash
cd src/overseer && cargo check
```

Expected: compiles with no errors (warnings about unused modules are fine).

- [ ] **Step 8: Commit**

```bash
git add src/overseer/ third-party/ Cargo.toml Cargo.lock
git commit -m "feat(overseer): project scaffolding — config, errors, dependencies"
```

---

### Task 2: Database layer — schema and connection

**Files:**
- Create: `src/overseer/src/db/mod.rs`
- Create: `src/overseer/src/db/schema.sql`
- Modify: `src/overseer/src/main.rs`

- [ ] **Step 1: Create `src/overseer/src/db/schema.sql`**

```sql
CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    source TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',
    expires_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE VIRTUAL TABLE IF NOT EXISTS memory_embeddings USING vec0(
    embedding float[384]
);

CREATE TABLE IF NOT EXISTS memory_links (
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    linked_id TEXT NOT NULL,
    linked_type TEXT NOT NULL CHECK (linked_type IN ('memory', 'decision')),
    relation_type TEXT NOT NULL,
    PRIMARY KEY (memory_id, linked_id)
);

CREATE TABLE IF NOT EXISTS job_definitions (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    config TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS job_runs (
    id TEXT PRIMARY KEY,
    definition_id TEXT NOT NULL REFERENCES job_definitions(id),
    parent_id TEXT REFERENCES job_runs(id),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    triggered_by TEXT NOT NULL,
    result TEXT,
    error TEXT,
    started_at TEXT,
    completed_at TEXT
);

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    run_id TEXT REFERENCES job_runs(id),
    subject TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'in_progress', 'completed', 'failed')),
    assigned_to TEXT,
    output TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS decisions (
    id TEXT PRIMARY KEY,
    agent TEXT NOT NULL,
    context TEXT NOT NULL,
    decision TEXT NOT NULL,
    reasoning TEXT NOT NULL DEFAULT '',
    tags TEXT NOT NULL DEFAULT '[]',
    run_id TEXT REFERENCES job_runs(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size INTEGER NOT NULL,
    run_id TEXT REFERENCES job_runs(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

- [ ] **Step 2: Create `src/overseer/src/db/mod.rs`**

```rust
pub mod memory;
pub mod jobs;
pub mod decisions;
pub mod artifacts;

use sqlite_vec::sqlite3_vec_init;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;

pub async fn open(path: &Path) -> anyhow::Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Load sqlite-vec extension before any connections are made
    unsafe {
        libsqlite3_sys::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite3_vec_init as *const (),
        )));
    }

    let options = SqliteConnectOptions::from_str(&format!("sqlite:{}?mode=rwc", path.display()))?
        .pragma("journal_mode", "WAL")
        .pragma("foreign_keys", "ON");

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    // Run schema
    let schema = include_str!("schema.sql");
    sqlx::raw_sql(schema).execute(&pool).await?;

    Ok(pool)
}

pub async fn open_in_memory() -> anyhow::Result<SqlitePool> {
    unsafe {
        libsqlite3_sys::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite3_vec_init as *const (),
        )));
    }

    let options = SqliteConnectOptions::from_str("sqlite::memory:")?
        .pragma("foreign_keys", "ON");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;

    let schema = include_str!("schema.sql");
    sqlx::raw_sql(schema).execute(&pool).await?;

    Ok(pool)
}
```

Note: add `libsqlite3-sys` to Cargo.toml (sqlx bundles it, but we need it for `sqlite3_auto_extension`).
```

- [ ] **Step 3: Add db module to main.rs and test DB opens**

Add `mod db;` to `main.rs`. After config loading, add:

```rust
mod db;

// ... in main():
let pool = db::open(&config.storage.database_path).await?;
tracing::info!("database opened at {:?}", config.storage.database_path);
drop(pool); // placeholder — will be passed to services later
```

- [ ] **Step 4: Write a test that verifies schema creation**

Add to `src/overseer/src/db/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_database_opens_and_creates_schema() {
        let pool = open_in_memory().await.unwrap();

        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        let names: Vec<&str> = tables.iter().map(|t| t.0.as_str()).collect();
        assert!(names.contains(&"memories"));
        assert!(names.contains(&"job_definitions"));
        assert!(names.contains(&"job_runs"));
        assert!(names.contains(&"tasks"));
        assert!(names.contains(&"decisions"));
        assert!(names.contains(&"artifacts"));
        assert!(names.contains(&"memory_links"));
    }

    #[tokio::test]
    async fn test_vec0_extension_loaded() {
        let pool = open_in_memory().await.unwrap();

        let (type_name,): (String,) = sqlx::query_as(
            "SELECT type FROM sqlite_master WHERE name='memory_embeddings'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(type_name, "table");
    }
}
```

- [ ] **Step 5: Run tests**

```bash
cd src/overseer && cargo test db::tests
```

Expected: 2 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/overseer/src/db/
git commit -m "feat(overseer): database layer — schema, sqlite-vec, connection"
```

---

### Task 3: Embedding trait and stub provider

**Files:**
- Create: `src/overseer/src/embedding/mod.rs`
- Create: `src/overseer/src/embedding/stub.rs`

- [ ] **Step 1: Create `src/overseer/src/embedding/mod.rs`**

```rust
pub mod stub;

use crate::error::Result;

pub trait EmbeddingProvider: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn model_name(&self) -> &str;
    fn dimensions(&self) -> usize;
}
```

- [ ] **Step 2: Create `src/overseer/src/embedding/stub.rs`**

```rust
use super::EmbeddingProvider;
use crate::error::Result;

pub struct StubEmbedding {
    dims: usize,
}

impl StubEmbedding {
    pub fn new(dims: usize) -> Self {
        Self { dims }
    }
}

impl EmbeddingProvider for StubEmbedding {
    fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.0; self.dims])
    }

    fn model_name(&self) -> &str {
        "stub"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stub_returns_zero_vector() {
        let stub = StubEmbedding::new(384);
        let embedding = stub.embed("anything").unwrap();
        assert_eq!(embedding.len(), 384);
        assert!(embedding.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn test_stub_model_name() {
        let stub = StubEmbedding::new(384);
        assert_eq!(stub.model_name(), "stub");
        assert_eq!(stub.dimensions(), 384);
    }
}
```

- [ ] **Step 3: Add module to main.rs and run tests**

Add `mod embedding;` to `main.rs`.

```bash
cd src/overseer && cargo test embedding
```

Expected: 2 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/embedding/
git commit -m "feat(overseer): embedding trait and stub provider"
```

---

### Task 4: Memory storage and service

**Files:**
- Create: `src/overseer/src/db/memory.rs`
- Create: `src/overseer/src/services/mod.rs`
- Create: `src/overseer/src/services/memory.rs`

- [ ] **Step 1: Create `src/overseer/src/db/memory.rs`**

```rust
use crate::error::{OverseerError, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zerocopy::IntoBytes;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Memory {
    pub id: String,
    pub content: String,
    pub embedding_model: String,
    pub source: String,
    pub tags: Vec<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryLink {
    pub memory_id: String,
    pub linked_id: String,
    pub linked_type: String,
    pub relation_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemorySearchResult {
    pub memory: Memory,
    pub distance: f64,
}

pub fn insert_memory(
    conn: &Connection,
    content: &str,
    embedding: &[f32],
    embedding_model: &str,
    source: &str,
    tags: &[String],
    expires_at: Option<&str>,
) -> Result<Memory> {
    let id = Uuid::new_v4().to_string();
    let tags_json = serde_json::to_string(tags).map_err(|e| OverseerError::Internal(e.to_string()))?;

    conn.execute(
        "INSERT INTO memories (id, content, embedding_model, source, tags, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, content, embedding_model, source, tags_json, expires_at],
    )?;

    // Insert into vector table — rowid must match, so we use the numeric hash approach
    // sqlite-vec uses rowid as integer, so we store a mapping
    let rowid: i64 = conn.query_row(
        "SELECT rowid FROM memories WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )?;

    conn.execute(
        "INSERT INTO memory_embeddings (rowid, embedding) VALUES (?1, ?2)",
        params![rowid, embedding.as_bytes()],
    )?;

    get_memory(conn, &id)?.ok_or_else(|| OverseerError::Internal("inserted memory not found".into()))
}

pub fn get_memory(conn: &Connection, id: &str) -> Result<Option<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, embedding_model, source, tags, expires_at, created_at, updated_at FROM memories WHERE id = ?1",
    )?;

    let result = stmt.query_row(params![id], |row| {
        let tags_str: String = row.get(4)?;
        Ok(Memory {
            id: row.get(0)?,
            content: row.get(1)?,
            embedding_model: row.get(2)?,
            source: row.get(3)?,
            tags: serde_json::from_str(&tags_str).unwrap_or_default(),
            expires_at: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    });

    match result {
        Ok(m) => Ok(Some(m)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn delete_memory(conn: &Connection, id: &str) -> Result<bool> {
    let rowid: std::result::Result<i64, _> = conn.query_row(
        "SELECT rowid FROM memories WHERE id = ?1",
        params![id],
        |row| row.get(0),
    );

    if let Ok(rowid) = rowid {
        conn.execute("DELETE FROM memory_embeddings WHERE rowid = ?1", params![rowid])?;
    }

    let deleted = conn.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
    Ok(deleted > 0)
}

pub fn search_memories(
    conn: &Connection,
    query_embedding: &[f32],
    tags: Option<&[String]>,
    limit: usize,
) -> Result<Vec<MemorySearchResult>> {
    let mut results = Vec::new();

    let mut stmt = conn.prepare(
        "SELECT m.id, m.content, m.embedding_model, m.source, m.tags, m.expires_at, m.created_at, m.updated_at, v.distance
         FROM memory_embeddings v
         JOIN memories m ON m.rowid = v.rowid
         WHERE v.embedding MATCH ?1
         ORDER BY v.distance
         LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![query_embedding.as_bytes(), limit as i64], |row| {
        let tags_str: String = row.get(4)?;
        Ok(MemorySearchResult {
            memory: Memory {
                id: row.get(0)?,
                content: row.get(1)?,
                embedding_model: row.get(2)?,
                source: row.get(3)?,
                tags: serde_json::from_str(&tags_str).unwrap_or_default(),
                expires_at: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            },
            distance: row.get(8)?,
        })
    })?;

    for row in rows {
        let result = row?;
        if let Some(filter_tags) = tags {
            if filter_tags.iter().any(|t| result.memory.tags.contains(t)) {
                results.push(result);
            }
        } else {
            results.push(result);
        }
    }

    Ok(results)
}

pub fn insert_memory_link(
    conn: &Connection,
    memory_id: &str,
    linked_id: &str,
    linked_type: &str,
    relation_type: &str,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO memory_links (memory_id, linked_id, linked_type, relation_type) VALUES (?1, ?2, ?3, ?4)",
        params![memory_id, linked_id, linked_type, relation_type],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_insert_and_get_memory() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn.lock().unwrap();

        let embedding = vec![0.1_f32; 384];
        let tags = vec!["test".to_string()];
        let mem = insert_memory(&conn, "hello world", &embedding, "stub", "test-agent", &tags, None).unwrap();

        assert_eq!(mem.content, "hello world");
        assert_eq!(mem.source, "test-agent");
        assert_eq!(mem.tags, vec!["test"]);

        let fetched = get_memory(&conn, &mem.id).unwrap().unwrap();
        assert_eq!(fetched.id, mem.id);
    }

    #[test]
    fn test_delete_memory() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn.lock().unwrap();

        let embedding = vec![0.1_f32; 384];
        let mem = insert_memory(&conn, "to delete", &embedding, "stub", "agent", &[], None).unwrap();

        assert!(delete_memory(&conn, &mem.id).unwrap());
        assert!(get_memory(&conn, &mem.id).unwrap().is_none());
    }

    #[test]
    fn test_search_memories() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn.lock().unwrap();

        insert_memory(&conn, "first", &vec![0.1_f32; 384], "stub", "agent", &[], None).unwrap();
        insert_memory(&conn, "second", &vec![0.2_f32; 384], "stub", "agent", &[], None).unwrap();
        insert_memory(&conn, "third", &vec![0.3_f32; 384], "stub", "agent", &[], None).unwrap();

        let query = vec![0.3_f32; 384];
        let results = search_memories(&conn, &query, None, 2).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].memory.content, "third"); // closest match
    }

    #[test]
    fn test_memory_link() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn.lock().unwrap();

        let embedding = vec![0.1_f32; 384];
        let m1 = insert_memory(&conn, "mem1", &embedding, "stub", "agent", &[], None).unwrap();
        let m2 = insert_memory(&conn, "mem2", &embedding, "stub", "agent", &[], None).unwrap();

        insert_memory_link(&conn, &m1.id, &m2.id, "memory", "related").unwrap();

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_links WHERE memory_id = ?1",
            params![m1.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }
}
```

- [ ] **Step 2: Create `src/overseer/src/services/mod.rs`**

```rust
pub mod memory;
pub mod jobs;
pub mod decisions;
pub mod artifacts;

use crate::db::Database;
use crate::embedding::EmbeddingProvider;
use std::sync::Arc;

pub struct AppState {
    pub memory: memory::MemoryService,
    pub jobs: jobs::JobService,
    pub decisions: decisions::DecisionService,
    pub artifacts: artifacts::ArtifactService,
}

impl AppState {
    pub fn new(db: Arc<Database>, embedder: Arc<dyn EmbeddingProvider>, artifact_path: std::path::PathBuf) -> Self {
        Self {
            memory: memory::MemoryService::new(db.clone(), embedder),
            jobs: jobs::JobService::new(db.clone()),
            decisions: decisions::DecisionService::new(db.clone()),
            artifacts: artifacts::ArtifactService::new(db, artifact_path),
        }
    }
}
```

- [ ] **Step 3: Create `src/overseer/src/services/memory.rs`**

```rust
use crate::db::Database;
use crate::db::memory::{self, Memory, MemorySearchResult};
use crate::embedding::EmbeddingProvider;
use crate::error::Result;
use std::sync::Arc;

pub struct MemoryService {
    db: Arc<Database>,
    embedder: Arc<dyn EmbeddingProvider>,
}

impl MemoryService {
    pub fn new(db: Arc<Database>, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self { db, embedder }
    }

    pub fn store(
        &self,
        content: &str,
        source: &str,
        tags: &[String],
        links: &[(String, String, String)], // (linked_id, linked_type, relation_type)
        expires_at: Option<&str>,
    ) -> Result<Memory> {
        let embedding = self.embedder.embed(content)?;
        let conn = self.db.conn.lock().map_err(|e| crate::error::OverseerError::Internal(e.to_string()))?;

        let mem = memory::insert_memory(
            &conn,
            content,
            &embedding,
            self.embedder.model_name(),
            source,
            tags,
            expires_at,
        )?;

        for (linked_id, linked_type, relation_type) in links {
            memory::insert_memory_link(&conn, &mem.id, linked_id, linked_type, relation_type)?;
        }

        Ok(mem)
    }

    pub fn recall(
        &self,
        query: &str,
        tags: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>> {
        let embedding = self.embedder.embed(query)?;
        let conn = self.db.conn.lock().map_err(|e| crate::error::OverseerError::Internal(e.to_string()))?;
        memory::search_memories(&conn, &embedding, tags, limit)
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let conn = self.db.conn.lock().map_err(|e| crate::error::OverseerError::Internal(e.to_string()))?;
        memory::delete_memory(&conn, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::stub::StubEmbedding;

    #[test]
    fn test_memory_service_store_and_recall() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let embedder = Arc::new(StubEmbedding::new(384));
        let svc = MemoryService::new(db, embedder);

        let mem = svc.store("test memory", "agent", &["tag1".into()], &[], None).unwrap();
        assert_eq!(mem.content, "test memory");

        // Stub returns zero vectors, so all memories are equidistant — just verify it returns results
        let results = svc.recall("anything", None, 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_memory_service_delete() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let embedder = Arc::new(StubEmbedding::new(384));
        let svc = MemoryService::new(db, embedder);

        let mem = svc.store("to delete", "agent", &[], &[], None).unwrap();
        assert!(svc.delete(&mem.id).unwrap());

        let results = svc.recall("to delete", None, 10).unwrap();
        assert_eq!(results.len(), 0);
    }
}
```

- [ ] **Step 4: Create stub files for other services**

Create `src/overseer/src/services/jobs.rs`:

```rust
use crate::db::Database;
use std::sync::Arc;

pub struct JobService {
    db: Arc<Database>,
}

impl JobService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}
```

Create `src/overseer/src/services/decisions.rs`:

```rust
use crate::db::Database;
use std::sync::Arc;

pub struct DecisionService {
    db: Arc<Database>,
}

impl DecisionService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}
```

Create `src/overseer/src/services/artifacts.rs`:

```rust
use crate::db::Database;
use std::path::PathBuf;
use std::sync::Arc;

pub struct ArtifactService {
    db: Arc<Database>,
    artifact_path: PathBuf,
}

impl ArtifactService {
    pub fn new(db: Arc<Database>, artifact_path: PathBuf) -> Self {
        Self { db, artifact_path }
    }
}
```

- [ ] **Step 5: Add modules to main.rs and run tests**

Add `mod services;` to `main.rs`.

```bash
cd src/overseer && cargo test
```

Expected: all tests pass (db tests + embedding tests + memory service tests).

- [ ] **Step 6: Commit**

```bash
git add src/overseer/src/
git commit -m "feat(overseer): memory storage, service, and vector search"
```

---

### Task 5: Jobs and decisions storage + services

**Files:**
- Create: `src/overseer/src/db/jobs.rs`
- Create: `src/overseer/src/db/decisions.rs`
- Create: `src/overseer/src/db/artifacts.rs`
- Modify: `src/overseer/src/services/jobs.rs`
- Modify: `src/overseer/src/services/decisions.rs`
- Modify: `src/overseer/src/services/artifacts.rs`

- [ ] **Step 1: Create `src/overseer/src/db/jobs.rs`**

```rust
use crate::error::{OverseerError, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JobDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub config: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
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

#[derive(Debug, Serialize, Deserialize, Clone)]
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

pub fn create_job_definition(conn: &Connection, name: &str, description: &str, config: &serde_json::Value) -> Result<JobDefinition> {
    let id = Uuid::new_v4().to_string();
    let config_str = serde_json::to_string(config).map_err(|e| OverseerError::Internal(e.to_string()))?;
    conn.execute(
        "INSERT INTO job_definitions (id, name, description, config) VALUES (?1, ?2, ?3, ?4)",
        params![id, name, description, config_str],
    )?;
    get_job_definition(conn, &id)?.ok_or_else(|| OverseerError::Internal("inserted definition not found".into()))
}

pub fn get_job_definition(conn: &Connection, id: &str) -> Result<Option<JobDefinition>> {
    let mut stmt = conn.prepare("SELECT id, name, description, config, created_at, updated_at FROM job_definitions WHERE id = ?1")?;
    let result = stmt.query_row(params![id], |row| {
        let config_str: String = row.get(3)?;
        Ok(JobDefinition {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            config: serde_json::from_str(&config_str).unwrap_or_default(),
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    });
    match result {
        Ok(d) => Ok(Some(d)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn list_job_definitions(conn: &Connection) -> Result<Vec<JobDefinition>> {
    let mut stmt = conn.prepare("SELECT id, name, description, config, created_at, updated_at FROM job_definitions ORDER BY created_at DESC")?;
    let rows = stmt.query_map([], |row| {
        let config_str: String = row.get(3)?;
        Ok(JobDefinition {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            config: serde_json::from_str(&config_str).unwrap_or_default(),
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub fn start_job_run(conn: &Connection, definition_id: &str, triggered_by: &str, parent_id: Option<&str>) -> Result<JobRun> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO job_runs (id, definition_id, parent_id, triggered_by, started_at) VALUES (?1, ?2, ?3, ?4, datetime('now'))",
        params![id, definition_id, parent_id, triggered_by],
    )?;
    conn.execute("UPDATE job_runs SET status = 'running' WHERE id = ?1", params![id])?;
    get_job_run(conn, &id)?.ok_or_else(|| OverseerError::Internal("inserted run not found".into()))
}

pub fn get_job_run(conn: &Connection, id: &str) -> Result<Option<JobRun>> {
    let mut stmt = conn.prepare("SELECT id, definition_id, parent_id, status, triggered_by, result, error, started_at, completed_at FROM job_runs WHERE id = ?1")?;
    let result = stmt.query_row(params![id], |row| {
        let result_str: Option<String> = row.get(5)?;
        Ok(JobRun {
            id: row.get(0)?,
            definition_id: row.get(1)?,
            parent_id: row.get(2)?,
            status: row.get(3)?,
            triggered_by: row.get(4)?,
            result: result_str.and_then(|s| serde_json::from_str(&s).ok()),
            error: row.get(6)?,
            started_at: row.get(7)?,
            completed_at: row.get(8)?,
        })
    });
    match result {
        Ok(r) => Ok(Some(r)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn update_job_run(conn: &Connection, id: &str, status: Option<&str>, result: Option<&serde_json::Value>, error: Option<&str>) -> Result<JobRun> {
    if let Some(s) = status {
        conn.execute("UPDATE job_runs SET status = ?1 WHERE id = ?2", params![s, id])?;
        if s == "completed" || s == "failed" || s == "cancelled" {
            conn.execute("UPDATE job_runs SET completed_at = datetime('now') WHERE id = ?1", params![id])?;
        }
    }
    if let Some(r) = result {
        let json = serde_json::to_string(r).map_err(|e| OverseerError::Internal(e.to_string()))?;
        conn.execute("UPDATE job_runs SET result = ?1 WHERE id = ?2", params![json, id])?;
    }
    if let Some(e) = error {
        conn.execute("UPDATE job_runs SET error = ?1 WHERE id = ?2", params![e, id])?;
    }
    get_job_run(conn, id)?.ok_or_else(|| OverseerError::NotFound(format!("job run {id} not found")))
}

pub fn list_job_runs(conn: &Connection, status: Option<&str>) -> Result<Vec<JobRun>> {
    let sql = if status.is_some() {
        "SELECT id, definition_id, parent_id, status, triggered_by, result, error, started_at, completed_at FROM job_runs WHERE status = ?1 ORDER BY started_at DESC"
    } else {
        "SELECT id, definition_id, parent_id, status, triggered_by, result, error, started_at, completed_at FROM job_runs ORDER BY started_at DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = if let Some(s) = status {
        stmt.query_map(params![s], |row| {
            let result_str: Option<String> = row.get(5)?;
            Ok(JobRun {
                id: row.get(0)?, definition_id: row.get(1)?, parent_id: row.get(2)?,
                status: row.get(3)?, triggered_by: row.get(4)?,
                result: result_str.and_then(|s| serde_json::from_str(&s).ok()),
                error: row.get(6)?, started_at: row.get(7)?, completed_at: row.get(8)?,
            })
        })?
    } else {
        stmt.query_map([], |row| {
            let result_str: Option<String> = row.get(5)?;
            Ok(JobRun {
                id: row.get(0)?, definition_id: row.get(1)?, parent_id: row.get(2)?,
                status: row.get(3)?, triggered_by: row.get(4)?,
                result: result_str.and_then(|s| serde_json::from_str(&s).ok()),
                error: row.get(6)?, started_at: row.get(7)?, completed_at: row.get(8)?,
            })
        })?
    };
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub fn create_task(conn: &Connection, subject: &str, run_id: Option<&str>, assigned_to: Option<&str>) -> Result<Task> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO tasks (id, run_id, subject, assigned_to) VALUES (?1, ?2, ?3, ?4)",
        params![id, run_id, subject, assigned_to],
    )?;
    get_task(conn, &id)?.ok_or_else(|| OverseerError::Internal("inserted task not found".into()))
}

pub fn get_task(conn: &Connection, id: &str) -> Result<Option<Task>> {
    let mut stmt = conn.prepare("SELECT id, run_id, subject, status, assigned_to, output, created_at, updated_at FROM tasks WHERE id = ?1")?;
    let result = stmt.query_row(params![id], |row| {
        let output_str: Option<String> = row.get(5)?;
        Ok(Task {
            id: row.get(0)?, run_id: row.get(1)?, subject: row.get(2)?,
            status: row.get(3)?, assigned_to: row.get(4)?,
            output: output_str.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: row.get(6)?, updated_at: row.get(7)?,
        })
    });
    match result {
        Ok(t) => Ok(Some(t)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn update_task(conn: &Connection, id: &str, status: Option<&str>, assigned_to: Option<&str>, output: Option<&serde_json::Value>) -> Result<Task> {
    if let Some(s) = status {
        conn.execute("UPDATE tasks SET status = ?1, updated_at = datetime('now') WHERE id = ?2", params![s, id])?;
    }
    if let Some(a) = assigned_to {
        conn.execute("UPDATE tasks SET assigned_to = ?1, updated_at = datetime('now') WHERE id = ?2", params![a, id])?;
    }
    if let Some(o) = output {
        let json = serde_json::to_string(o).map_err(|e| OverseerError::Internal(e.to_string()))?;
        conn.execute("UPDATE tasks SET output = ?1, updated_at = datetime('now') WHERE id = ?2", params![json, id])?;
    }
    get_task(conn, id)?.ok_or_else(|| OverseerError::NotFound(format!("task {id} not found")))
}

pub fn list_tasks(conn: &Connection, status: Option<&str>, assigned_to: Option<&str>, run_id: Option<&str>) -> Result<Vec<Task>> {
    let mut conditions = vec!["1=1".to_string()];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];

    if let Some(s) = status {
        conditions.push(format!("status = ?{}", param_values.len() + 1));
        param_values.push(Box::new(s.to_string()));
    }
    if let Some(a) = assigned_to {
        conditions.push(format!("assigned_to = ?{}", param_values.len() + 1));
        param_values.push(Box::new(a.to_string()));
    }
    if let Some(r) = run_id {
        conditions.push(format!("run_id = ?{}", param_values.len() + 1));
        param_values.push(Box::new(r.to_string()));
    }

    let sql = format!(
        "SELECT id, run_id, subject, status, assigned_to, output, created_at, updated_at FROM tasks WHERE {} ORDER BY created_at DESC",
        conditions.join(" AND ")
    );

    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(params.as_slice(), |row| {
        let output_str: Option<String> = row.get(5)?;
        Ok(Task {
            id: row.get(0)?, run_id: row.get(1)?, subject: row.get(2)?,
            status: row.get(3)?, assigned_to: row.get(4)?,
            output: output_str.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: row.get(6)?, updated_at: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_job_definition_crud() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn.lock().unwrap();

        let def = create_job_definition(&conn, "review-pr", "Review a pull request", &serde_json::json!({})).unwrap();
        assert_eq!(def.name, "review-pr");

        let fetched = get_job_definition(&conn, &def.id).unwrap().unwrap();
        assert_eq!(fetched.name, "review-pr");

        let all = list_job_definitions(&conn).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_job_run_lifecycle() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn.lock().unwrap();

        let def = create_job_definition(&conn, "test-job", "", &serde_json::json!({})).unwrap();
        let run = start_job_run(&conn, &def.id, "test-agent", None).unwrap();
        assert_eq!(run.status, "running");

        let updated = update_job_run(&conn, &run.id, Some("completed"), Some(&serde_json::json!({"ok": true})), None).unwrap();
        assert_eq!(updated.status, "completed");
        assert!(updated.completed_at.is_some());
    }

    #[test]
    fn test_task_crud() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn.lock().unwrap();

        let task = create_task(&conn, "do something", None, None).unwrap();
        assert_eq!(task.status, "pending");

        let updated = update_task(&conn, &task.id, Some("in_progress"), Some("worker-1"), None).unwrap();
        assert_eq!(updated.status, "in_progress");
        assert_eq!(updated.assigned_to.unwrap(), "worker-1");

        let active = list_tasks(&conn, Some("in_progress"), None, None).unwrap();
        assert_eq!(active.len(), 1);
    }

    #[test]
    fn test_sub_job_runs() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn.lock().unwrap();

        let def = create_job_definition(&conn, "deploy", "", &serde_json::json!({})).unwrap();
        let parent = start_job_run(&conn, &def.id, "agent", None).unwrap();
        let child = start_job_run(&conn, &def.id, "agent", Some(&parent.id)).unwrap();
        assert_eq!(child.parent_id.unwrap(), parent.id);
    }
}
```

- [ ] **Step 2: Create `src/overseer/src/db/decisions.rs`**

```rust
use crate::error::{OverseerError, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Decision {
    pub id: String,
    pub agent: String,
    pub context: String,
    pub decision: String,
    pub reasoning: String,
    pub tags: Vec<String>,
    pub run_id: Option<String>,
    pub created_at: String,
}

pub fn log_decision(
    conn: &Connection,
    agent: &str,
    context: &str,
    decision: &str,
    reasoning: &str,
    tags: &[String],
    run_id: Option<&str>,
) -> Result<Decision> {
    let id = Uuid::new_v4().to_string();
    let tags_json = serde_json::to_string(tags).map_err(|e| OverseerError::Internal(e.to_string()))?;
    conn.execute(
        "INSERT INTO decisions (id, agent, context, decision, reasoning, tags, run_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, agent, context, decision, reasoning, tags_json, run_id],
    )?;
    get_decision(conn, &id)?.ok_or_else(|| OverseerError::Internal("inserted decision not found".into()))
}

pub fn get_decision(conn: &Connection, id: &str) -> Result<Option<Decision>> {
    let mut stmt = conn.prepare("SELECT id, agent, context, decision, reasoning, tags, run_id, created_at FROM decisions WHERE id = ?1")?;
    let result = stmt.query_row(params![id], |row| {
        let tags_str: String = row.get(5)?;
        Ok(Decision {
            id: row.get(0)?, agent: row.get(1)?, context: row.get(2)?,
            decision: row.get(3)?, reasoning: row.get(4)?,
            tags: serde_json::from_str(&tags_str).unwrap_or_default(),
            run_id: row.get(6)?, created_at: row.get(7)?,
        })
    });
    match result {
        Ok(d) => Ok(Some(d)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn query_decisions(
    conn: &Connection,
    agent: Option<&str>,
    tags: Option<&[String]>,
    limit: usize,
) -> Result<Vec<Decision>> {
    let mut conditions = vec!["1=1".to_string()];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];

    if let Some(a) = agent {
        conditions.push(format!("agent = ?{}", param_values.len() + 1));
        param_values.push(Box::new(a.to_string()));
    }

    let sql = format!(
        "SELECT id, agent, context, decision, reasoning, tags, run_id, created_at FROM decisions WHERE {} ORDER BY created_at DESC LIMIT ?{}",
        conditions.join(" AND "),
        param_values.len() + 1
    );
    param_values.push(Box::new(limit as i64));

    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(params.as_slice(), |row| {
        let tags_str: String = row.get(5)?;
        Ok(Decision {
            id: row.get(0)?, agent: row.get(1)?, context: row.get(2)?,
            decision: row.get(3)?, reasoning: row.get(4)?,
            tags: serde_json::from_str(&tags_str).unwrap_or_default(),
            run_id: row.get(6)?, created_at: row.get(7)?,
        })
    })?;

    let mut results: Vec<Decision> = rows.collect::<std::result::Result<Vec<_>, _>>()?;

    // Post-filter by tags if specified
    if let Some(filter_tags) = tags {
        results.retain(|d| filter_tags.iter().any(|t| d.tags.contains(t)));
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_log_and_query_decision() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn.lock().unwrap();

        let d = log_decision(&conn, "claude", "reviewing PR #42", "approve", "tests pass, clean code", &["code-review".into()], None).unwrap();
        assert_eq!(d.agent, "claude");
        assert_eq!(d.decision, "approve");

        let results = query_decisions(&conn, Some("claude"), None, 10).unwrap();
        assert_eq!(results.len(), 1);

        let by_tag = query_decisions(&conn, None, Some(&["code-review".into()]), 10).unwrap();
        assert_eq!(by_tag.len(), 1);

        let empty = query_decisions(&conn, None, Some(&["nonexistent".into()]), 10).unwrap();
        assert_eq!(empty.len(), 0);
    }
}
```

- [ ] **Step 3: Create `src/overseer/src/db/artifacts.rs`**

```rust
use crate::error::{OverseerError, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ArtifactMetadata {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub run_id: Option<String>,
    pub created_at: String,
}

pub fn insert_artifact(conn: &Connection, name: &str, content_type: &str, size: i64, run_id: Option<&str>) -> Result<ArtifactMetadata> {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO artifacts (id, name, content_type, size, run_id) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, name, content_type, size, run_id],
    )?;
    get_artifact(conn, &id)?.ok_or_else(|| OverseerError::Internal("inserted artifact not found".into()))
}

pub fn get_artifact(conn: &Connection, id: &str) -> Result<Option<ArtifactMetadata>> {
    let mut stmt = conn.prepare("SELECT id, name, content_type, size, run_id, created_at FROM artifacts WHERE id = ?1")?;
    let result = stmt.query_row(params![id], |row| {
        Ok(ArtifactMetadata {
            id: row.get(0)?, name: row.get(1)?, content_type: row.get(2)?,
            size: row.get(3)?, run_id: row.get(4)?, created_at: row.get(5)?,
        })
    });
    match result {
        Ok(a) => Ok(Some(a)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn list_artifacts(conn: &Connection, run_id: Option<&str>) -> Result<Vec<ArtifactMetadata>> {
    let (sql, param): (&str, Vec<&dyn rusqlite::types::ToSql>) = if let Some(r) = run_id {
        ("SELECT id, name, content_type, size, run_id, created_at FROM artifacts WHERE run_id = ?1 ORDER BY created_at DESC", vec![&r as &dyn rusqlite::types::ToSql])
    } else {
        ("SELECT id, name, content_type, size, run_id, created_at FROM artifacts ORDER BY created_at DESC", vec![])
    };

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(param.as_slice(), |row| {
        Ok(ArtifactMetadata {
            id: row.get(0)?, name: row.get(1)?, content_type: row.get(2)?,
            size: row.get(3)?, run_id: row.get(4)?, created_at: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_artifact_metadata_crud() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn.lock().unwrap();

        let art = insert_artifact(&conn, "output.json", "application/json", 1234, None).unwrap();
        assert_eq!(art.name, "output.json");

        let fetched = get_artifact(&conn, &art.id).unwrap().unwrap();
        assert_eq!(fetched.size, 1234);

        let all = list_artifacts(&conn, None).unwrap();
        assert_eq!(all.len(), 1);
    }
}
```

- [ ] **Step 4: Wire up full services**

Update `src/overseer/src/services/jobs.rs`:

```rust
use crate::db::Database;
use crate::db::jobs::{self, JobDefinition, JobRun, Task};
use crate::error::Result;
use std::sync::Arc;

pub struct JobService {
    db: Arc<Database>,
}

impl JobService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn create_definition(&self, name: &str, description: &str, config: &serde_json::Value) -> Result<JobDefinition> {
        let conn = self.db.conn.lock().map_err(|e| crate::error::OverseerError::Internal(e.to_string()))?;
        jobs::create_job_definition(&conn, name, description, config)
    }

    pub fn list_definitions(&self) -> Result<Vec<JobDefinition>> {
        let conn = self.db.conn.lock().map_err(|e| crate::error::OverseerError::Internal(e.to_string()))?;
        jobs::list_job_definitions(&conn)
    }

    pub fn start_run(&self, definition_id: &str, triggered_by: &str, parent_id: Option<&str>) -> Result<JobRun> {
        let conn = self.db.conn.lock().map_err(|e| crate::error::OverseerError::Internal(e.to_string()))?;
        jobs::start_job_run(&conn, definition_id, triggered_by, parent_id)
    }

    pub fn update_run(&self, id: &str, status: Option<&str>, result: Option<&serde_json::Value>, error: Option<&str>) -> Result<JobRun> {
        let conn = self.db.conn.lock().map_err(|e| crate::error::OverseerError::Internal(e.to_string()))?;
        jobs::update_job_run(&conn, id, status, result, error)
    }

    pub fn list_runs(&self, status: Option<&str>) -> Result<Vec<JobRun>> {
        let conn = self.db.conn.lock().map_err(|e| crate::error::OverseerError::Internal(e.to_string()))?;
        jobs::list_job_runs(&conn, status)
    }

    pub fn create_task(&self, subject: &str, run_id: Option<&str>, assigned_to: Option<&str>) -> Result<Task> {
        let conn = self.db.conn.lock().map_err(|e| crate::error::OverseerError::Internal(e.to_string()))?;
        jobs::create_task(&conn, subject, run_id, assigned_to)
    }

    pub fn update_task(&self, id: &str, status: Option<&str>, assigned_to: Option<&str>, output: Option<&serde_json::Value>) -> Result<Task> {
        let conn = self.db.conn.lock().map_err(|e| crate::error::OverseerError::Internal(e.to_string()))?;
        jobs::update_task(&conn, id, status, assigned_to, output)
    }

    pub fn list_tasks(&self, status: Option<&str>, assigned_to: Option<&str>, run_id: Option<&str>) -> Result<Vec<Task>> {
        let conn = self.db.conn.lock().map_err(|e| crate::error::OverseerError::Internal(e.to_string()))?;
        jobs::list_tasks(&conn, status, assigned_to, run_id)
    }
}
```

Update `src/overseer/src/services/decisions.rs`:

```rust
use crate::db::Database;
use crate::db::decisions::{self, Decision};
use crate::error::Result;
use std::sync::Arc;

pub struct DecisionService {
    db: Arc<Database>,
}

impl DecisionService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn log(&self, agent: &str, context: &str, decision: &str, reasoning: &str, tags: &[String], run_id: Option<&str>) -> Result<Decision> {
        let conn = self.db.conn.lock().map_err(|e| crate::error::OverseerError::Internal(e.to_string()))?;
        decisions::log_decision(&conn, agent, context, decision, reasoning, tags, run_id)
    }

    pub fn query(&self, agent: Option<&str>, tags: Option<&[String]>, limit: usize) -> Result<Vec<Decision>> {
        let conn = self.db.conn.lock().map_err(|e| crate::error::OverseerError::Internal(e.to_string()))?;
        decisions::query_decisions(&conn, agent, tags, limit)
    }
}
```

Update `src/overseer/src/services/artifacts.rs`:

```rust
use crate::db::Database;
use crate::db::artifacts::{self, ArtifactMetadata};
use crate::error::{OverseerError, Result};
use std::path::PathBuf;
use std::sync::Arc;

pub struct ArtifactService {
    db: Arc<Database>,
    artifact_path: PathBuf,
}

impl ArtifactService {
    pub fn new(db: Arc<Database>, artifact_path: PathBuf) -> Self {
        Self { db, artifact_path }
    }

    pub fn store(&self, name: &str, content_type: &str, data: &[u8], run_id: Option<&str>) -> Result<ArtifactMetadata> {
        let conn = self.db.conn.lock().map_err(|e| OverseerError::Internal(e.to_string()))?;
        let meta = artifacts::insert_artifact(&conn, name, content_type, data.len() as i64, run_id)?;

        std::fs::create_dir_all(&self.artifact_path)?;
        let path = self.artifact_path.join(&meta.id);
        std::fs::write(&path, data)?;

        Ok(meta)
    }

    pub fn get(&self, id: &str) -> Result<(ArtifactMetadata, Vec<u8>)> {
        let conn = self.db.conn.lock().map_err(|e| OverseerError::Internal(e.to_string()))?;
        let meta = artifacts::get_artifact(&conn, id)?
            .ok_or_else(|| OverseerError::NotFound(format!("artifact {id} not found")))?;

        let path = self.artifact_path.join(id);
        let data = std::fs::read(&path)?;

        Ok((meta, data))
    }

    pub fn list(&self, run_id: Option<&str>) -> Result<Vec<ArtifactMetadata>> {
        let conn = self.db.conn.lock().map_err(|e| OverseerError::Internal(e.to_string()))?;
        artifacts::list_artifacts(&conn, run_id)
    }
}
```

- [ ] **Step 5: Run all tests**

```bash
cd src/overseer && cargo test
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/overseer/src/
git commit -m "feat(overseer): jobs, decisions, artifacts — storage and services"
```

---

### Task 6: HTTP API

**Files:**
- Create: `src/overseer/src/api/mod.rs`
- Create: `src/overseer/src/api/memory.rs`
- Create: `src/overseer/src/api/jobs.rs`
- Create: `src/overseer/src/api/decisions.rs`
- Create: `src/overseer/src/api/artifacts.rs`

- [ ] **Step 1: Create `src/overseer/src/api/mod.rs`**

```rust
mod memory;
mod jobs;
mod decisions;
mod artifacts;

use crate::services::AppState;
use axum::Router;
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .nest("/api/memories", memory::router(state.clone()))
        .nest("/api/decisions", decisions::router(state.clone()))
        .nest("/api/jobs", jobs::router(state.clone()))
        .nest("/api/tasks", jobs::task_router(state.clone()))
        .nest("/api/artifacts", artifacts::router(state))
}
```

- [ ] **Step 2: Create `src/overseer/src/api/memory.rs`**

```rust
use crate::error::OverseerError;
use crate::services::AppState;
use axum::{extract::{Path, Query, State}, routing::{delete, get, post}, Json, Router};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct StoreMemoryRequest {
    pub content: String,
    pub source: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<LinkSpec>,
    pub expires_at: Option<String>,
}

#[derive(Deserialize)]
pub struct LinkSpec {
    pub linked_id: String,
    pub linked_type: String,
    pub relation_type: String,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub tags: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize { 10 }

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", post(store_memory))
        .route("/search", get(search_memories))
        .route("/{id}", delete(delete_memory))
        .with_state(state)
}

async fn store_memory(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StoreMemoryRequest>,
) -> Result<Json<serde_json::Value>, OverseerError> {
    let links: Vec<(String, String, String)> = req.links.into_iter()
        .map(|l| (l.linked_id, l.linked_type, l.relation_type))
        .collect();

    let mem = state.memory.store(
        &req.content, &req.source, &req.tags, &links, req.expires_at.as_deref(),
    )?;
    Ok(Json(serde_json::to_value(mem).unwrap()))
}

async fn search_memories(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, OverseerError> {
    let tags: Option<Vec<String>> = query.tags.map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
    let results = state.memory.recall(&query.q, tags.as_deref(), query.limit)?;
    Ok(Json(serde_json::to_value(results).unwrap()))
}

async fn delete_memory(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, OverseerError> {
    let deleted = state.memory.delete(&id)?;
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}
```

- [ ] **Step 3: Create `src/overseer/src/api/decisions.rs`**

```rust
use crate::error::OverseerError;
use crate::services::AppState;
use axum::{extract::{Query, State}, routing::{get, post}, Json, Router};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct LogDecisionRequest {
    pub agent: String,
    pub context: String,
    pub decision: String,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub run_id: Option<String>,
}

#[derive(Deserialize)]
pub struct DecisionQuery {
    pub agent: Option<String>,
    pub tags: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize { 20 }

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", post(log_decision).get(query_decisions))
        .with_state(state)
}

async fn log_decision(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LogDecisionRequest>,
) -> Result<Json<serde_json::Value>, OverseerError> {
    let d = state.decisions.log(&req.agent, &req.context, &req.decision, &req.reasoning, &req.tags, req.run_id.as_deref())?;
    Ok(Json(serde_json::to_value(d).unwrap()))
}

async fn query_decisions(
    State(state): State<Arc<AppState>>,
    Query(q): Query<DecisionQuery>,
) -> Result<Json<serde_json::Value>, OverseerError> {
    let tags: Option<Vec<String>> = q.tags.map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
    let results = state.decisions.query(q.agent.as_deref(), tags.as_deref(), q.limit)?;
    Ok(Json(serde_json::to_value(results).unwrap()))
}
```

- [ ] **Step 4: Create `src/overseer/src/api/jobs.rs`**

```rust
use crate::error::OverseerError;
use crate::services::AppState;
use axum::{extract::{Path, Query, State}, routing::{get, patch, post}, Json, Router};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct CreateDefinitionRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_config")]
    pub config: serde_json::Value,
}

fn default_config() -> serde_json::Value { serde_json::json!({}) }

#[derive(Deserialize)]
pub struct StartRunRequest {
    pub definition_id: String,
    pub triggered_by: String,
    pub parent_id: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateRunRequest {
    pub status: Option<String>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct RunQuery {
    pub status: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateTaskRequest {
    pub subject: String,
    pub run_id: Option<String>,
    pub assigned_to: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateTaskRequest {
    pub status: Option<String>,
    pub assigned_to: Option<String>,
    pub output: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct TaskQuery {
    pub status: Option<String>,
    pub assigned_to: Option<String>,
    pub run_id: Option<String>,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/definitions", post(create_definition).get(list_definitions))
        .route("/runs", post(start_run).get(list_runs))
        .route("/runs/{id}", patch(update_run))
        .with_state(state)
}

pub fn task_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", post(create_task).get(list_tasks))
        .route("/{id}", patch(update_task))
        .with_state(state)
}

async fn create_definition(State(state): State<Arc<AppState>>, Json(req): Json<CreateDefinitionRequest>) -> Result<Json<serde_json::Value>, OverseerError> {
    let def = state.jobs.create_definition(&req.name, &req.description, &req.config)?;
    Ok(Json(serde_json::to_value(def).unwrap()))
}

async fn list_definitions(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, OverseerError> {
    let defs = state.jobs.list_definitions()?;
    Ok(Json(serde_json::to_value(defs).unwrap()))
}

async fn start_run(State(state): State<Arc<AppState>>, Json(req): Json<StartRunRequest>) -> Result<Json<serde_json::Value>, OverseerError> {
    let run = state.jobs.start_run(&req.definition_id, &req.triggered_by, req.parent_id.as_deref())?;
    Ok(Json(serde_json::to_value(run).unwrap()))
}

async fn update_run(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<UpdateRunRequest>) -> Result<Json<serde_json::Value>, OverseerError> {
    let run = state.jobs.update_run(&id, req.status.as_deref(), req.result.as_ref(), req.error.as_deref())?;
    Ok(Json(serde_json::to_value(run).unwrap()))
}

async fn list_runs(State(state): State<Arc<AppState>>, Query(q): Query<RunQuery>) -> Result<Json<serde_json::Value>, OverseerError> {
    let runs = state.jobs.list_runs(q.status.as_deref())?;
    Ok(Json(serde_json::to_value(runs).unwrap()))
}

async fn create_task(State(state): State<Arc<AppState>>, Json(req): Json<CreateTaskRequest>) -> Result<Json<serde_json::Value>, OverseerError> {
    let task = state.jobs.create_task(&req.subject, req.run_id.as_deref(), req.assigned_to.as_deref())?;
    Ok(Json(serde_json::to_value(task).unwrap()))
}

async fn update_task(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<UpdateTaskRequest>) -> Result<Json<serde_json::Value>, OverseerError> {
    let task = state.jobs.update_task(&id, req.status.as_deref(), req.assigned_to.as_deref(), req.output.as_ref())?;
    Ok(Json(serde_json::to_value(task).unwrap()))
}

async fn list_tasks(State(state): State<Arc<AppState>>, Query(q): Query<TaskQuery>) -> Result<Json<serde_json::Value>, OverseerError> {
    let tasks = state.jobs.list_tasks(q.status.as_deref(), q.assigned_to.as_deref(), q.run_id.as_deref())?;
    Ok(Json(serde_json::to_value(tasks).unwrap()))
}
```

- [ ] **Step 5: Create `src/overseer/src/api/artifacts.rs`**

```rust
use crate::error::OverseerError;
use crate::services::AppState;
use axum::{extract::{Path, Query, State}, routing::{get, post}, Json, Router};
use axum::body::Bytes;
use axum::http::header;
use axum::response::IntoResponse;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct StoreArtifactRequest {
    pub name: String,
    pub content_type: String,
    pub data: String, // base64 encoded
    pub run_id: Option<String>,
}

#[derive(Deserialize)]
pub struct ArtifactQuery {
    pub run_id: Option<String>,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", post(store_artifact).get(list_artifacts))
        .route("/{id}", get(get_artifact))
        .with_state(state)
}

async fn store_artifact(
    State(state): State<Arc<AppState>>,
    Json(req): Json<StoreArtifactRequest>,
) -> Result<Json<serde_json::Value>, OverseerError> {
    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD
        .decode(&req.data)
        .map_err(|e| OverseerError::Validation(format!("invalid base64: {e}")))?;

    let meta = state.artifacts.store(&req.name, &req.content_type, &data, req.run_id.as_deref())?;
    Ok(Json(serde_json::to_value(meta).unwrap()))
}

async fn get_artifact(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, OverseerError> {
    let (meta, data) = state.artifacts.get(&id)?;
    Ok((
        [(header::CONTENT_TYPE, meta.content_type)],
        Bytes::from(data),
    ))
}

async fn list_artifacts(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ArtifactQuery>,
) -> Result<Json<serde_json::Value>, OverseerError> {
    let artifacts = state.artifacts.list(q.run_id.as_deref())?;
    Ok(Json(serde_json::to_value(artifacts).unwrap()))
}
```

Note: add `base64 = "0.22"` to Cargo.toml dependencies.

- [ ] **Step 6: Add api module to main.rs, verify compilation**

Add `mod api;` to `main.rs`.

```bash
cd src/overseer && cargo check
```

Expected: compiles.

- [ ] **Step 7: Commit**

```bash
git add src/overseer/
git commit -m "feat(overseer): HTTP API — all REST endpoints"
```

---

### Task 7: MCP server

**Files:**
- Create: `src/overseer/src/mcp/mod.rs`

- [ ] **Step 1: Create `src/overseer/src/mcp/mod.rs`**

```rust
use crate::services::AppState;
use rmcp::{
    ServerHandler, tool, tool_handler, tool_router,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo, CallToolResult, Content},
    ErrorData as McpError,
    schemars,
};
use std::sync::Arc;

// -- Parameter structs --------------------------------------------------------

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StoreMemoryParams {
    #[schemars(description = "The text content to store as a memory")]
    pub content: String,
    #[schemars(description = "Which agent or session is storing this")]
    pub source: String,
    #[schemars(description = "Categorization tags")]
    #[serde(default)]
    pub tags: Vec<String>,
    #[schemars(description = "Optional expiry timestamp (ISO 8601)")]
    pub expires_at: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RecallMemoryParams {
    #[schemars(description = "Search query text")]
    pub query: String,
    #[schemars(description = "Filter by tags")]
    pub tags: Option<Vec<String>>,
    #[schemars(description = "Max results to return")]
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize { 10 }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteMemoryParams {
    #[schemars(description = "Memory ID to delete")]
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LogDecisionParams {
    pub agent: String,
    pub context: String,
    pub decision: String,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub run_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QueryDecisionsParams {
    pub agent: Option<String>,
    pub tags: Option<Vec<String>>,
    #[serde(default = "default_decision_limit")]
    pub limit: usize,
}

fn default_decision_limit() -> usize { 20 }

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateJobDefinitionParams {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StartJobParams {
    pub definition_id: String,
    pub triggered_by: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateJobRunParams {
    pub id: String,
    pub status: Option<String>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateTaskParams {
    pub subject: String,
    pub run_id: Option<String>,
    pub assigned_to: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateTaskParams {
    pub id: String,
    pub status: Option<String>,
    pub assigned_to: Option<String>,
    pub output: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListTasksParams {
    pub status: Option<String>,
    pub assigned_to: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StoreArtifactParams {
    pub name: String,
    pub content_type: String,
    #[schemars(description = "Base64-encoded artifact data")]
    pub data: String,
    pub run_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetArtifactParams {
    pub id: String,
}

// -- MCP Server ---------------------------------------------------------------

#[derive(Clone)]
pub struct OverseerMcp {
    state: Arc<AppState>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl OverseerMcp {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state, tool_router: Self::tool_router() }
    }

    #[tool(description = "Store a memory with optional tags and expiry")]
    fn store_memory(&self, Parameters(p): Parameters<StoreMemoryParams>) -> Result<CallToolResult, McpError> {
        let mem = self.state.memory.store(&p.content, &p.source, &p.tags, &[], p.expires_at.as_deref())
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&mem).unwrap())]))
    }

    #[tool(description = "Search memories by semantic similarity")]
    fn recall_memory(&self, Parameters(p): Parameters<RecallMemoryParams>) -> Result<CallToolResult, McpError> {
        let tags = p.tags.as_deref();
        let results = self.state.memory.recall(&p.query, tags, p.limit)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&results).unwrap())]))
    }

    #[tool(description = "Delete a memory by ID")]
    fn delete_memory(&self, Parameters(p): Parameters<DeleteMemoryParams>) -> Result<CallToolResult, McpError> {
        let deleted = self.state.memory.delete(&p.id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(format!("{{\"deleted\": {deleted}}"))]))
    }

    #[tool(description = "Record a decision with context and reasoning")]
    fn log_decision(&self, Parameters(p): Parameters<LogDecisionParams>) -> Result<CallToolResult, McpError> {
        let d = self.state.decisions.log(&p.agent, &p.context, &p.decision, &p.reasoning, &p.tags, p.run_id.as_deref())
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&d).unwrap())]))
    }

    #[tool(description = "Query past decisions by agent, tags, or text")]
    fn query_decisions(&self, Parameters(p): Parameters<QueryDecisionsParams>) -> Result<CallToolResult, McpError> {
        let tags = p.tags.as_deref();
        let results = self.state.decisions.query(p.agent.as_deref(), tags, p.limit)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&results).unwrap())]))
    }

    #[tool(description = "Register a reusable job definition")]
    fn create_job_definition(&self, Parameters(p): Parameters<CreateJobDefinitionParams>) -> Result<CallToolResult, McpError> {
        let config = p.config.unwrap_or(serde_json::json!({}));
        let def = self.state.jobs.create_definition(&p.name, &p.description, &config)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&def).unwrap())]))
    }

    #[tool(description = "Start a job run from a definition")]
    fn start_job(&self, Parameters(p): Parameters<StartJobParams>) -> Result<CallToolResult, McpError> {
        let run = self.state.jobs.start_run(&p.definition_id, &p.triggered_by, p.parent_id.as_deref())
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&run).unwrap())]))
    }

    #[tool(description = "Update a job run's status, result, or error")]
    fn update_job_run(&self, Parameters(p): Parameters<UpdateJobRunParams>) -> Result<CallToolResult, McpError> {
        let run = self.state.jobs.update_run(&p.id, p.status.as_deref(), p.result.as_ref(), p.error.as_deref())
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&run).unwrap())]))
    }

    #[tool(description = "Create a task, optionally within a job run")]
    fn create_task(&self, Parameters(p): Parameters<CreateTaskParams>) -> Result<CallToolResult, McpError> {
        let task = self.state.jobs.create_task(&p.subject, p.run_id.as_deref(), p.assigned_to.as_deref())
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&task).unwrap())]))
    }

    #[tool(description = "Update a task's status, assignee, or output")]
    fn update_task(&self, Parameters(p): Parameters<UpdateTaskParams>) -> Result<CallToolResult, McpError> {
        let task = self.state.jobs.update_task(&p.id, p.status.as_deref(), p.assigned_to.as_deref(), p.output.as_ref())
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&task).unwrap())]))
    }

    #[tool(description = "List tasks filtered by status, assignee, or job run")]
    fn list_tasks(&self, Parameters(p): Parameters<ListTasksParams>) -> Result<CallToolResult, McpError> {
        let tasks = self.state.jobs.list_tasks(p.status.as_deref(), p.assigned_to.as_deref(), p.run_id.as_deref())
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&tasks).unwrap())]))
    }

    #[tool(description = "Store a base64-encoded artifact")]
    fn store_artifact(&self, Parameters(p): Parameters<StoreArtifactParams>) -> Result<CallToolResult, McpError> {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD
            .decode(&p.data)
            .map_err(|e| McpError::invalid_params(format!("invalid base64: {e}"), None))?;
        let meta = self.state.artifacts.store(&p.name, &p.content_type, &data, p.run_id.as_deref())
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&meta).unwrap())]))
    }

    #[tool(description = "Retrieve an artifact by ID")]
    fn get_artifact(&self, Parameters(p): Parameters<GetArtifactParams>) -> Result<CallToolResult, McpError> {
        let (meta, _data) = self.state.artifacts.get(&p.id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        // Return metadata as JSON; actual blob retrieval is better done via HTTP
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&meta).unwrap())]))
    }
}

#[tool_handler]
impl ServerHandler for OverseerMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_instructions("Overseer — persistent memory, job orchestration, decisions, and artifacts for the Kerrigan agentic platform.".to_string())
    }
}
```

- [ ] **Step 2: Add mcp module to main.rs, verify compilation**

Add `mod mcp;` to `main.rs`.

```bash
cd src/overseer && cargo check
```

Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add src/overseer/src/mcp/
git commit -m "feat(overseer): MCP server — all tools defined"
```

---

### Task 8: Main entrypoint — wire everything together

**Files:**
- Modify: `src/overseer/src/main.rs`

- [ ] **Step 1: Write the full main.rs**

```rust
mod api;
mod config;
mod db;
mod embedding;
mod error;
mod mcp;
mod services;

use config::Config;
use db::Database;
use embedding::stub::StubEmbedding;
use mcp::OverseerMcp;
use services::AppState;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("overseer.toml"));

    let config = Config::load(&config_path)?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.logging.level)),
        )
        .init();

    tracing::info!("overseer starting");

    let database = Arc::new(Database::open(&config.storage.database_path)?);
    tracing::info!("database opened at {:?}", config.storage.database_path);

    let embedder: Arc<dyn embedding::EmbeddingProvider> = Arc::new(StubEmbedding::new(384));
    tracing::info!("embedding provider: {}", embedder.model_name());

    let state = Arc::new(AppState::new(
        database,
        embedder,
        config.storage.artifact_path.clone(),
    ));

    // HTTP server
    let http_router = api::router(state.clone());
    let http_addr = format!("0.0.0.0:{}", config.server.http_port);
    let listener = tokio::net::TcpListener::bind(&http_addr).await?;
    tracing::info!("HTTP server listening on {}", http_addr);

    match config.server.mcp_transport.as_str() {
        "stdio" => {
            // Run HTTP in a background task, MCP on stdio in foreground
            tokio::spawn(async move {
                axum::serve(listener, http_router).await.unwrap();
            });

            tracing::info!("MCP server starting on stdio");
            let mcp_server = OverseerMcp::new(state);
            let service = rmcp::ServiceExt::serve(mcp_server, rmcp::transport::stdio()).await?;
            service.waiting().await?;
        }
        _ => {
            // HTTP only mode (MCP can be added via streamable HTTP later)
            tracing::info!("running in HTTP-only mode");
            axum::serve(listener, http_router)
                .with_graceful_shutdown(async {
                    tokio::signal::ctrl_c().await.unwrap();
                    tracing::info!("shutting down");
                })
                .await?;
        }
    }

    Ok(())
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cd src/overseer && cargo check
```

Expected: compiles with no errors.

- [ ] **Step 3: Verify Buck2 build**

```bash
buck2 build root//src/overseer:overseer
```

Expected: BUILD SUCCEEDED.

- [ ] **Step 4: Smoke test — start overseer and hit the HTTP API**

Create a minimal `overseer.toml` at the repo root:

```toml
[server]
http_port = 3100
mcp_transport = "http"

[storage]
database_path = "data/overseer.db"
artifact_path = "data/artifacts"
```

Start overseer:
```bash
buck2 run root//src/overseer:overseer -- overseer.toml &
```

Test the API:
```bash
# Store a memory
curl -s -X POST http://localhost:3100/api/memories -H 'Content-Type: application/json' -d '{"content":"test memory","source":"test"}'

# Search memories
curl -s 'http://localhost:3100/api/memories/search?q=test'

# Create a job definition
curl -s -X POST http://localhost:3100/api/jobs/definitions -H 'Content-Type: application/json' -d '{"name":"test-job","description":"a test"}'

# Log a decision
curl -s -X POST http://localhost:3100/api/decisions -H 'Content-Type: application/json' -d '{"agent":"test","context":"testing","decision":"approve","reasoning":"it works"}'
```

Expected: JSON responses for each request.

Kill overseer and clean up:
```bash
kill %1
rm -rf data/
```

- [ ] **Step 5: Commit**

```bash
git add src/overseer/src/main.rs overseer.toml
git commit -m "feat(overseer): wire up main — HTTP + MCP server running"
```

---

### Task 9: Update CLAUDE.md and overseer CLAUDE.md

**Files:**
- Modify: `CLAUDE.md`
- Create: `src/overseer/CLAUDE.md`

- [ ] **Step 1: Create `src/overseer/CLAUDE.md`**

Document overseer's architecture, module structure, how to add new endpoints/tools, and test patterns — all derived from the actual code written in previous tasks.

- [ ] **Step 2: Update root `CLAUDE.md`**

Update the Components > Overseer section to reflect the actual implementation: HTTP API on port 3100, MCP via stdio, SQLite + sqlite-vec, the service layer architecture.

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md src/overseer/CLAUDE.md
git commit -m "docs: update CLAUDE.md with overseer architecture details"
```

---

## Follow-up (not in initial scope)

- **MCP Resources** — the spec defines read-only resources (memory://search, jobs://definitions, etc.). The tools cover all functionality; resources are additive convenience for MCP clients. Add after core is stable.
- **Real embedding provider** — swap stub for AI HAT 2 local model or remote API.
- **Expired memory cleanup** — background task or on-query pruning for memories past `expires_at`.
