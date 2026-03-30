# PostgreSQL/pgvector + Object Store Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add PostgreSQL/pgvector and object store (S3, etc.) as configurable storage backends alongside the existing SQLite + filesystem, making overseer production-deployable.

**Architecture:** Extract a `Database` trait from the current SQLite-specific `db::` module, implement it for both SQLite and PostgreSQL. Replace filesystem artifact storage with the `object_store` crate. URL-based config auto-detects backends. sea-query generates DB-agnostic SQL for CRUD; vector search uses backend-specific raw SQL.

**Tech Stack:** sqlx (sqlite + postgres features), sea-query + sea-query-binder, async-trait, pgvector, object_store (arrow-rs)

**Spec:** `docs/specs/2026-03-30-postgres-objectstore-design.md`

---

## Phase 1: Database Trait Extraction

Refactor only — no new backends, no new dependencies (except async-trait). All existing tests continue to pass after each task.

### Task 1: Extract model types to `db/models.rs`

**Files:**
- Create: `src/overseer/src/db/models.rs`
- Modify: `src/overseer/src/db/mod.rs`
- Modify: `src/overseer/src/db/memory.rs`
- Modify: `src/overseer/src/db/jobs.rs`
- Modify: `src/overseer/src/db/decisions.rs`
- Modify: `src/overseer/src/db/artifacts.rs`

- [ ] **Step 1: Create `db/models.rs` with all shared model types**

Move these structs (with their derives) into the new file:
- `Memory`, `MemorySearchResult` from `db/memory.rs:7-23`
- `JobDefinition`, `JobRun`, `Task` from `db/jobs.rs:6-39`
- `Decision` from `db/decisions.rs:6-16`
- `ArtifactMetadata` from `db/artifacts.rs:5-13`

```rust
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct MemorySearchResult {
    pub memory: Memory,
    pub distance: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub config: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactMetadata {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub size: i64,
    pub run_id: Option<String>,
    pub created_at: String,
}
```

- [ ] **Step 2: Update `db/mod.rs` to export models**

Add `pub mod models;` and `pub use models::*;` to `db/mod.rs`.

- [ ] **Step 3: Remove struct definitions from `db/memory.rs`, `db/jobs.rs`, `db/decisions.rs`, `db/artifacts.rs`**

Replace the struct definitions with `use super::models::*;` in each file. Keep the `row_to_*` functions and free functions in place.

- [ ] **Step 4: Verify all tests pass**

Run: `cargo test -p overseer`
Expected: all tests pass unchanged (re-exports maintain the same public API)

- [ ] **Step 5: Commit**

```bash
git add src/overseer/src/db/models.rs src/overseer/src/db/mod.rs src/overseer/src/db/memory.rs src/overseer/src/db/jobs.rs src/overseer/src/db/decisions.rs src/overseer/src/db/artifacts.rs
git commit -m "refactor(db): extract model types to db/models.rs"
```

---

### Task 2: Add async-trait and define the Database trait

**Files:**
- Modify: `src/overseer/Cargo.toml`
- Modify: `src/overseer/BUCK`
- Create: `src/overseer/src/db/trait_def.rs`
- Modify: `src/overseer/src/db/mod.rs`

- [ ] **Step 1: Add async-trait dependency**

```bash
cd src/overseer && cargo add async-trait
```

- [ ] **Step 2: Regenerate Buck2 third-party targets**

```bash
./tools/buckify.sh
```

- [ ] **Step 3: Add async-trait to BUCK deps**

Add `"//third-party:async-trait"` to `OVERSEER_DEPS` in `src/overseer/BUCK`.

- [ ] **Step 4: Create `db/trait_def.rs` with the Database trait**

Define the trait with all 26 methods matching the current free function signatures exactly:

```rust
use async_trait::async_trait;
use crate::error::Result;
use super::models::*;

#[async_trait]
pub trait Database: Send + Sync {
    // Memory
    async fn insert_memory(
        &self,
        provider_name: &str,
        content: &str,
        embedding: &[f32],
        embedding_model: &str,
        source: &str,
        tags: &[String],
        expires_at: Option<&str>,
    ) -> Result<Memory>;

    async fn get_memory(&self, id: &str) -> Result<Memory>;

    async fn delete_memory(&self, provider_name: &str, id: &str) -> Result<()>;

    async fn search_memories(
        &self,
        provider_name: &str,
        query_embedding: &[f32],
        tags_filter: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>>;

    async fn insert_memory_link(
        &self,
        memory_id: &str,
        linked_id: &str,
        linked_type: &str,
        relation_type: &str,
    ) -> Result<()>;

    async fn create_embedding_table(
        &self,
        provider_name: &str,
        dimensions: usize,
    ) -> Result<()>;

    // Jobs
    async fn create_job_definition(
        &self,
        name: &str,
        description: &str,
        config: serde_json::Value,
    ) -> Result<JobDefinition>;

    async fn get_job_definition(&self, id: &str) -> Result<Option<JobDefinition>>;

    async fn list_job_definitions(&self) -> Result<Vec<JobDefinition>>;

    async fn start_job_run(
        &self,
        definition_id: &str,
        triggered_by: &str,
        parent_id: Option<&str>,
    ) -> Result<JobRun>;

    async fn get_job_run(&self, id: &str) -> Result<Option<JobRun>>;

    async fn update_job_run(
        &self,
        id: &str,
        status: Option<&str>,
        result: Option<serde_json::Value>,
        error: Option<&str>,
    ) -> Result<JobRun>;

    async fn list_job_runs(&self, status: Option<&str>) -> Result<Vec<JobRun>>;

    async fn create_task(
        &self,
        subject: &str,
        run_id: Option<&str>,
        assigned_to: Option<&str>,
    ) -> Result<Task>;

    async fn get_task(&self, id: &str) -> Result<Option<Task>>;

    async fn update_task(
        &self,
        id: &str,
        status: Option<&str>,
        assigned_to: Option<&str>,
        output: Option<serde_json::Value>,
    ) -> Result<Task>;

    async fn list_tasks(
        &self,
        status: Option<&str>,
        assigned_to: Option<&str>,
        run_id: Option<&str>,
    ) -> Result<Vec<Task>>;

    // Decisions
    async fn log_decision(
        &self,
        agent: &str,
        context: &str,
        decision: &str,
        reasoning: &str,
        tags: &[String],
        run_id: Option<&str>,
    ) -> Result<Decision>;

    async fn get_decision(&self, id: &str) -> Result<Option<Decision>>;

    async fn query_decisions(
        &self,
        agent: Option<&str>,
        tags: Option<&[String]>,
        limit: i64,
    ) -> Result<Vec<Decision>>;

    // Artifacts (metadata only)
    async fn insert_artifact(
        &self,
        id: &str,
        name: &str,
        content_type: &str,
        size: i64,
        run_id: Option<&str>,
    ) -> Result<ArtifactMetadata>;

    async fn get_artifact(&self, id: &str) -> Result<Option<ArtifactMetadata>>;

    async fn list_artifacts(&self, run_id: Option<&str>) -> Result<Vec<ArtifactMetadata>>;
}
```

- [ ] **Step 5: Export trait from `db/mod.rs`**

Add `mod trait_def; pub use trait_def::Database;`

- [ ] **Step 6: Verify compilation**

