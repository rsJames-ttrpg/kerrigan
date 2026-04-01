# Multi-Embedder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Support multiple named embedding providers with per-provider vec0 tables, configurable via TOML, with Voyage AI as the first real provider.

**Architecture:** Config defines named providers (stub, voyage). On startup, each gets a vec0 table (`memory_embeddings_{name}`). An `EmbeddingRegistry` holds all providers; the default is used for store/recall. The `EmbeddingProvider` trait becomes async to support HTTP calls.

**Tech Stack:** Rust, sqlx, sqlite-vec, reqwest (for HTTP embedding calls), serde, tokio

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `src/overseer/src/config.rs` | Modify | New `EmbeddingConfig` with named providers map |
| `src/overseer/src/embedding/mod.rs` | Modify | Async trait + `EmbeddingRegistry` |
| `src/overseer/src/embedding/stub.rs` | Modify | Adapt to async trait |
| `src/overseer/src/embedding/voyage.rs` | Create | Voyage AI HTTP provider |
| `src/overseer/src/db/mod.rs` | Modify | Dynamic vec0 table creation, remove hardcoded table |
| `src/overseer/src/db/schema.sql` | Modify | Remove `memory_embeddings` vec0 table |
| `src/overseer/src/db/memory.rs` | Modify | `insert_memory`/`search_memories`/`delete_memory` take provider name |
| `src/overseer/src/services/memory.rs` | Modify | Use `EmbeddingRegistry` instead of single provider |
| `src/overseer/src/services/mod.rs` | Modify | `AppState` takes registry |
| `src/overseer/src/main.rs` | Modify | Build registry from config, pass to AppState |
| `src/overseer/src/mcp/mod.rs` | No change | Already delegates to service layer |
| `src/overseer/src/api/memory.rs` | No change | Already delegates to service layer |
| `src/overseer/Cargo.toml` | Modify | Add `reqwest` dependency |
| `src/overseer/BUCK` | Modify | Add `reqwest` to deps |

---

### Task 1: Update Config for Named Providers

**Files:**
- Modify: `src/overseer/src/config.rs`

- [ ] **Step 1: Write failing tests for new config shape**

Add to the existing `#[cfg(test)] mod tests` in `config.rs`:

```rust
#[test]
fn test_multi_provider_config() {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    write!(
        f,
        r#"
[embedding]
default = "voyage"

[embedding.providers.stub]
source = "stub"
dimensions = 384

[embedding.providers.voyage]
source = "voyage"
model = "voyage-3-lite"
dimensions = 512
api_key_env = "VOYAGE_API_KEY"
"#
    )
    .unwrap();
    let config = Config::load(f.path()).expect("should parse");
    assert_eq!(config.embedding.default, "voyage");
    assert_eq!(config.embedding.providers.len(), 2);

    let stub = &config.embedding.providers["stub"];
    assert_eq!(stub.source, "stub");
    assert_eq!(stub.dimensions, 384);

    let voyage = &config.embedding.providers["voyage"];
    assert_eq!(voyage.source, "voyage");
    assert_eq!(voyage.model.as_deref(), Some("voyage-3-lite"));
    assert_eq!(voyage.dimensions, 512);
    assert_eq!(voyage.api_key_env.as_deref(), Some("VOYAGE_API_KEY"));
}

#[test]
fn test_default_config_has_stub_provider() {
    let config = Config::load(std::path::Path::new("nonexistent.toml"))
        .expect("should fall back to defaults");
    assert_eq!(config.embedding.default, "stub");
    assert_eq!(config.embedding.providers.len(), 1);
    assert!(config.embedding.providers.contains_key("stub"));
}

#[test]
fn test_invalid_provider_name_rejected() {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    write!(
        f,
        r#"
[embedding]
default = "bad-name"

[embedding.providers.bad-name]
source = "stub"
dimensions = 384
"#
    )
    .unwrap();
    let config = Config::load(f.path()).expect("should parse");
    let result = config.embedding.validate();
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test config::tests -p overseer -- --nocapture`
Expected: compilation errors — `EmbeddingConfig` doesn't have `default` or `providers` fields yet.

