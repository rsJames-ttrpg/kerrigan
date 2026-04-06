---
title: Overseer Service
slug: overseer
description: Foundational layered monolith — HTTP + MCP over shared services, pluggable DB and object store
lastmod: 2026-04-06
tags: [overseer, api, database, mcp, service]
sources:
  - path: src/overseer/src/main.rs
    hash: f26703e8b7ca7fd6b0b88e784bf9aed68bd1a00663a5025eba5f9c0630af80d8
  - path: src/overseer/src/config.rs
    hash: 0c92d54253aa6139ccbfad0d401c1f669adba179c51d1c5f97864646829c5521
  - path: src/overseer/src/db/trait_def.rs
    hash: a38bf81d9938ce5883685e2223ecaa8d4dd134c60e551d6975bb4e2164ca222f
  - path: src/overseer/src/db/models.rs
    hash: 2b1b95c8f3e998fbf5c0bc42e206bcb48acafc6ec40faadc823c522f36ec3323
  - path: src/overseer/src/api/mod.rs
    hash: f1c944f484911d72ce0b6d83b3e58dbc5e61d4f14d0b8547b0432dfcb67feec2
  - path: src/overseer/src/mcp/mod.rs
    hash: 766bb95d7318151805b0a5807bdc9bbd0d1513bf46ce3caeebbd1fbbcbb5aff4
  - path: src/overseer/src/storage.rs
    hash: 769a7ef3d17df52c76b9e3b6a8862d3fcd9cdefccd2bb44fc2e60373c6e83155
  - path: src/overseer/src/error.rs
    hash: b37b233d32d5eebc2caf544a22a480e02e15dd8139076bd14f6d39f6ffea27d0
sections: [architecture, database-layer, service-layer, http-api, mcp-tools, storage, embedding, configuration, error-handling]
---

# Overseer Service

## Architecture

Overseer is a layered monolith with three tiers:

1. **Transport** — HTTP (axum) REST API + MCP (rmcp) via stdio or HTTP
2. **Service** — Business logic in `AppState` container holding domain services
3. **Data** — Pluggable database (`Arc<dyn Database>`) and object store (`Arc<dyn ObjectStore>`)

```
Transport (axum / rmcp)
        │
   AppState { MemoryService, JobService, DecisionService,
              ArtifactService, PipelineService, HatcheryService,
              AuthService, CredentialService }
        │
   Database trait + ObjectStore trait
        │
   SQLite/Postgres    LocalFS/S3
```

On startup, Overseer seeds six job definitions: `default`, `spec-from-problem`, `plan-from-spec`, `implement-from-plan`, `review-pr`, `evolve-from-analysis`. Credentials are seeded from `[[credentials]]` entries in `overseer.toml`.

## Database Layer

Six domain-specific traits compose into a single `Database` supertrait:

**MemoryStore** — vector-based semantic storage. `insert_memory()`, `search_memories(provider, embedding, tags, limit)` for KNN search, `create_embedding_table(provider, dimensions)` to initialize per-provider vector tables.

**JobStore** — job definitions (reusable templates), runs (executions with config_overrides), and tasks. `start_job_run(definition_id, triggered_by, parent_id, overrides)`, `list_pending_unassigned_runs()` for hatchery dispatch.

**DecisionStore** — append-only audit log. `log_decision(agent, context, decision, reasoning, tags, run_id)`.

**ArtifactStore** — metadata only (blobs in ObjectStore). `insert_artifact(id, name, content_type, size, run_id, artifact_type)`.

**HatcheryStore** — drone cluster registration. `register_hatchery()`, `heartbeat_hatchery()`, `assign_job_to_hatchery(run_id, hatchery_id)`.

**CredentialStore** — repo credential storage with URL pattern matching. `match_credentials(repo_url)` returns best-match credentials by pattern specificity.

### Domain Models

| Type | Key Fields |
|------|-----------|
| `JobRunStatus` | Pending, Running, Completed, Failed, Cancelled. `is_terminal()` for Completed/Failed/Cancelled |
| `TaskStatus` | Pending, InProgress, Completed, Failed |
| `HatcheryStatus` | Online, Degraded, Offline |
| `ArtifactType` | Generic, Conversation, Session, EvolutionReport |
| `CredentialType` | GithubPat, Other(String). `secrets_key()` → "github_pat" |
| `JobRun` | id, definition_id, parent_id, status, triggered_by, config_overrides, result, error |
| `Task` | id, run_id, subject, status, assigned_to, output (JSON) |
| `Credential` | id, pattern, credential_type, secret (not serialized) |

### Backend Implementations