Run: `cargo check -p overseer`
Expected: compiles (trait defined but not yet implemented)

- [ ] **Step 7: Commit**

```bash
git add src/overseer/Cargo.toml src/overseer/BUCK src/overseer/src/db/trait_def.rs src/overseer/src/db/mod.rs
git commit -m "refactor(db): define Database trait with async-trait"
```

---

### Task 3: Create `SqliteDatabase` implementing the trait

**Files:**
- Create: `src/overseer/src/db/sqlite.rs`
- Modify: `src/overseer/src/db/mod.rs`

- [ ] **Step 1: Create `db/sqlite.rs` with `SqliteDatabase` struct**

```rust
use async_trait::async_trait;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;

use crate::error::OverseerError;
use super::models::*;
use super::trait_def::Database;
use crate::error::Result;

pub struct SqliteDatabase {
    pool: SqlitePool,
}
```

- [ ] **Step 2: Move SQLite-specific init functions into `SqliteDatabase`**

Move `register_vec_extension()`, `init_pool()`, `open()`, `open_in_memory()`, `open_in_memory_named()` from `db/mod.rs` into `SqliteDatabase` as associated functions / constructor methods:

```rust
impl SqliteDatabase {
    fn register_vec_extension() {
        // ... same as current db/mod.rs:19-28
    }

    async fn init_pool(opts: SqliteConnectOptions) -> std::result::Result<SqlitePool, OverseerError> {
        // ... same as current db/mod.rs:30-45
    }

    pub async fn open(path: &str) -> std::result::Result<Self, OverseerError> {
        let opts = SqliteConnectOptions::from_str(&format!("sqlite:{path}"))
            .map_err(OverseerError::Storage)?
            .create_if_missing(true)
            .pragma("journal_mode", "WAL")
            .pragma("foreign_keys", "ON");
        let pool = Self::init_pool(opts).await?;
        Ok(Self { pool })
    }

    pub async fn open_in_memory() -> std::result::Result<Self, OverseerError> {
        Self::open_in_memory_named("overseer_test").await
    }

    pub async fn open_in_memory_named(name: &str) -> std::result::Result<Self, OverseerError> {
        let opts = SqliteConnectOptions::from_str(
            &format!("sqlite:file:{name}?mode=memory&cache=shared"),
        )
        .map_err(OverseerError::Storage)?
        .pragma("journal_mode", "WAL")
        .pragma("foreign_keys", "ON");
        let pool = Self::init_pool(opts).await?;
        Ok(Self { pool })
    }
}
```

- [ ] **Step 3: Implement `Database` trait for `SqliteDatabase` by delegating to existing free functions**

```rust
#[async_trait]
impl Database for SqliteDatabase {
    async fn insert_memory(
        &self,
        provider_name: &str,
        content: &str,
        embedding: &[f32],
        embedding_model: &str,
        source: &str,
        tags: &[String],
        expires_at: Option<&str>,
    ) -> Result<Memory> {
        super::memory::insert_memory(
            &self.pool, provider_name, content, embedding,
            embedding_model, source, tags, expires_at,
        ).await
    }

    async fn get_memory(&self, id: &str) -> Result<Memory> {
        super::memory::get_memory(&self.pool, id).await
    }

    async fn delete_memory(&self, provider_name: &str, id: &str) -> Result<()> {
        super::memory::delete_memory(&self.pool, provider_name, id).await
    }

    async fn search_memories(
        &self,
        provider_name: &str,
        query_embedding: &[f32],
        tags_filter: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>> {
        super::memory::search_memories(
            &self.pool, provider_name, query_embedding, tags_filter, limit,
        ).await
    }

    async fn insert_memory_link(
        &self,
        memory_id: &str,
        linked_id: &str,
        linked_type: &str,
        relation_type: &str,
    ) -> Result<()> {
        super::memory::insert_memory_link(
            &self.pool, memory_id, linked_id, linked_type, relation_type,
        ).await
    }

    async fn create_embedding_table(
        &self,
        provider_name: &str,
        dimensions: usize,
    ) -> Result<()> {
        super::create_embedding_table(&self.pool, provider_name, dimensions).await
    }

    async fn create_job_definition(
        &self,
        name: &str,
        description: &str,
        config: serde_json::Value,
    ) -> Result<JobDefinition> {
        super::jobs::create_job_definition(&self.pool, name, description, config).await
    }

    async fn get_job_definition(&self, id: &str) -> Result<Option<JobDefinition>> {
        super::jobs::get_job_definition(&self.pool, id).await
    }

    async fn list_job_definitions(&self) -> Result<Vec<JobDefinition>> {
        super::jobs::list_job_definitions(&self.pool).await
    }

    async fn start_job_run(
        &self,
        definition_id: &str,
        triggered_by: &str,
        parent_id: Option<&str>,
    ) -> Result<JobRun> {
        super::jobs::start_job_run(&self.pool, definition_id, triggered_by, parent_id).await
    }

    async fn get_job_run(&self, id: &str) -> Result<Option<JobRun>> {
        super::jobs::get_job_run(&self.pool, id).await
    }

    async fn update_job_run(
        &self,
        id: &str,
        status: Option<&str>,
        result: Option<serde_json::Value>,
        error: Option<&str>,
    ) -> Result<JobRun> {
        super::jobs::update_job_run(&self.pool, id, status, result, error).await
    }

    async fn list_job_runs(&self, status: Option<&str>) -> Result<Vec<JobRun>> {
        super::jobs::list_job_runs(&self.pool, status).await
    }

    async fn create_task(
        &self,
        subject: &str,
        run_id: Option<&str>,
        assigned_to: Option<&str>,
    ) -> Result<Task> {
        super::jobs::create_task(&self.pool, subject, run_id, assigned_to).await
    }

    async fn get_task(&self, id: &str) -> Result<Option<Task>> {
        super::jobs::get_task(&self.pool, id).await
    }

    async fn update_task(
        &self,
        id: &str,
        status: Option<&str>,
        assigned_to: Option<&str>,
        output: Option<serde_json::Value>,
    ) -> Result<Task> {
        super::jobs::update_task(&self.pool, id, status, assigned_to, output).await
    }

    async fn list_tasks(
        &self,
        status: Option<&str>,
        assigned_to: Option<&str>,
        run_id: Option<&str>,
    ) -> Result<Vec<Task>> {
        super::jobs::list_tasks(&self.pool, status, assigned_to, run_id).await
    }

    async fn log_decision(
        &self,
        agent: &str,
        context: &str,
        decision: &str,
        reasoning: &str,
        tags: &[String],
        run_id: Option<&str>,
    ) -> Result<Decision> {
        super::decisions::log_decision(
            &self.pool, agent, context, decision, reasoning, tags, run_id,
        ).await
    }

    async fn get_decision(&self, id: &str) -> Result<Option<Decision>> {
        super::decisions::get_decision(&self.pool, id).await
    }

    async fn query_decisions(
        &self,
        agent: Option<&str>,
        tags: Option<&[String]>,
        limit: i64,
    ) -> Result<Vec<Decision>> {
        super::decisions::query_decisions(&self.pool, agent, tags, limit).await
    }

    async fn insert_artifact(
        &self,
        id: &str,
        name: &str,
        content_type: &str,
        size: i64,
        run_id: Option<&str>,
    ) -> Result<ArtifactMetadata> {
        super::artifacts::insert_artifact(&self.pool, id, name, content_type, size, run_id).await
    }

    async fn get_artifact(&self, id: &str) -> Result<Option<ArtifactMetadata>> {
        super::artifacts::get_artifact(&self.pool, id).await
    }

    async fn list_artifacts(&self, run_id: Option<&str>) -> Result<Vec<ArtifactMetadata>> {
        super::artifacts::list_artifacts(&self.pool, run_id).await
    }
}
```

