# Overseer: PostgreSQL/pgvector + Object Store Support

**Date:** 2026-03-30
**Status:** Draft

## Context

Overseer is evolving from a local dev tool into a persistent, deployed service. The current storage stack — SQLite + sqlite-vec + local filesystem — works well for single-node use but doesn't fit a production deployment where you want managed databases, durable object storage, and the ability to run multiple instances. This design adds PostgreSQL/pgvector and object store (S3, etc.) as configurable backends while keeping SQLite + local filesystem available for dev/testing.

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| DB abstraction | Trait-based (`Arc<dyn Database>`) | Clean separation, each backend self-contained, testable independently |
| Query builder | sea-query + sqlx | sea-query generates DB-agnostic SQL; sqlx executes. Avoids maintaining parallel raw SQL for standard CRUD |
| Vector search | Backend-specific raw SQL | sqlite-vec and pgvector have fundamentally different query syntax; no useful abstraction |
| Blob storage | `object_store` crate (arrow-rs) | Unified `ObjectStore` trait with impls for local FS, S3, GCS, Azure. Async/tokio-native |
| Config | URL-based auto-detection | `database_url` scheme selects backend (`sqlite://` vs `postgres://`); `artifact_url` scheme selects store (`file://` vs `s3://`) |
| Migrations | sqlx migrate | Per-backend migration directories, run at startup |
| Both backends | SQLite + Postgres selectable via config | Keeps SQLite for dev/testing, Postgres for production |

## Configuration

```toml
[storage]
# Backend auto-detected from URL scheme:
#   sqlite://data/overseer.db       → SQLite + sqlite-vec
#   postgres://user:pass@host/db    → PostgreSQL + pgvector
database_url = "sqlite://data/overseer.db"

# Blob backend auto-detected from URL scheme:
#   file://data/artifacts            → local filesystem
#   s3://bucket/prefix               → AWS S3 / MinIO / R2
#   gs://bucket/prefix               → Google Cloud Storage
#   az://container/prefix            → Azure Blob Storage
artifact_url = "file://data/artifacts"

# S3-specific config (optional, only when artifact_url is s3://)
# [storage.s3]
# region = "us-east-1"
# endpoint = "http://localhost:9000"   # MinIO/custom endpoint
# access_key_env = "AWS_ACCESS_KEY_ID"
# secret_key_env = "AWS_SECRET_ACCESS_KEY"
```

### Config Struct Changes

`StorageConfig` replaces `database_path` and `artifact_path` with URL-based fields:

```rust
#[derive(Debug, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_database_url")]
    pub database_url: String,        // sqlite://... or postgres://...

    #[serde(default = "default_artifact_url")]
    pub artifact_url: String,        // file://... or s3://... or gs://...

    #[serde(default)]
    pub s3: Option<S3Config>,        // optional S3-specific settings
}

#[derive(Debug, Deserialize)]
pub struct S3Config {
    pub region: Option<String>,
    pub endpoint: Option<String>,
    pub access_key_env: Option<String>,
    pub secret_key_env: Option<String>,
}
```

Defaults: `sqlite://data/overseer.db` and `file://data/artifacts` (preserves current behavior).

## Database Trait

A `Database` trait in `src/db/mod.rs` replaces direct `SqlitePool` usage:

```rust
#[async_trait]
pub trait Database: Send + Sync {
    // --- Memory ---
    async fn insert_memory(&self, provider_name: &str, content: &str, embedding: &[f32],
        embedding_model: &str, source: &str, tags: &[String],
        expires_at: Option<&str>) -> Result<Memory>;
    async fn get_memory(&self, id: &str) -> Result<Memory>;
    async fn delete_memory(&self, provider_name: &str, id: &str) -> Result<()>;
    async fn search_memories(&self, provider_name: &str, query_embedding: &[f32],
        tags_filter: Option<&[String]>, limit: usize) -> Result<Vec<MemorySearchResult>>;
    async fn insert_memory_link(&self, memory_id: &str, linked_id: &str,
        linked_type: &str, relation_type: &str) -> Result<()>;
    async fn create_embedding_table(&self, provider_name: &str, dimensions: usize) -> Result<()>;

    // --- Jobs ---
    async fn create_job_definition(&self, name: &str, description: &str,
        config: &serde_json::Value) -> Result<JobDefinition>;
    async fn get_job_definition(&self, id: &str) -> Result<Option<JobDefinition>>;
    async fn list_job_definitions(&self) -> Result<Vec<JobDefinition>>;
    async fn start_job_run(&self, definition_id: &str, triggered_by: &str,
        parent_id: Option<&str>) -> Result<JobRun>;
    async fn get_job_run(&self, id: &str) -> Result<Option<JobRun>>;
    async fn update_job_run(&self, id: &str, status: &str,
        result: Option<&serde_json::Value>, error: Option<&str>) -> Result<JobRun>;
    async fn list_job_runs(&self, status: Option<&str>) -> Result<Vec<JobRun>>;
    async fn create_task(&self, subject: &str, run_id: &str,
        assigned_to: Option<&str>) -> Result<Task>;
    async fn get_task(&self, id: &str) -> Result<Option<Task>>;
    async fn update_task(&self, id: &str, status: Option<&str>,
        assigned_to: Option<&str>, output: Option<&serde_json::Value>) -> Result<Task>;
    async fn list_tasks(&self, status: Option<&str>, assigned_to: Option<&str>,
        run_id: Option<&str>) -> Result<Vec<Task>>;

    // --- Decisions ---
    async fn insert_decision(&self, agent: &str, context: &str, decision: &str,
        reasoning: &str, tags: &[String], run_id: Option<&str>) -> Result<Decision>;
    async fn query_decisions(&self, agent: Option<&str>, tags: Option<&[String]>,
        limit: usize) -> Result<Vec<Decision>>;

    // --- Artifacts (metadata only) ---
    async fn insert_artifact(&self, id: &str, name: &str, content_type: &str,
        size: i64, run_id: Option<&str>) -> Result<ArtifactMetadata>;
    async fn get_artifact(&self, id: &str) -> Result<Option<ArtifactMetadata>>;
    async fn list_artifacts(&self, run_id: Option<&str>) -> Result<Vec<ArtifactMetadata>>;
}
```