**SQLite** (`sqlite.rs`) — sqlx pool, sqlite-vec for KNN (`WHERE embedding MATCH ?`), per-provider vector tables (`memory_embeddings_{name}`), JSON as TEXT.

**PostgreSQL** (`postgres.rs`) — sqlx pool, pgvector for KNN (`ORDER BY embedding <-> $1`), single `memory_embeddings` table with `provider` column, JSON as JSONB.

Both backends use `sea-query` table/column enums from `db/tables.rs` for type-safe SQL building.

## Service Layer

`AppState` is the central container, cloned into transport handlers:

```rust
pub struct AppState {
    pub memory: MemoryService,     // embed + store/recall via default provider
    pub jobs: JobService,          // definition + run + task CRUD
    pub decisions: DecisionService,
    pub artifacts: ArtifactService,
    pub pipeline: PipelineService, // stage advancement (spec→plan→implement)
    pub hatchery: HatcheryService,
    pub auth: AuthService,
    pub credentials: CredentialService,
}
```

`MemoryService` orchestrates embedding (via `EmbeddingRegistry`) before delegating to the database. Other services are thin delegation wrappers.

## HTTP API

Router mounts at `/api/`:

| Route | Methods | Purpose |
|-------|---------|---------|
| `/api/memories` | POST, DELETE /{id} | Store/delete memories |
| `/api/memories/search` | GET | KNN semantic search (q, tags, limit) |
| `/api/decisions` | POST, GET | Log and query decisions |
| `/api/jobs` | CRUD | Job definitions |
| `/api/jobs/runs` | CRUD | Job runs, pending list |
| `/api/tasks` | CRUD | Tasks within runs |
| `/api/artifacts` | POST, GET | Artifact metadata + blob storage |
| `/api/hatcheries` | CRUD | Hatchery registration, heartbeat, job assignment |
| `/api/credentials` | CRUD | Credential management, pattern matching |

## MCP Tools

22 tools exposed via rmcp `ServerHandler`, parameter schemas auto-generated via schemars:

**Memory:** `store_memory`, `recall_memory`, `delete_memory`
**Decisions:** `log_decision`, `query_decisions`
**Jobs:** `create_job_definition`, `start_job`, `update_job_run`, `submit_job` (high-level: resolve by name, build config, assign), `list_job_runs`, `list_job_definitions`, `advance_job_run`
**Tasks:** `create_task`, `update_task`, `list_tasks`
**Hatcheries:** `register_hatchery`, `heartbeat_hatchery`, `list_hatcheries`, `deregister_hatchery`, `assign_job_to_hatchery`
**Artifacts:** `store_artifact`, `get_artifact`

## Storage

`create_object_store(config)` dispatches on URL scheme:

- `file://path` → `LocalFileSystem`
- `s3://bucket/prefix` → `AmazonS3` (with optional region, endpoint, IAM from env)

Artifact blobs stored in ObjectStore, metadata in database.

## Embedding

```rust
pub trait EmbeddingProvider: Send + Sync {
    fn embed(&self, text: &str) -> Future<Output = Result<Vec<f32>>>;
    fn model_name(&self) -> &str;
    fn dimensions(&self) -> usize;
}
```

`EmbeddingRegistry` holds named providers. Two implementations:

- **StubEmbedding** — zero vectors, configurable dimensions, no API key needed
- **VoyageEmbedding** — HTTP calls to Voyage AI API, requires `VOYAGE_API_KEY`

Each provider gets a dedicated vector table. Switching default provider doesn't lose old memories.

## Configuration

`overseer.toml` with sensible defaults:

| Section | Key | Default |
|---------|-----|---------|
| `server.http_port` | HTTP listen port | 3100 |
| `server.mcp_transport` | "stdio" or "http" | "stdio" |
| `storage.database_url` | SQLite or Postgres URL | sqlite://data/overseer.db |
| `storage.artifact_url` | file:// or s3:// | file://data/artifacts |
| `embedding.default` | Provider name | "stub" |
| `logging.level` | Log level | "info" |
| `[[credentials]]` | Pattern + type + secret_env | (none) |

## Error Handling

Single `OverseerError` enum with HTTP status mapping:

| Variant | HTTP Status |
|---------|-------------|
| `Storage(sqlx::Error)` | 500 |
| `NotFound(String)` | 404 |
| `Validation(String)` | 400 |
| `Embedding(String)` | 500 |
| `Io(io::Error)` | 500 |
| `Internal(String)` | 500 |
| `ObjectStore(String)` | 500 |

Sensitive errors logged server-side, generic messages returned to clients. Also maps to `McpError` in the MCP layer.