- [ ] **Step 4: Update `db/mod.rs`**

Remove moved functions (`register_vec_extension`, `init_pool`, `open`, `open_in_memory`, `open_in_memory_named`). Keep `create_embedding_table` (still used by `SqliteDatabase` impl). Add:

```rust
pub mod sqlite;
pub use sqlite::SqliteDatabase;
```

Add backward-compat re-exports so existing code that calls `db::open()` still compiles during transition:

```rust
pub async fn open(path: &str) -> Result<SqliteDatabase, OverseerError> {
    SqliteDatabase::open(path).await
}
pub async fn open_in_memory() -> Result<SqliteDatabase, OverseerError> {
    SqliteDatabase::open_in_memory().await
}
pub async fn open_in_memory_named(name: &str) -> Result<SqliteDatabase, OverseerError> {
    SqliteDatabase::open_in_memory_named(name).await
}
```

- [ ] **Step 5: Update `db/mod.rs` tests to use `SqliteDatabase`**

The existing tests in `db/mod.rs:93-171` use `open_in_memory()` — these still work via re-exports. Update `create_embedding_table` calls to go through the `Database` trait if desired, but can remain as-is for now.

- [ ] **Step 6: Verify all tests pass**

Run: `cargo test -p overseer`
Expected: all tests pass (delegation layer is transparent)

- [ ] **Step 7: Commit**

```bash
git add src/overseer/src/db/sqlite.rs src/overseer/src/db/mod.rs
git commit -m "refactor(db): create SqliteDatabase implementing Database trait"
```

---

### Task 4: Update services to use `Arc<dyn Database>`

**Files:**
- Modify: `src/overseer/src/services/mod.rs`
- Modify: `src/overseer/src/services/memory.rs`
- Modify: `src/overseer/src/services/jobs.rs`
- Modify: `src/overseer/src/services/decisions.rs`
- Modify: `src/overseer/src/services/artifacts.rs`

- [ ] **Step 1: Update `services/mod.rs`**

```rust
pub mod artifacts;
pub mod decisions;
pub mod jobs;
pub mod memory;

use std::path::PathBuf;
use std::sync::Arc;

use crate::db::Database;
use crate::embedding::EmbeddingRegistry;

pub struct AppState {
    pub memory: memory::MemoryService,
    pub jobs: jobs::JobService,
    pub decisions: decisions::DecisionService,
    pub artifacts: artifacts::ArtifactService,
}

impl AppState {
    pub fn new(db: Arc<dyn Database>, registry: EmbeddingRegistry, artifact_path: PathBuf) -> Self {
        Self {
            memory: memory::MemoryService::new(db.clone(), registry),
            jobs: jobs::JobService::new(db.clone()),
            decisions: decisions::DecisionService::new(db.clone()),
            artifacts: artifacts::ArtifactService::new(db, artifact_path),
        }
    }
}
```

- [ ] **Step 2: Update `services/memory.rs`**

Change `pool: SqlitePool` to `db: Arc<dyn Database>`. Change all `db::function(&self.pool, ...)` to `self.db.method(...)`:

```rust
use std::sync::Arc;

use crate::db::Database;
use crate::db::models::{Memory, MemorySearchResult};
use crate::embedding::EmbeddingRegistry;
use crate::error::Result;

pub struct MemoryService {
    db: Arc<dyn Database>,
    registry: EmbeddingRegistry,
}

impl MemoryService {
    pub fn new(db: Arc<dyn Database>, registry: EmbeddingRegistry) -> Self {
        Self { db, registry }
    }

    pub async fn store(
        &self,
        content: &str,
        source: &str,
        tags: &[String],
        expires_at: Option<&str>,
    ) -> Result<Memory> {
        let provider = self.registry.get_default();
        let provider_name = self.registry.default_name();
        let embedding = provider.embed(content).await?;
        self.db.insert_memory(
            provider_name, content, &embedding, provider_name, source, tags, expires_at,
        ).await
    }

    pub async fn recall(
        &self,
        query: &str,
        tags_filter: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>> {
        let provider = self.registry.get_default();
        let provider_name = self.registry.default_name();
        let embedding = provider.embed(query).await?;
        self.db.search_memories(provider_name, &embedding, tags_filter, limit).await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let memory = self.db.get_memory(id).await?;
        self.db.delete_memory(&memory.embedding_model, id).await
    }
}
```

- [ ] **Step 3: Update `services/jobs.rs`**

Same pattern — replace `pool: SqlitePool` with `db: Arc<dyn Database>`, delegate to `self.db.method(...)` instead of `db::function(&self.pool, ...)`.

- [ ] **Step 4: Update `services/decisions.rs`**

Same pattern.

- [ ] **Step 5: Update `services/artifacts.rs`**

Change `pool: SqlitePool` to `db: Arc<dyn Database>`:

```rust
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;

use crate::db::Database;
use crate::db::models::ArtifactMetadata;
use crate::error::{OverseerError, Result};

pub struct ArtifactService {
    db: Arc<dyn Database>,
    artifact_path: PathBuf,
}

impl ArtifactService {
    pub fn new(db: Arc<dyn Database>, artifact_path: PathBuf) -> Self {
        Self { db, artifact_path }
    }

    pub async fn store(
        &self,
        name: &str,
        content_type: &str,
        data: &[u8],
        run_id: Option<&str>,
    ) -> Result<ArtifactMetadata> {
        let id = uuid::Uuid::new_v4().to_string();
        let dest = self.artifact_path.join(&id);
        fs::create_dir_all(&self.artifact_path).await?;
        fs::write(&dest, data).await?;
        self.db.insert_artifact(&id, name, content_type, data.len() as i64, run_id).await
    }

    pub async fn get(&self, id: &str) -> Result<(ArtifactMetadata, Vec<u8>)> {
        let metadata = self.db.get_artifact(id).await?
            .ok_or_else(|| OverseerError::NotFound(format!("artifact {id}")))?;
        let path = self.artifact_path.join(id);
        let data = fs::read(&path).await?;
        Ok((metadata, data))
    }

    pub async fn list(&self, run_id: Option<&str>) -> Result<Vec<ArtifactMetadata>> {
        self.db.list_artifacts(run_id).await
    }
}
```

- [ ] **Step 6: Update all service tests**

In each test module, change from `open_in_memory_named(name)` + `SqlitePool` to `SqliteDatabase::open_in_memory_named(name)` + `Arc::new(db)`:

For `services/memory.rs` tests:
```rust
async fn make_service(name: &str) -> MemoryService {
    let db = SqliteDatabase::open_in_memory_named(name).await.expect("db opens");
    let db: Arc<dyn Database> = Arc::new(db);
    db.create_embedding_table("stub", 384).await.expect("create table");
    let mut providers: HashMap<String, Arc<dyn EmbeddingProvider>> = HashMap::new();
    providers.insert("stub".into(), Arc::new(StubEmbedding::new(384)));
    let registry = EmbeddingRegistry::new(providers, "stub".into()).unwrap();
    MemoryService::new(db, registry)
}
```