- [ ] **Step 3: Implement new config structs**

Replace the existing `EmbeddingConfig` and its `Default` impl in `config.rs`:

```rust
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct EmbeddingConfig {
    #[serde(default = "default_embedding_default")]
    pub default: String,
    #[serde(default = "default_providers")]
    pub providers: HashMap<String, ProviderConfig>,
}

#[derive(Debug, Deserialize)]
pub struct ProviderConfig {
    pub source: String,
    pub dimensions: usize,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            default: default_embedding_default(),
            providers: default_providers(),
        }
    }
}

fn default_embedding_default() -> String {
    "stub".to_string()
}

fn default_providers() -> HashMap<String, ProviderConfig> {
    let mut m = HashMap::new();
    m.insert(
        "stub".to_string(),
        ProviderConfig {
            source: "stub".to_string(),
            dimensions: 384,
            model: None,
            api_key_env: None,
        },
    );
    m
}

impl EmbeddingConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        let name_re = regex_lite::Regex::new(r"^[a-z0-9_]+$").unwrap();
        for name in self.providers.keys() {
            anyhow::ensure!(
                name_re.is_match(name),
                "provider name '{name}' must match [a-z0-9_]+"
            );
        }
        anyhow::ensure!(
            self.providers.contains_key(&self.default),
            "default provider '{}' not found in providers",
            self.default
        );
        Ok(())
    }
}
```

Remove the old `default_provider()` function and old `EmbeddingConfig` struct/impl.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test config::tests -p overseer -- --nocapture`
Expected: all config tests pass, including the three new ones.

- [ ] **Step 5: Commit**

```bash
git add src/overseer/src/config.rs
git commit -m "feat(overseer): multi-provider embedding config with validation"
```

---

### Task 2: Add `regex-lite` dependency

**Files:**
- Modify: `src/overseer/Cargo.toml`
- Modify: `src/overseer/BUCK`

- [ ] **Step 1: Add the crate**

```bash
cd src/overseer && cargo add regex-lite
```

- [ ] **Step 2: Buckify**

```bash
./tools/buckify.sh
```

- [ ] **Step 3: Add to BUCK deps**

Add `"//third-party:regex-lite"` to `OVERSEER_DEPS` in `src/overseer/BUCK`.

- [ ] **Step 4: Verify build**

```bash
buck2 build root//src/overseer:overseer
```
Expected: BUILD SUCCEEDED

- [ ] **Step 5: Commit**

```bash
git add src/overseer/Cargo.toml Cargo.lock src/overseer/BUCK third-party/BUCK
git commit -m "chore: add regex-lite dependency"
```

---

### Task 3: Make EmbeddingProvider Trait Async

**Files:**
- Modify: `src/overseer/src/embedding/mod.rs`
- Modify: `src/overseer/src/embedding/stub.rs`
- Modify: `src/overseer/src/services/memory.rs`

- [ ] **Step 1: Update the trait to async**

In `embedding/mod.rs`:

```rust
pub mod stub;

use crate::error::Result;

pub trait EmbeddingProvider: Send + Sync {
    fn embed(&self, text: &str) -> impl std::future::Future<Output = Result<Vec<f32>>> + Send;
    fn model_name(&self) -> &str;
    fn dimensions(&self) -> usize;
}
```

- [ ] **Step 2: Update StubEmbedding to async**

In `embedding/stub.rs`, change the `embed` method:

```rust
impl EmbeddingProvider for StubEmbedding {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.0; self.dims])
    }

    fn model_name(&self) -> &str {
        "stub"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }
}
```

- [ ] **Step 3: Update MemoryService to await embed calls**

In `services/memory.rs`, add `.await` to both `embed` calls:

```rust
// In store():
let embedding = self.embedder.embed(content).await?;