### Implementations

**`SqliteDatabase`** (`src/db/sqlite.rs`):
- Wraps `SqlitePool`
- Registers sqlite-vec extension on init
- Uses sea-query with `SqliteQueryBuilder` for CRUD
- Vector search: raw SQL with `WHERE embedding MATCH ?1 AND k = ?2`
- Embedding tables: `CREATE VIRTUAL TABLE ... USING vec0(embedding float[N])`

**`PostgresDatabase`** (`src/db/postgres.rs`):
- Wraps `PgPool`
- Requires pgvector extension (`CREATE EXTENSION IF NOT EXISTS vector`)
- Uses sea-query with `PostgresQueryBuilder` for CRUD
- Vector search: raw SQL with `ORDER BY embedding <-> $1 LIMIT $2`
- Embedding tables: standard table with `vector(N)` column type

### sea-query Usage

Standard CRUD queries (insert, select, update, delete for jobs, decisions, artifacts, memory metadata) use sea-query's builder API:

```rust
// Example: insert artifact metadata
let query = Query::insert()
    .into_table(Artifacts::Table)
    .columns([Artifacts::Id, Artifacts::Name, Artifacts::ContentType, Artifacts::Size, Artifacts::RunId])
    .values_panic([id.into(), name.into(), content_type.into(), size.into(), run_id.into()])
    .to_string(SqliteQueryBuilder);  // or PostgresQueryBuilder
```

Table/column enums defined once, shared by both backends. The query builder selection is the only difference for CRUD.

Vector search queries remain as raw SQL per backend — sqlite-vec and pgvector syntax is too different for a query builder to abstract usefully.

## Artifact Storage with ObjectStore

`ArtifactService` holds `Arc<dyn ObjectStore>` instead of a `PathBuf`:

```rust
pub struct ArtifactService {
    db: Arc<dyn Database>,
    store: Arc<dyn object_store::ObjectStore>,
}
```

### Operations

- **store**: `store.put(&path, PutPayload::from(data)).await` → then `db.insert_artifact(...)`. Blob write first to avoid orphaned metadata.
- **get**: `db.get_artifact(id)` → `store.get(&path).await.bytes()`. Metadata from DB, blob from store.
- **list**: `db.list_artifacts(run_id)` — metadata only from DB.

### Object Path Convention

Artifacts stored at path `artifacts/{id}` within the configured store. For S3 with bucket `my-bucket` and prefix `data/`, the full key is `data/artifacts/{id}`.

### Factory

```rust
pub fn create_object_store(config: &StorageConfig) -> Result<Arc<dyn ObjectStore>> {
    let url = &config.artifact_url;
    match url.split("://").next() {
        Some("file") => Ok(Arc::new(LocalFileSystem::new_with_prefix(path)?)),
        Some("s3")   => Ok(Arc::new(AmazonS3Builder::from_env().with_url(url).build()?)),
        Some("gs")   => Ok(Arc::new(GoogleCloudStorageBuilder::from_env().with_url(url).build()?)),
        Some("az")   => Ok(Arc::new(MicrosoftAzureBuilder::from_env().with_url(url).build()?)),
        _            => Err(OverseerError::Validation("unsupported artifact_url scheme")),
    }
}
```

## Migrations

Replace `schema.sql` with sqlx migrate. Two migration directories:

```
migrations/
  sqlite/
    0001_initial.sql      # Current schema (memories, jobs, decisions, artifacts, memory_links)
  postgres/
    0001_initial.sql      # Equivalent schema using PostgreSQL types + pgvector
```

### PostgreSQL Schema Differences

| SQLite | PostgreSQL |
|--------|-----------|
| `TEXT` for JSON fields | `JSONB` |
| `TEXT` for timestamps | `TIMESTAMPTZ` with `DEFAULT now()` |
| `TEXT` for UUIDs | `UUID` (native type) |
| `INTEGER` rowid | `BIGSERIAL` |
| `REAL` for sizes | `BIGINT` |
| vec0 virtual tables | Standard table with `vector(N)` column |
| `DEFAULT (datetime('now'))` | `DEFAULT now()` |

### pgvector Embedding Table

```sql
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE memory_embeddings (
    id BIGSERIAL PRIMARY KEY,
    memory_id UUID NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    embedding vector,  -- dimension set per-provider
    UNIQUE(memory_id, provider)
);

CREATE INDEX ON memory_embeddings USING ivfflat (embedding vector_cosine_ops);
```

Note: pgvector uses a single `memory_embeddings` table with a `provider` column, rather than one table per provider. The `ON DELETE CASCADE` replaces the manual embedding cleanup in `delete_memory`.

## AppState & Service Changes

```rust
pub struct AppState {
    pub memory: MemoryService,
    pub jobs: JobService,
    pub decisions: DecisionService,
    pub artifacts: ArtifactService,
}

impl AppState {
    pub fn new(
        db: Arc<dyn Database>,
        registry: EmbeddingRegistry,
        artifact_store: Arc<dyn ObjectStore>,
    ) -> Self {
        Self {
            memory: MemoryService::new(db.clone(), registry),
            jobs: JobService::new(db.clone()),
            decisions: DecisionService::new(db.clone()),
            artifacts: ArtifactService::new(db, artifact_store),
        }
    }
}
```

Services change from `SqlitePool` → `Arc<dyn Database>`. Method calls change from `db::function(&pool, ...)` to `self.db.method(...)`.

## Error Handling

`OverseerError::Storage` currently wraps `sqlx::Error`. It will need to also handle `object_store::Error`:

```rust
pub enum OverseerError {
    Storage(String),          // was #[from] sqlx::Error, now stringified for both
    ObjectStore(String),      // object_store errors
    NotFound(String),
    Validation(String),
    Embedding(String),
    Io(#[from] std::io::Error),
    Internal(String),
}
```

Alternatively, keep `Storage` for DB errors and add `ObjectStore` variant for blob errors so they can be distinguished in logs.

## Startup Flow

```
main():
  1. Load config (overseer.toml)
  2. Parse database_url → create Arc<dyn Database>
     - sqlite:// → SqliteDatabase::new(url) [registers vec extension, runs migrations]
     - postgres:// → PostgresDatabase::new(url) [enables pgvector, runs migrations]
  3. Parse artifact_url → create Arc<dyn ObjectStore>
  4. Initialize embedding providers from config
  5. Create embedding tables via db.create_embedding_table() per provider
  6. Build AppState(db, registry, artifact_store)
  7. Start HTTP server
```

## New Dependencies

| Crate | Purpose |
|-------|---------|
| `sea-query` | DB-agnostic query builder |
| `async-trait` | Dyn-compatible async trait methods for `Arc<dyn Database>` |
| `object_store` | Blob storage abstraction (local FS, S3, GCS, Azure) |

Note: Native `async fn in trait` is stable but NOT dyn-compatible. Since we use `Arc<dyn Database>`, we need the `async-trait` crate (or `trait_variant`) to generate boxed futures for dynamic dispatch.

sqlx gains the `postgres` feature. `libsqlite3-sys` and `sqlite-vec` remain for the SQLite backend.

## Testing Strategy

- **SQLite backend tests**: Continue using `open_in_memory_named()` for fast, isolated unit tests
- **PostgreSQL backend tests**: Integration tests against a real Postgres instance (Docker or CI service). Gated behind `#[cfg(feature = "postgres-tests")]` or similar
- **ObjectStore tests**: Use `object_store::memory::InMemory` for unit tests. Integration tests with real S3/MinIO gated behind a feature flag
- **Database trait tests**: Shared test functions that run against both backends to verify behavioral parity

## Verification

1. `cargo check` — compiles with both SQLite and Postgres support
2. `cargo test` — existing tests pass (SQLite path unchanged)
3. Manual test: start with `database_url = "sqlite://..."` → existing behavior preserved
4. Manual test: start with `database_url = "postgres://..."` → connects, creates schema, CRUD works
5. Manual test: `artifact_url = "file://..."` → existing behavior preserved
6. Manual test: `artifact_url = "s3://..."` → stores/retrieves blobs from S3-compatible store
7. `buck2 build root//src/overseer:overseer` — hermetic build still works