For `services/jobs.rs` tests:
```rust
let db = SqliteDatabase::open_in_memory_named("svc_jobs_test_def").await.expect("db opens");
let svc = JobService::new(Arc::new(db));
```

For `services/decisions.rs` tests — same pattern.

For `services/artifacts.rs` tests:
```rust
let db = SqliteDatabase::open_in_memory_named("svc_artifacts_test_store").await.expect("db opens");
let dir = test_dir();
let svc = ArtifactService::new(Arc::new(db), dir);
```

- [ ] **Step 7: Verify all tests pass**

Run: `cargo test -p overseer`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add src/overseer/src/services/
git commit -m "refactor(services): use Arc<dyn Database> instead of SqlitePool"
```

---

### Task 5: Update main.rs to use Database trait

**Files:**
- Modify: `src/overseer/src/main.rs`

- [ ] **Step 1: Update main.rs**

Change pool creation and embedding table setup to use the trait:

```rust
use db::{Database, SqliteDatabase};
// ...

let db_path = config.storage.database_path.to_string_lossy();
let db: Arc<dyn Database> = Arc::new(SqliteDatabase::open(&db_path).await?);
tracing::info!("database opened at {:?}", db_path);

// ... embedding provider setup ...

for (name, provider_config) in &config.embedding.providers {
    // ... provider creation ...
    db.create_embedding_table(name, provider_config.dimensions).await?;
    providers.insert(name.clone(), provider);
}

// ...

let state = Arc::new(AppState::new(
    db,
    registry,
    config.storage.artifact_path.clone(),
));
```

- [ ] **Step 2: Verify build and tests**

Run: `cargo test -p overseer && buck2 build root//src/overseer:overseer`
Expected: all pass

- [ ] **Step 3: Commit**

```bash
git add src/overseer/src/main.rs
git commit -m "refactor(main): use Database trait for startup"
```

---

### Task 6: Remove backward-compat re-exports from `db/mod.rs`

**Files:**
- Modify: `src/overseer/src/db/mod.rs`

- [ ] **Step 1: Remove the `open`, `open_in_memory`, `open_in_memory_named` re-exports**

Now that all callers (main.rs, service tests, db tests) use `SqliteDatabase` directly, remove the wrapper functions from `db/mod.rs`.

- [ ] **Step 2: Update any remaining test imports**

Grep for `db::open` and `db::open_in_memory` across the codebase. Update to `db::SqliteDatabase::open_in_memory_named(...)`.

- [ ] **Step 3: Verify all tests pass**

Run: `cargo test -p overseer`

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/db/
git commit -m "refactor(db): remove backward-compat re-exports"
```

---

## Phase 2: sea-query Integration

Replace hand-written SQL in the SQLite free functions with sea-query builders. This prepares for the PostgreSQL backend which will use the same table/column enums with `PostgresQueryBuilder`.

### Task 7: Add sea-query dependencies

**Files:**
- Modify: `src/overseer/Cargo.toml`
- Modify: `src/overseer/BUCK`

- [ ] **Step 1: Add sea-query and sea-query-binder**

```bash
cd src/overseer && cargo add sea-query --features backend-sqlite && cargo add sea-query-binder --features sqlx-sqlite,runtime-tokio
```

- [ ] **Step 2: Regenerate Buck2 targets and update BUCK**

```bash
./tools/buckify.sh
```

Add `"//third-party:sea-query"` and `"//third-party:sea-query-binder"` to `OVERSEER_DEPS` in `src/overseer/BUCK`.

- [ ] **Step 3: Verify build**

Run: `cargo check -p overseer && buck2 build root//src/overseer:overseer`

- [ ] **Step 4: Commit**

```bash
git add src/overseer/Cargo.toml src/overseer/BUCK Cargo.lock third-party/
git commit -m "deps: add sea-query and sea-query-binder"
```

---

### Task 8: Define sea-query table/column enums

**Files:**
- Create: `src/overseer/src/db/tables.rs`
- Modify: `src/overseer/src/db/mod.rs`

- [ ] **Step 1: Create `db/tables.rs`**

```rust
use sea_query::Iden;

#[derive(Iden)]
pub enum Memories {
    Table,
    Id,
    Content,
    EmbeddingModel,
    Source,
    Tags,
    ExpiresAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
pub enum MemoryLinks {
    Table,
    MemoryId,
    LinkedId,
    LinkedType,
    RelationType,
}

#[derive(Iden)]
pub enum JobDefinitions {
    Table,
    Id,
    Name,
    Description,
    Config,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
pub enum JobRuns {
    Table,
    Id,
    DefinitionId,
    ParentId,
    Status,
    TriggeredBy,
    Result,
    Error,
    StartedAt,
    CompletedAt,
}

#[derive(Iden)]
pub enum Tasks {
    Table,
    Id,
    RunId,
    Subject,
    Status,
    AssignedTo,
    Output,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
pub enum Decisions {
    Table,
    Id,
    Agent,
    Context,
    Decision,
    Reasoning,
    Tags,
    RunId,
    CreatedAt,
}

#[derive(Iden)]
pub enum Artifacts {
    Table,
    Id,
    Name,
    ContentType,
    Size,
    RunId,
    CreatedAt,
}
```

Note: sea-query's `#[derive(Iden)]` generates snake_case table/column names from the enum variant names. Verify the generated names match the actual SQL column names (e.g., `EmbeddingModel` → `embedding_model`, `ContentType` → `content_type`).

- [ ] **Step 2: Export from `db/mod.rs`**

Add `pub mod tables;`

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p overseer`

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/db/tables.rs src/overseer/src/db/mod.rs
git commit -m "refactor(db): define sea-query table/column enums"
```

---

### Task 9: Rewrite artifact CRUD with sea-query

**Files:**
- Modify: `src/overseer/src/db/artifacts.rs`

Start with artifacts — simplest module (3 functions, no vector search). This establishes the sea-query pattern for remaining modules.

- [ ] **Step 1: Rewrite `insert_artifact` using sea-query**

```rust
use sea_query::{Query, SqliteQueryBuilder};
use sea_query_binder::SqlxBinder;
use super::tables::Artifacts;

pub async fn insert_artifact(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    content_type: &str,
    size: i64,
    run_id: Option<&str>,
) -> Result<ArtifactMetadata> {
    let (sql, values) = Query::insert()
        .into_table(Artifacts::Table)
        .columns([
            Artifacts::Id,
            Artifacts::Name,
            Artifacts::ContentType,
            Artifacts::Size,
            Artifacts::RunId,
        ])
        .values_panic([
            id.into(),
            name.into(),
            content_type.into(),
            size.into(),
            run_id.map(|s| s.to_string()).into(),
        ])
        .returning(
            Query::returning().columns([
                Artifacts::Id,
                Artifacts::Name,
                Artifacts::ContentType,
                Artifacts::Size,
                Artifacts::RunId,
                Artifacts::CreatedAt,
            ]),
        )
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_one(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row_to_artifact(&row))
}
```

- [ ] **Step 2: Rewrite `get_artifact` and `list_artifacts` similarly**