// In recall():
let embedding = self.embedder.embed(query).await?;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p overseer`
Expected: all 43 tests pass (stub tests need `#[tokio::test]` now).

- [ ] **Step 5: Update stub tests to async**

In `embedding/stub.rs`, change tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stub_returns_zero_vector() {
        let stub = StubEmbedding::new(384);
        let embedding = stub.embed("anything").await.unwrap();
        assert_eq!(embedding.len(), 384);
        assert!(embedding.iter().all(|&v| v == 0.0));
    }

    #[tokio::test]
    async fn test_stub_model_name() {
        let stub = StubEmbedding::new(384);
        assert_eq!(stub.model_name(), "stub");
        assert_eq!(stub.dimensions(), 384);
    }
}
```

- [ ] **Step 6: Run tests again**

Run: `cargo test -p overseer`
Expected: all 43 tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/overseer/src/embedding/ src/overseer/src/services/memory.rs
git commit -m "refactor(overseer): make EmbeddingProvider trait async"
```

---

### Task 4: Build EmbeddingRegistry

**Files:**
- Modify: `src/overseer/src/embedding/mod.rs`

- [ ] **Step 1: Write failing tests for the registry**

Add to `embedding/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use stub::StubEmbedding;

    fn make_registry() -> EmbeddingRegistry {
        let mut providers: HashMap<String, Arc<dyn EmbeddingProvider>> = HashMap::new();
        providers.insert("stub".into(), Arc::new(StubEmbedding::new(384)));
        providers.insert("other".into(), Arc::new(StubEmbedding::new(768)));
        EmbeddingRegistry::new(providers, "stub".into()).unwrap()
    }

    #[test]
    fn test_registry_get_default() {
        let reg = make_registry();
        assert_eq!(reg.default_name(), "stub");
        assert_eq!(reg.get_default().dimensions(), 384);
    }

    #[test]
    fn test_registry_get_by_name() {
        let reg = make_registry();
        assert!(reg.get("stub").is_some());
        assert!(reg.get("other").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_invalid_default() {
        let providers: HashMap<String, Arc<dyn EmbeddingProvider>> = HashMap::new();
        let result = EmbeddingRegistry::new(providers, "missing".into());
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test embedding::tests -p overseer`
Expected: compilation errors — `EmbeddingRegistry` doesn't exist.

- [ ] **Step 3: Implement EmbeddingRegistry**

Add to `embedding/mod.rs`:

```rust
use std::collections::HashMap;
use std::sync::Arc;

pub struct EmbeddingRegistry {
    providers: HashMap<String, Arc<dyn EmbeddingProvider>>,
    default: String,
}

impl EmbeddingRegistry {
    pub fn new(
        providers: HashMap<String, Arc<dyn EmbeddingProvider>>,
        default: String,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            providers.contains_key(&default),
            "default provider '{default}' not found in registry"
        );
        Ok(Self { providers, default })
    }

    pub fn get_default(&self) -> &Arc<dyn EmbeddingProvider> {
        &self.providers[&self.default]
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn EmbeddingProvider>> {
        self.providers.get(name)
    }

    pub fn default_name(&self) -> &str {
        &self.default
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test embedding::tests -p overseer`
Expected: all 3 registry tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/overseer/src/embedding/mod.rs
git commit -m "feat(overseer): add EmbeddingRegistry for named provider lookup"
```

---

### Task 5: Dynamic vec0 Table Creation

**Files:**
- Modify: `src/overseer/src/db/schema.sql`
- Modify: `src/overseer/src/db/mod.rs`

- [ ] **Step 1: Remove hardcoded vec0 table from schema.sql**

Remove these lines from `schema.sql`:

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS memory_embeddings USING vec0(
    embedding float[384]
);
```

- [ ] **Step 2: Write failing test for dynamic table creation**

Add to `db/mod.rs` tests:

```rust
#[tokio::test]
async fn test_create_embedding_table() {
    let pool = open_in_memory_named("db_test_create_emb_table").await.expect("pool opens");

    create_embedding_table(&pool, "test_provider", 512).await.expect("create table");

    // Verify the table exists
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
    let pool = open_in_memory_named("db_test_emb_idempotent").await.expect("pool opens");

    create_embedding_table(&pool, "dup", 384).await.expect("first create");
    create_embedding_table(&pool, "dup", 384).await.expect("second create should be ok");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test db::tests -p overseer`
Expected: compilation error — `create_embedding_table` doesn't exist.

- [ ] **Step 4: Implement `create_embedding_table`**

Add to `db/mod.rs`:

```rust
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
```

- [ ] **Step 5: Run tests**

Run: `cargo test db::tests -p overseer`
Expected: all db tests pass. The `test_vec0_extension_loaded` test may need updating since the old `memory_embeddings` table no longer exists.

- [ ] **Step 6: Fix `test_vec0_extension_loaded`**

Replace the existing test to verify dynamic creation works instead of checking for the hardcoded table:

```rust
#[tokio::test]
async fn test_vec0_extension_loaded() {
    let pool = open_in_memory_named("db_test_vec0_loaded").await.expect("pool opens");

    // Verify we can create a vec0 table (proves the extension is loaded)
    create_embedding_table(&pool, "vec0test", 128).await.expect("vec0 table creation should work");

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'memory_embeddings_vec0test'",
    )
    .fetch_all(&pool)
    .await
    .expect("query");
    assert!(!tables.is_empty(), "vec0 virtual table should exist");
}
```

- [ ] **Step 7: Run all tests**