For `get_artifact`:
```rust
let (sql, values) = Query::select()
    .columns([
        Artifacts::Id, Artifacts::Name, Artifacts::ContentType,
        Artifacts::Size, Artifacts::RunId, Artifacts::CreatedAt,
    ])
    .from(Artifacts::Table)
    .and_where(sea_query::Expr::col(Artifacts::Id).eq(id))
    .build_sqlx(SqliteQueryBuilder);
```

For `list_artifacts` with the optional `run_id` filter:
```rust
let mut query = Query::select();
query.columns([...]).from(Artifacts::Table);
if let Some(rid) = run_id {
    query.and_where(sea_query::Expr::col(Artifacts::RunId).eq(rid));
}
query.order_by(Artifacts::CreatedAt, sea_query::Order::Asc);
let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
```

- [ ] **Step 3: Run artifact tests**

Run: `cargo test -p overseer db::artifacts`
Expected: all artifact tests pass

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/db/artifacts.rs
git commit -m "refactor(db): rewrite artifact queries with sea-query"
```

---

### Task 10: Rewrite decisions CRUD with sea-query

**Files:**
- Modify: `src/overseer/src/db/decisions.rs`

- [ ] **Step 1: Rewrite `log_decision`, `get_decision`, `query_decisions` with sea-query**

Same pattern as artifacts. The `query_decisions` function's `WHERE (?1 IS NULL OR agent = ?1)` pattern becomes:

```rust
if let Some(agent) = agent {
    query.and_where(Expr::col(Decisions::Agent).eq(agent));
}
```

Tags filtering remains as post-query Rust logic (no change to that pattern).

- [ ] **Step 2: Run decisions tests**

Run: `cargo test -p overseer db::decisions`
Expected: all pass

- [ ] **Step 3: Commit**

```bash
git add src/overseer/src/db/decisions.rs
git commit -m "refactor(db): rewrite decisions queries with sea-query"
```

---

### Task 11: Rewrite jobs CRUD with sea-query

**Files:**
- Modify: `src/overseer/src/db/jobs.rs`

- [ ] **Step 1: Rewrite all 11 job functions with sea-query**

The trickiest function is `update_job_run` with its conditional `completed_at = datetime('now')` logic. Use `Expr::cust("datetime('now')")` for the SQLite backend:

```rust
let mut update = Query::update();
update.table(JobRuns::Table);

if let Some(s) = status {
    update.value(JobRuns::Status, s);
    if ["completed", "failed", "cancelled"].contains(&s) {
        update.value(JobRuns::CompletedAt, Expr::cust("datetime('now')"));
    }
}
if let Some(r) = &result_json {
    update.value(JobRuns::Result, r.as_str());
}
if let Some(e) = error {
    update.value(JobRuns::Error, e);
}
update.and_where(Expr::col(JobRuns::Id).eq(id));
```

For `start_job_run`, the `started_at = datetime('now')` becomes:
```rust
.values_panic([
    id.into(), definition_id.into(), parent_id_val,
    "running".into(), triggered_by.into(),
    sea_query::SimpleExpr::Custom("datetime('now')".into()),
])
```

Note: sea-query's `values_panic` takes `Into<SimpleExpr>`, so custom SQL expressions may need `Expr::cust()`.

- [ ] **Step 2: Run jobs tests**

Run: `cargo test -p overseer db::jobs`
Expected: all pass

- [ ] **Step 3: Commit**

```bash
git add src/overseer/src/db/jobs.rs
git commit -m "refactor(db): rewrite jobs queries with sea-query"
```

---

### Task 12: Rewrite memory metadata queries with sea-query

**Files:**
- Modify: `src/overseer/src/db/memory.rs`

- [ ] **Step 1: Rewrite `insert_memory`, `get_memory`, `delete_memory`, `insert_memory_link` with sea-query**

Only the metadata CRUD gets sea-query treatment. The vector embedding insert/search (`INSERT INTO memory_embeddings_{provider}`, `WHERE embedding MATCH ?1 AND k = ?2`) stays as raw SQL — these are sqlite-vec specific and will differ completely for pgvector.

- [ ] **Step 2: Run memory tests**

Run: `cargo test -p overseer db::memory`
Expected: all pass

- [ ] **Step 3: Run full test suite**

Run: `cargo test -p overseer`
Expected: all pass

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/db/memory.rs
git commit -m "refactor(db): rewrite memory metadata queries with sea-query"
```

---

## Phase 3: URL-based Configuration

### Task 13: Update StorageConfig to URL-based fields

**Files:**
- Modify: `src/overseer/src/config.rs`
- Modify: `overseer.toml`

- [ ] **Step 1: Update `StorageConfig` struct**

```rust
#[derive(Debug, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_database_url")]
    pub database_url: String,
    #[serde(default = "default_artifact_url")]
    pub artifact_url: String,
    #[serde(default)]
    pub s3: Option<S3Config>,
}

#[derive(Debug, Deserialize, Default)]
pub struct S3Config {
    pub region: Option<String>,
    pub endpoint: Option<String>,
    pub access_key_env: Option<String>,
    pub secret_key_env: Option<String>,
}

fn default_database_url() -> String {
    "sqlite://data/overseer.db".to_string()
}

fn default_artifact_url() -> String {
    "file://data/artifacts".to_string()
}
```

- [ ] **Step 2: Update all config tests**

Update assertions from `database_path`/`artifact_path` to `database_url`/`artifact_url`. Update the "full toml" test data.

- [ ] **Step 3: Update `overseer.toml`**

```toml
[storage]
database_url = "sqlite://data/overseer.db"
artifact_url = "file://data/artifacts"
```

- [ ] **Step 4: Run config tests**

Run: `cargo test -p overseer config`
Expected: all pass

- [ ] **Step 5: Commit**

```bash
git add src/overseer/src/config.rs overseer.toml
git commit -m "config: switch StorageConfig to URL-based fields"
```

---

### Task 14: Add `open_from_url` factory and update main.rs

**Files:**
- Modify: `src/overseer/src/db/mod.rs`
- Modify: `src/overseer/src/main.rs`

- [ ] **Step 1: Add factory function to `db/mod.rs`**

```rust
pub async fn open_from_url(url: &str) -> std::result::Result<Arc<dyn Database>, OverseerError> {
    if url.starts_with("sqlite://") {
        let path = &url["sqlite://".len()..];
        Ok(Arc::new(SqliteDatabase::open(path).await?))
    } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        Err(OverseerError::Validation(
            "postgres support not yet implemented".to_string(),
        ))
    } else {
        Err(OverseerError::Validation(format!(
            "unsupported database URL scheme: {url}"
        )))
    }
}
```

- [ ] **Step 2: Update main.rs to use URL**

```rust
let db = db::open_from_url(&config.storage.database_url).await?;
tracing::info!("database opened: {}", config.storage.database_url);

// Extract artifact path from file:// URL for now
let artifact_path = if let Some(path) = config.storage.artifact_url.strip_prefix("file://") {
    PathBuf::from(path)
} else {
    anyhow::bail!("only file:// artifact URLs supported currently");
};
```

- [ ] **Step 3: Run full test suite and verify startup**