Run: `cargo test -p overseer`
Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/overseer/src/db/mod.rs src/overseer/src/db/schema.sql
git commit -m "feat(overseer): dynamic vec0 table creation per embedding provider"
```

---

### Task 6: Update DB Memory Functions for Named Tables

**Files:**
- Modify: `src/overseer/src/db/memory.rs`

- [ ] **Step 1: Update `insert_memory` to take provider_name**

Change the function signature to add `provider_name: &str` and use it in the embedding insert:

```rust
pub async fn insert_memory(
    pool: &SqlitePool,
    provider_name: &str,
    content: &str,
    embedding: &[f32],
    embedding_model: &str,
    source: &str,
    tags: &[String],
    expires_at: Option<&str>,
) -> Result<Memory> {
```

Change the embedding insert SQL from:

```rust
sqlx::query("INSERT INTO memory_embeddings (rowid, embedding) VALUES (?1, ?2)")
```

to:

```rust
let emb_sql = format!(
    "INSERT INTO memory_embeddings_{provider_name} (rowid, embedding) VALUES (?1, ?2)"
);
sqlx::query(&emb_sql)
```

- [ ] **Step 2: Update `search_memories` to take provider_name**

Change signature to add `provider_name: &str`. Update the SQL:

```rust
let sql = format!(
    "SELECT m.id, m.content, m.embedding_model, m.source, m.tags, m.expires_at, \
     m.created_at, m.updated_at, v.distance \
     FROM memory_embeddings_{provider_name} v \
     JOIN memories m ON m.rowid = v.rowid \
     WHERE v.embedding MATCH ?1 AND k = ?2 \
     ORDER BY v.distance"
);
let rows = sqlx::query(&sql)
```

- [ ] **Step 3: Update `delete_memory` to take provider_name**

Change signature to add `provider_name: &str`. Update the embedding cleanup:

```rust
if let Some(rowid) = rowid {
    let del_sql = format!(
        "DELETE FROM memory_embeddings_{provider_name} WHERE rowid = ?1"
    );
    let _ = sqlx::query(&del_sql)
        .bind(rowid)
        .execute(pool)
        .await;
}
```

- [ ] **Step 4: Update all tests to pass provider_name**

In all test calls, add `"stub"` as the first argument after `pool`. For example:

```rust
let memory = insert_memory(&pool, "stub", "hello world", &embedding, "stub", "unit-test", &tags, None)
    .await
    .expect("insert succeeds");
```

Each test also needs to create the vec0 table before inserting:

```rust
use crate::db::create_embedding_table;

// At the start of each test that inserts memories:
create_embedding_table(&pool, "stub", 384).await.expect("create table");
```

Update `search_memories` calls: `search_memories(&pool, "stub", &query, None, 3)`

Update `delete_memory` calls: `delete_memory(&pool, "stub", &memory.id)`

- [ ] **Step 5: Run tests**

Run: `cargo test db::memory -p overseer`
Expected: all memory db tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/overseer/src/db/memory.rs
git commit -m "feat(overseer): memory db functions use named provider tables"
```

---

### Task 7: Update MemoryService to Use Registry

**Files:**
- Modify: `src/overseer/src/services/memory.rs`
- Modify: `src/overseer/src/services/mod.rs`

- [ ] **Step 1: Update MemoryService**

Replace the contents of `services/memory.rs`:

```rust
use sqlx::SqlitePool;

use crate::db::memory as db;
use crate::embedding::EmbeddingRegistry;
use crate::error::Result;

pub struct MemoryService {
    pool: SqlitePool,
    registry: EmbeddingRegistry,
}

impl MemoryService {
    pub fn new(pool: SqlitePool, registry: EmbeddingRegistry) -> Self {
        Self { pool, registry }
    }

    pub async fn store(
        &self,
        content: &str,
        source: &str,
        tags: &[String],
        expires_at: Option<&str>,
    ) -> Result<db::Memory> {
        let provider = self.registry.get_default();
        let provider_name = self.registry.default_name();
        let embedding = provider.embed(content).await?;
        db::insert_memory(
            &self.pool,
            provider_name,
            content,
            &embedding,
            provider_name,
            source,
            tags,
            expires_at,
        )
        .await
    }

    pub async fn recall(
        &self,
        query: &str,
        tags_filter: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<db::MemorySearchResult>> {
        let provider = self.registry.get_default();
        let provider_name = self.registry.default_name();
        let embedding = provider.embed(query).await?;
        db::search_memories(&self.pool, provider_name, &embedding, tags_filter, limit).await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        // Look up the memory to find which provider table it's in
        let memory = db::get_memory(&self.pool, id).await?;
        db::delete_memory(&self.pool, &memory.embedding_model, id).await
    }
}
```

- [ ] **Step 2: Update AppState**

In `services/mod.rs`, change the import and `AppState::new`:

```rust
use crate::embedding::EmbeddingRegistry;

pub struct AppState {
    pub memory: memory::MemoryService,
    pub jobs: jobs::JobService,
    pub decisions: decisions::DecisionService,
    pub artifacts: artifacts::ArtifactService,
}

impl AppState {
    pub fn new(
        pool: SqlitePool,
        registry: EmbeddingRegistry,
        artifact_path: PathBuf,
    ) -> Self {
        Self {
            memory: memory::MemoryService::new(pool.clone(), registry),
            jobs: jobs::JobService::new(pool.clone()),
            decisions: decisions::DecisionService::new(pool.clone()),
            artifacts: artifacts::ArtifactService::new(pool, artifact_path),
        }
    }
}
```

- [ ] **Step 3: Update MemoryService tests**

In `services/memory.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_in_memory_named, create_embedding_table};
    use crate::embedding::stub::StubEmbedding;
    use std::collections::HashMap;
    use std::sync::Arc;

    async fn make_service() -> MemoryService {
        let pool = open_in_memory_named("svc_mem_test").await.expect("pool opens");
        create_embedding_table(&pool, "stub", 384).await.expect("create table");
        let mut providers: HashMap<String, Arc<dyn crate::embedding::EmbeddingProvider>> = HashMap::new();
        providers.insert("stub".into(), Arc::new(StubEmbedding::new(384)));
        let registry = EmbeddingRegistry::new(providers, "stub".into()).unwrap();
        MemoryService::new(pool, registry)
    }

    // ... existing tests unchanged (store, recall, delete all go through service)
}
```

Note: each test needs its own pool name to avoid interference. Update `make_service` to take a name parameter, or give each test a unique name.

- [ ] **Step 4: Run tests**

Run: `cargo test services::memory -p overseer`
Expected: all memory service tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/overseer/src/services/memory.rs src/overseer/src/services/mod.rs
git commit -m "feat(overseer): MemoryService uses EmbeddingRegistry"
```

---

### Task 8: Update main.rs to Build Registry from Config

**Files:**
- Modify: `src/overseer/src/main.rs`

- [ ] **Step 1: Replace embedder construction with registry**

In `main.rs`, replace:

```rust
let embedder: Arc<dyn embedding::EmbeddingProvider> = Arc::new(StubEmbedding::new(384));
tracing::info!("embedding provider: {}", embedder.model_name());

let state = Arc::new(AppState::new(
    pool,
    embedder,
    config.storage.artifact_path.clone(),
));
```

with:

```rust
config.embedding.validate()?;

let mut providers: std::collections::HashMap<String, Arc<dyn embedding::EmbeddingProvider>> =
    std::collections::HashMap::new();

for (name, provider_config) in &config.embedding.providers {
    let provider: Arc<dyn embedding::EmbeddingProvider> = match provider_config.source.as_str() {
        "stub" => Arc::new(StubEmbedding::new(provider_config.dimensions)),
        "voyage" => {
            let model = provider_config.model.as_deref()
                .expect("voyage provider requires 'model' in config");
            let api_key_env = provider_config.api_key_env.as_deref()
                .expect("voyage provider requires 'api_key_env' in config");
            let api_key = std::env::var(api_key_env)
                .unwrap_or_else(|_| panic!("env var '{api_key_env}' not set"));
            Arc::new(embedding::voyage::VoyageEmbedding::new(
                model.to_string(),
                api_key,
                provider_config.dimensions,
            ))
        }
        other => anyhow::bail!("unknown embedding source: {other}"),
    };
    tracing::info!("embedding provider '{name}': {} ({}d)", provider.model_name(), provider.dimensions());
    db::create_embedding_table(&pool, name, provider_config.dimensions).await?;
    providers.insert(name.clone(), provider);
}

let registry = embedding::EmbeddingRegistry::new(providers, config.embedding.default.clone())?;
tracing::info!("default embedding provider: {}", config.embedding.default);

let state = Arc::new(AppState::new(
    pool,
    registry,
    config.storage.artifact_path.clone(),
));
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p overseer`
Expected: compiles (VoyageEmbedding doesn't exist yet, so this will fail — that's Task 9).

- [ ] **Step 3: Commit (if it compiles, otherwise defer to after Task 9)**

```bash
git add src/overseer/src/main.rs
git commit -m "feat(overseer): build embedding registry from config on startup"
```

---

### Task 9: Implement Voyage AI Provider

**Files:**
- Create: `src/overseer/src/embedding/voyage.rs`
- Modify: `src/overseer/src/embedding/mod.rs`
- Modify: `src/overseer/Cargo.toml`
- Modify: `src/overseer/BUCK`

- [ ] **Step 1: Add reqwest dependency**

```bash
cd src/overseer && cargo add reqwest --features json
```

Then buckify:

```bash
cd /home/jackm/repos/kerrigan && cargo generate-lockfile && ./tools/buckify.sh
```

Add `"//third-party:reqwest"` to `OVERSEER_DEPS` in `src/overseer/BUCK`.

- [ ] **Step 2: Create `embedding/voyage.rs`**

```rust
use crate::embedding::EmbeddingProvider;
use crate::error::{OverseerError, Result};

pub struct VoyageEmbedding {
    model: String,
    api_key: String,
    dimensions: usize,
    client: reqwest::Client,
}

impl VoyageEmbedding {
    pub fn new(model: String, api_key: String, dimensions: usize) -> Self {
        Self {
            model,
            api_key,
            dimensions,
            client: reqwest::Client::new(),
        }
    }
}

impl EmbeddingProvider for VoyageEmbedding {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let body = serde_json::json!({
            "input": [text],
            "model": self.model,
        });

        let response = self
            .client
            .post("https://api.voyageai.com/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| OverseerError::Embedding(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(OverseerError::Embedding(format!(
                "voyage API returned {status}: {body}"
            )));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| OverseerError::Embedding(e.to_string()))?;

        let embedding: Vec<f32> = json["data"][0]["embedding"]
            .as_array()
            .ok_or_else(|| OverseerError::Embedding("unexpected response shape".into()))?
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect();

        if embedding.len() != self.dimensions {
            return Err(OverseerError::Embedding(format!(
                "dimension mismatch: expected {}, got {}",
                self.dimensions,
                embedding.len()
            )));
        }

        Ok(embedding)
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}
```

- [ ] **Step 3: Register the module**

In `embedding/mod.rs`, add:

```rust
pub mod voyage;
```

- [ ] **Step 4: Verify full build**

Run: `cargo check -p overseer`
Expected: compiles.

- [ ] **Step 5: Run all tests**

Run: `cargo test -p overseer`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/overseer/src/embedding/voyage.rs src/overseer/src/embedding/mod.rs src/overseer/Cargo.toml Cargo.lock src/overseer/BUCK third-party/BUCK
git commit -m "feat(overseer): add Voyage AI embedding provider"
```