Run: `cargo test -p overseer && buck2 build root//src/overseer:overseer`

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/db/mod.rs src/overseer/src/main.rs
git commit -m "feat(db): add open_from_url factory, update main.rs for URL config"
```

---

## Phase 4: PostgreSQL Backend

### Task 15: Add PostgreSQL dependencies

**Files:**
- Modify: `src/overseer/Cargo.toml`
- Modify: `src/overseer/BUCK`

- [ ] **Step 1: Add sqlx postgres feature and pgvector crate**

```bash
cd src/overseer && cargo add pgvector --features sqlx
```

Update sqlx in Cargo.toml to add `postgres` feature: `features = ["runtime-tokio", "sqlite", "postgres", "uuid"]`

Update sea-query to add postgres backend: add `backend-postgres` feature.
Update sea-query-binder to add `sqlx-postgres` feature.

- [ ] **Step 2: Regenerate Buck2 targets**

```bash
./tools/buckify.sh
```

Update `src/overseer/BUCK` with new deps: `"//third-party:pgvector"` and any new transitive deps. May need fixups for `pgvector` or its deps.

- [ ] **Step 3: Verify build**

Run: `cargo check -p overseer`

- [ ] **Step 4: Commit**

```bash
git add src/overseer/Cargo.toml src/overseer/BUCK Cargo.lock third-party/
git commit -m "deps: add sqlx postgres feature and pgvector"
```

---

### Task 16: Create PostgreSQL migration

**Files:**
- Create: `src/overseer/migrations/postgres/0001_initial.sql`

- [ ] **Step 1: Write the initial PostgreSQL schema**

```sql
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS memories (
    id TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    source TEXT NOT NULL,
    tags JSONB NOT NULL DEFAULT '[]',
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS memory_links (
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    linked_id TEXT NOT NULL,
    linked_type TEXT NOT NULL CHECK (linked_type IN ('memory', 'decision')),
    relation_type TEXT NOT NULL,
    PRIMARY KEY (memory_id, linked_id)
);

CREATE TABLE IF NOT EXISTS memory_embeddings (
    id BIGSERIAL PRIMARY KEY,
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    embedding vector,
    UNIQUE(memory_id, provider)
);

CREATE INDEX IF NOT EXISTS idx_memory_embeddings_provider
    ON memory_embeddings (provider);

CREATE TABLE IF NOT EXISTS job_definitions (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    config JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS job_runs (
    id TEXT PRIMARY KEY,
    definition_id TEXT NOT NULL REFERENCES job_definitions(id),
    parent_id TEXT REFERENCES job_runs(id),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    triggered_by TEXT NOT NULL,
    result JSONB,
    error TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    run_id TEXT REFERENCES job_runs(id),
    subject TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'in_progress', 'completed', 'failed')),
    assigned_to TEXT,
    output JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS decisions (
    id TEXT PRIMARY KEY,
    agent TEXT NOT NULL,
    context TEXT NOT NULL,
    decision TEXT NOT NULL,
    reasoning TEXT NOT NULL DEFAULT '',
    tags JSONB NOT NULL DEFAULT '[]',
    run_id TEXT REFERENCES job_runs(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size BIGINT NOT NULL,
    run_id TEXT REFERENCES job_runs(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

- [ ] **Step 2: Commit**

```bash
git add src/overseer/migrations/
git commit -m "feat(db): add PostgreSQL initial migration with pgvector"
```

---

### Task 17: Implement `PostgresDatabase`

**Files:**
- Create: `src/overseer/src/db/postgres.rs`
- Modify: `src/overseer/src/db/mod.rs`

This is the largest single task. The Postgres backend implements all 26 `Database` trait methods using sea-query with `PostgresQueryBuilder` for CRUD, and raw SQL for vector search.

- [ ] **Step 1: Create `db/postgres.rs` with struct and constructor**

```rust
use async_trait::async_trait;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::error::{OverseerError, Result};
use super::models::*;
use super::trait_def::Database;

pub struct PostgresDatabase {
    pool: PgPool,
}

impl PostgresDatabase {
    pub async fn open(url: &str) -> std::result::Result<Self, OverseerError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(url)
            .await
            .map_err(OverseerError::Storage)?;

        // Run migrations
        sqlx::raw_sql(include_str!("../../migrations/postgres/0001_initial.sql"))
            .execute(&pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(Self { pool })
    }
}
```

- [ ] **Step 2: Implement CRUD methods using sea-query + PostgresQueryBuilder**

All CRUD methods follow the same pattern as the SQLite versions but with `PostgresQueryBuilder` instead of `SqliteQueryBuilder`. Key differences:

- Timestamp expressions: `now()` instead of `datetime('now')`
- JSON columns: Postgres JSONB — `sqlx::types::Json<Vec<String>>` for tags, but since our model uses `String`, handle conversion in `row_to_*` functions
- Row mapping uses `PgRow` instead of `SqliteRow`

Write `row_to_*` helper functions for `PgRow`:
```rust
fn row_to_memory(row: &sqlx::postgres::PgRow) -> Memory {
    use sqlx::Row;
    let tags: serde_json::Value = row.get("tags");
    let tags: Vec<String> = serde_json::from_value(tags).unwrap_or_default();
    Memory {
        id: row.get("id"),
        content: row.get("content"),
        embedding_model: row.get("embedding_model"),
        source: row.get("source"),
        tags,
        expires_at: row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("expires_at")
            .map(|dt| dt.to_rfc3339()),
        created_at: row.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
        updated_at: row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at").to_rfc3339(),
    }
}
```

- [ ] **Step 3: Implement vector search methods**

`create_embedding_table` — no-op for Postgres (the `memory_embeddings` table is created by migration with a generic `vector` column; we could add an IVFFlat index per provider if desired, but skip for now).

`insert_memory`:
```rust
async fn insert_memory(&self, provider_name: &str, content: &str, embedding: &[f32],
    embedding_model: &str, source: &str, tags: &[String], expires_at: Option<&str>,
) -> Result<Memory> {
    let id = uuid::Uuid::new_v4().to_string();
    let tags_json = serde_json::to_value(tags).map_err(|e| OverseerError::Internal(e.to_string()))?;

    let mut tx = self.pool.begin().await.map_err(OverseerError::Storage)?;

    sqlx::query(
        "INSERT INTO memories (id, content, embedding_model, source, tags, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6::timestamptz)"
    )
    .bind(&id).bind(content).bind(embedding_model).bind(source)
    .bind(&tags_json).bind(expires_at)
    .execute(&mut *tx).await.map_err(OverseerError::Storage)?;

    let vec = pgvector::Vector::from(embedding.to_vec());
    sqlx::query(
        "INSERT INTO memory_embeddings (memory_id, provider, embedding) VALUES ($1, $2, $3)"
    )
    .bind(&id).bind(provider_name).bind(&vec)
    .execute(&mut *tx).await.map_err(OverseerError::Storage)?;

    tx.commit().await.map_err(OverseerError::Storage)?;

    self.get_memory(&id).await
}
```

`search_memories`:
```rust
async fn search_memories(&self, provider_name: &str, query_embedding: &[f32],
    tags_filter: Option<&[String]>, limit: usize,
) -> Result<Vec<MemorySearchResult>> {
    let vec = pgvector::Vector::from(query_embedding.to_vec());
    let fetch_limit = (limit * 10).max(100) as i64;

    let rows = sqlx::query(
        "SELECT m.id, m.content, m.embedding_model, m.source, m.tags, m.expires_at, \
         m.created_at, m.updated_at, e.embedding <-> $1 AS distance \
         FROM memory_embeddings e \
         JOIN memories m ON m.id = e.memory_id \
         WHERE e.provider = $2 \
         ORDER BY e.embedding <-> $1 \
         LIMIT $3"
    )
    .bind(&vec).bind(provider_name).bind(fetch_limit)
    .fetch_all(&self.pool).await.map_err(OverseerError::Storage)?;

    let mut results: Vec<MemorySearchResult> = rows.iter().map(|row| {
        use sqlx::Row;
        let distance: f64 = row.get("distance");
        MemorySearchResult { memory: row_to_memory(row), distance }
    }).collect();

    if let Some(filter_tags) = tags_filter {
        if !filter_tags.is_empty() {
            results.retain(|r| filter_tags.iter().any(|ft| r.memory.tags.contains(ft)));
        }
    }
    results.truncate(limit);
    Ok(results)
}
```

`delete_memory`: Just delete from `memories` — the `ON DELETE CASCADE` handles embedding cleanup:
```rust
async fn delete_memory(&self, _provider_name: &str, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM memories WHERE id = $1")
        .bind(id).execute(&self.pool).await.map_err(OverseerError::Storage)?;
    Ok(())
}
```

- [ ] **Step 4: Export from `db/mod.rs`**

Add `pub mod postgres;` and `pub use postgres::PostgresDatabase;`

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p overseer`

- [ ] **Step 6: Commit**

```bash
git add src/overseer/src/db/postgres.rs src/overseer/src/db/mod.rs
git commit -m "feat(db): implement PostgresDatabase with pgvector support"
```

---

### Task 18: Wire Postgres into `open_from_url`

**Files:**
- Modify: `src/overseer/src/db/mod.rs`

- [ ] **Step 1: Update the factory**

```rust
pub async fn open_from_url(url: &str) -> std::result::Result<Arc<dyn Database>, OverseerError> {
    if url.starts_with("sqlite://") {
        let path = &url["sqlite://".len()..];
        Ok(Arc::new(SqliteDatabase::open(path).await?))
    } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        Ok(Arc::new(PostgresDatabase::open(url).await?))
    } else {
        Err(OverseerError::Validation(format!(
            "unsupported database URL scheme: {url}"
        )))
    }
}
```

- [ ] **Step 2: Verify build**

Run: `cargo check -p overseer`

- [ ] **Step 3: Commit**

```bash
git add src/overseer/src/db/mod.rs
git commit -m "feat(db): wire PostgresDatabase into open_from_url"
```

---

### Task 19: Add PostgreSQL integration tests

**Files:**
- Create: `src/overseer/tests/postgres_integration.rs` (or `src/overseer/src/db/postgres.rs` test module)

- [ ] **Step 1: Write integration tests gated behind env var**

Tests only run when `TEST_DATABASE_URL` is set to a Postgres URL:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    async fn get_test_db() -> Option<PostgresDatabase> {
        let url = env::var("TEST_DATABASE_URL").ok()?;
        if !url.starts_with("postgres") { return None; }
        Some(PostgresDatabase::open(&url).await.expect("postgres connects"))
    }

    #[tokio::test]
    async fn test_postgres_memory_crud() {
        let Some(db) = get_test_db().await else { return };
        db.create_embedding_table("test", 384).await.unwrap();
        let mem = db.insert_memory("test", "hello", &vec![0.1f32; 384], "test", "test", &[], None)
            .await.unwrap();
        assert_eq!(mem.content, "hello");
        let fetched = db.get_memory(&mem.id).await.unwrap();
        assert_eq!(fetched.id, mem.id);
        db.delete_memory("test", &mem.id).await.unwrap();
    }

    // ... similar tests for jobs, decisions, artifacts, vector search
}
```

- [ ] **Step 2: Test with a local Postgres instance**

Run: `TEST_DATABASE_URL=postgres://localhost/overseer_test cargo test -p overseer postgres`

- [ ] **Step 3: Commit**

```bash
git add src/overseer/src/db/postgres.rs
git commit -m "test: add PostgreSQL integration tests"
```

---

## Phase 5: Object Store for Artifacts

### Task 20: Add object_store dependency

**Files:**
- Modify: `src/overseer/Cargo.toml`
- Modify: `src/overseer/BUCK`
- Modify: `src/overseer/src/error.rs`

- [ ] **Step 1: Add object_store crate**

```bash
cd src/overseer && cargo add object_store --features aws
```

- [ ] **Step 2: Regenerate Buck2 targets**

```bash
./tools/buckify.sh
```

This crate has many transitive deps (hyper, ring, rustls, etc.). Check for needed fixups in `third-party/fixups/`. The project already has `ring` and `rustls` fixups from `ureq`, so transitive deps may already be covered.

Add `"//third-party:object_store"` to BUCK deps.

- [ ] **Step 3: Add `ObjectStore` error variant**

In `error.rs`, add:
```rust
#[error("object store error: {0}")]
ObjectStore(String),
```

Add `From<object_store::Error>`:
```rust
impl From<object_store::Error> for OverseerError {
    fn from(e: object_store::Error) -> Self {
        OverseerError::ObjectStore(e.to_string())
    }
}
```

Update `IntoResponse` impl:
```rust
OverseerError::ObjectStore(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
```

- [ ] **Step 4: Verify build**

Run: `cargo check -p overseer`

- [ ] **Step 5: Commit**

```bash
git add src/overseer/Cargo.toml src/overseer/BUCK Cargo.lock third-party/ src/overseer/src/error.rs
git commit -m "deps: add object_store crate with S3 support"
```

---

### Task 21: Create object store factory

**Files:**
- Create: `src/overseer/src/storage.rs`
- Modify: `src/overseer/src/main.rs` (add `mod storage;`)

- [ ] **Step 1: Create `storage.rs`**

```rust
use std::sync::Arc;

use object_store::ObjectStore;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;

use crate::config::StorageConfig;
use crate::error::{OverseerError, Result};

pub fn create_object_store(config: &StorageConfig) -> Result<Arc<dyn ObjectStore>> {
    let url = &config.artifact_url;
    if let Some(path) = url.strip_prefix("file://") {
        let store = LocalFileSystem::new_with_prefix(path)
            .map_err(|e| OverseerError::ObjectStore(e.to_string()))?;
        Ok(Arc::new(store))
    } else if url.starts_with("s3://") {
        let mut builder = object_store::aws::AmazonS3Builder::from_env()
            .with_url(url);
        if let Some(s3) = &config.s3 {
            if let Some(region) = &s3.region {
                builder = builder.with_region(region);
            }
            if let Some(endpoint) = &s3.endpoint {
                builder = builder.with_endpoint(endpoint);
            }
            if let Some(key_env) = &s3.access_key_env {
                if let Ok(key) = std::env::var(key_env) {
                    builder = builder.with_access_key_id(key);
                }
            }
            if let Some(secret_env) = &s3.secret_key_env {
                if let Ok(secret) = std::env::var(secret_env) {
                    builder = builder.with_secret_access_key(secret);
                }
            }
        }
        let store = builder.build()
            .map_err(|e| OverseerError::ObjectStore(e.to_string()))?;
        Ok(Arc::new(store))
    } else {
        Err(OverseerError::Validation(format!(
            "unsupported artifact_url scheme: {url}"
        )))
    }
}

pub fn create_in_memory_store() -> Arc<dyn ObjectStore> {
    Arc::new(InMemory::new())
}
```

- [ ] **Step 2: Add `mod storage;` to main.rs**

- [ ] **Step 3: Commit**

```bash
git add src/overseer/src/storage.rs src/overseer/src/main.rs
git commit -m "feat(storage): add object store factory with file:// and s3:// support"
```

---

### Task 22: Update ArtifactService to use ObjectStore

**Files:**
- Modify: `src/overseer/src/services/artifacts.rs`
- Modify: `src/overseer/src/services/mod.rs`
- Modify: `src/overseer/src/main.rs`

- [ ] **Step 1: Update `ArtifactService`**

```rust
use std::sync::Arc;

use object_store::ObjectStore;
use object_store::path::Path as ObjectPath;
use object_store::PutPayload;

use crate::db::Database;
use crate::db::models::ArtifactMetadata;
use crate::error::{OverseerError, Result};

pub struct ArtifactService {
    db: Arc<dyn Database>,
    store: Arc<dyn ObjectStore>,
}

impl ArtifactService {
    pub fn new(db: Arc<dyn Database>, store: Arc<dyn ObjectStore>) -> Self {
        Self { db, store }
    }

    pub async fn store(
        &self,
        name: &str,
        content_type: &str,
        data: &[u8],
        run_id: Option<&str>,
    ) -> Result<ArtifactMetadata> {
        let id = uuid::Uuid::new_v4().to_string();
        let path = ObjectPath::from(format!("artifacts/{id}"));

        // Write blob first — if this fails, no orphaned DB row
        self.store
            .put(&path, PutPayload::from(data.to_vec()))
            .await
            .map_err(|e| OverseerError::ObjectStore(e.to_string()))?;

        self.db
            .insert_artifact(&id, name, content_type, data.len() as i64, run_id)
            .await
    }

    pub async fn get(&self, id: &str) -> Result<(ArtifactMetadata, Vec<u8>)> {
        let metadata = self
            .db
            .get_artifact(id)
            .await?
            .ok_or_else(|| OverseerError::NotFound(format!("artifact {id}")))?;

        let path = ObjectPath::from(format!("artifacts/{id}"));
        let result = self
            .store
            .get(&path)
            .await
            .map_err(|e| OverseerError::ObjectStore(e.to_string()))?;
        let data = result
            .bytes()
            .await
            .map_err(|e| OverseerError::ObjectStore(e.to_string()))?;

        Ok((metadata, data.to_vec()))
    }

    pub async fn list(&self, run_id: Option<&str>) -> Result<Vec<ArtifactMetadata>> {
        self.db.list_artifacts(run_id).await
    }
}
```

- [ ] **Step 2: Update `AppState::new`**

```rust
pub fn new(
    db: Arc<dyn Database>,
    registry: EmbeddingRegistry,
    store: Arc<dyn ObjectStore>,
) -> Self {
    Self {
        memory: memory::MemoryService::new(db.clone(), registry),
        jobs: jobs::JobService::new(db.clone()),
        decisions: decisions::DecisionService::new(db.clone()),
        artifacts: artifacts::ArtifactService::new(db, store),
    }
}
```

- [ ] **Step 3: Update main.rs**

Replace artifact path extraction with object store creation:
```rust
let store = storage::create_object_store(&config.storage)?;
tracing::info!("artifact store: {}", config.storage.artifact_url);

let state = Arc::new(AppState::new(db, registry, store));
```

- [ ] **Step 4: Update artifact service tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDatabase;
    use crate::storage::create_in_memory_store;

    #[tokio::test]
    async fn test_artifact_service_store_and_get() {
        let db = SqliteDatabase::open_in_memory_named("svc_artifacts_test_store")
            .await.expect("db opens");
        let store = create_in_memory_store();
        let svc = ArtifactService::new(Arc::new(db), store);

        let data = b"hello artifact world";
        let meta = svc.store("hello.txt", "text/plain", data, None).await.expect("store succeeds");

        assert_eq!(meta.name, "hello.txt");
        assert_eq!(meta.content_type, "text/plain");
        assert_eq!(meta.size, data.len() as i64);

        let (fetched_meta, fetched_data) = svc.get(&meta.id).await.expect("get succeeds");
        assert_eq!(fetched_meta.id, meta.id);
        assert_eq!(fetched_data, data);
    }

    #[tokio::test]
    async fn test_artifact_service_list() {
        let db = SqliteDatabase::open_in_memory_named("svc_artifacts_test_list")
            .await.expect("db opens");
        let store = create_in_memory_store();
        let svc = ArtifactService::new(Arc::new(db), store);

        svc.store("a.bin", "application/octet-stream", b"aaa", None).await.expect("store a");
        svc.store("b.bin", "application/octet-stream", b"bbb", None).await.expect("store b");

        let all = svc.list(None).await.expect("list all");
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_artifact_service_get_not_found() {
        let db = SqliteDatabase::open_in_memory_named("svc_artifacts_test_notfound")
            .await.expect("db opens");
        let store = create_in_memory_store();
        let svc = ArtifactService::new(Arc::new(db), store);

        let result = svc.get("nonexistent-id").await;
        assert!(matches!(result, Err(OverseerError::NotFound(_))));
    }
}
```

- [ ] **Step 5: Run all tests**

Run: `cargo test -p overseer`
Expected: all pass

- [ ] **Step 6: Verify Buck2 build**

Run: `buck2 build root//src/overseer:overseer`

- [ ] **Step 7: Commit**

```bash
git add src/overseer/src/services/ src/overseer/src/main.rs
git commit -m "feat(artifacts): replace filesystem with ObjectStore"
```

---

## Phase 6: Cleanup and Documentation

### Task 23: Update CLAUDE.md files and overseer docs

**Files:**
- Modify: `CLAUDE.md`
- Modify: `src/overseer/CLAUDE.md`

- [ ] **Step 1: Update root CLAUDE.md**

Add Postgres and object store info to the Components/Overseer section. Update the StorageConfig description. Add new dependency workflow notes for object_store.

- [ ] **Step 2: Update `src/overseer/CLAUDE.md`**

Update architecture diagram, module table, key patterns, configuration docs to reflect the new `Database` trait, Postgres backend, and object store.

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md src/overseer/CLAUDE.md
git commit -m "docs: update CLAUDE.md for postgres and object store support"
```

---

## Verification

After all phases:

1. `cargo test -p overseer` — all unit tests pass (SQLite path)
2. `buck2 build root//src/overseer:overseer` — hermetic build works
3. `buck2 run root//src/overseer:overseer` with `database_url = "sqlite://data/overseer.db"` — existing behavior preserved
4. `buck2 run root//src/overseer:overseer` with `database_url = "postgres://..."` — connects, creates schema, CRUD works (manual test with local Postgres)
5. `buck2 run root//src/overseer:overseer` with `artifact_url = "file://data/artifacts"` — existing behavior preserved
6. S3 testing — manual with MinIO or AWS S3 endpoint
7. `buck2 run root//tools:prek -- run --all-files` — pre-commit checks pass