---

### Task 10: Update Default Config File and Docs

**Files:**
- Modify: `overseer.toml`
- Modify: `src/overseer/CLAUDE.md`

- [ ] **Step 1: Update overseer.toml**

Replace the `[embedding]` section:

```toml
[embedding]
default = "stub"

[embedding.providers.stub]
source = "stub"
dimensions = 384

# Uncomment to use Voyage AI:
# [embedding.providers.voyage]
# source = "voyage"
# model = "voyage-3-lite"
# dimensions = 512
# api_key_env = "VOYAGE_API_KEY"
```

- [ ] **Step 2: Update CLAUDE.md**

Add a section about embedding configuration after "Key Patterns":

```markdown
## Embedding Providers

Named providers configured in `overseer.toml`. Each gets a `memory_embeddings_{name}` vec0 table.

Supported sources:
- `stub` — zero vectors for testing (no API key needed)
- `voyage` — Voyage AI REST API (needs `VOYAGE_API_KEY` env var)

To switch providers, change `[embedding] default = "new_provider"`. Old memories stay in their original table but won't appear in search until re-embedded.
```

- [ ] **Step 3: Run full test suite**

```bash
buck2 test root//src/overseer:overseer-test
```
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add overseer.toml src/overseer/CLAUDE.md
git commit -m "docs: update config and CLAUDE.md for multi-embedder support"
```

---

### Task 11: End-to-End Smoke Test

- [ ] **Step 1: Run with default config**

```bash
buck2 run root//src/overseer:overseer
```

Expected: starts up, logs show `embedding provider 'stub': stub (384d)`, HTTP server listening.

- [ ] **Step 2: Test memory store and recall via HTTP**

```bash
curl -s -X POST http://localhost:3100/memory -H 'Content-Type: application/json' \
  -d '{"content":"hello multi-embedder","source":"smoke-test","tags":["test"]}'

curl -s 'http://localhost:3100/memory/search?query=hello&limit=5'
```

Expected: store returns the memory JSON, search returns it in results.

- [ ] **Step 3: Verify vec0 table was created**

```bash
sqlite3 data/overseer.db ".tables"
```

Expected: `memory_embeddings_stub` appears (not the old `memory_embeddings`).

- [ ] **Step 4: All good — final commit if any loose changes**

```bash
buck2 test root//src/overseer:overseer-test
```
Expected: all tests pass.
