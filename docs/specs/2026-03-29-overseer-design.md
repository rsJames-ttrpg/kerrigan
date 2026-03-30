# Overseer Design Spec

Overseer is the foundational service of the Kerrigan agentic development platform. It provides persistent memory, job orchestration, decision logging, and artifact storage — exposed via both HTTP REST and MCP interfaces.

## Clients

- **Claude Code** — connects via MCP
- **Local AI models** (AI HAT 2) — connect via HTTP API
- **Human operator** — CLI or dashboard via HTTP API

## Architecture

Single Rust binary, layered monolith. Both transports share the same service layer — no business logic in the transport adapters.

```
┌─────────────────────────────────────────┐
│              MCP Transport              │
├─────────────────────────────────────────┤
│              HTTP API (REST)            │
├─────────────────────────────────────────┤
│            Service Layer                │
│  ┌──────────┬──────────┬─────────────┐  │
│  │ Memory   │ Jobs     │ Decisions   │  │
│  │ Service  │ Service  │ Service     │  │
│  └──────────┴──────────┴─────────────┘  │
├─────────────────────────────────────────┤
│         Storage Layer (traits)          │
│  ┌──────────┬──────────┬─────────────┐  │
│  │ Vector   │ Relational│ Artifact   │  │
│  │ Store    │ Store     │ Store      │  │
│  └──────────┴──────────┴─────────────┘  │
├─────────────────────────────────────────┤
│     SQLite (+ sqlite-vec extension)     │
│     Filesystem (artifact blobs)         │
└─────────────────────────────────────────┘
```

## Data Model

### Memory

| Column | Type | Notes |
|--------|------|-------|
| `id` | UUID | PK |
| `content` | TEXT | The memory text |
| `embedding` | BLOB (vector) | Via sqlite-vec |
| `embedding_model` | TEXT | Which model produced the embedding |
| `source` | TEXT | Agent or session that created it |
| `tags` | JSON array | Categorization (e.g. "architecture", "preference") |
| `expires_at` | TIMESTAMP | Optional TTL, null = permanent |
| `created_at` | TIMESTAMP | |
| `updated_at` | TIMESTAMP | |

### Memory Links

| Column | Type | Notes |
|--------|------|-------|
| `memory_id` | UUID | FK to memory |
| `linked_id` | UUID | FK to target entity |
| `linked_type` | TEXT | `memory` or `decision` |
| `relation_type` | TEXT | e.g. "related", "caused_by", "supersedes" |

### Job Definitions

| Column | Type | Notes |
|--------|------|-------|
| `id` | UUID | PK |
| `name` | TEXT | Unique human-readable name (e.g. "review-pr") |
| `description` | TEXT | What the job does |
| `config` | JSON | Job-specific parameters and templates |
| `created_at` | TIMESTAMP | |
| `updated_at` | TIMESTAMP | |

### Job Runs

| Column | Type | Notes |
|--------|------|-------|
| `id` | UUID | PK |
| `definition_id` | UUID | FK to job definition |
| `parent_id` | UUID | Nullable FK to job run (sub-jobs) |
| `status` | TEXT | `pending`, `running`, `completed`, `failed`, `cancelled` |
| `triggered_by` | TEXT | Agent or user that started it |
| `result` | JSON | Output data, summary |
| `error` | TEXT | Error message if failed |
| `started_at` | TIMESTAMP | |
| `completed_at` | TIMESTAMP | |

### Tasks

| Column | Type | Notes |
|--------|------|-------|
| `id` | UUID | PK |
| `run_id` | UUID | Nullable FK to job run (standalone if null) |
| `subject` | TEXT | What to do |
| `status` | TEXT | `pending`, `in_progress`, `completed`, `failed` |
| `assigned_to` | TEXT | Agent picking it up |
| `output` | JSON | Result data |
| `created_at` | TIMESTAMP | |
| `updated_at` | TIMESTAMP | |

### Decisions (append-only)

| Column | Type | Notes |
|--------|------|-------|
| `id` | UUID | PK |
| `agent` | TEXT | Who made the decision |
| `context` | TEXT | Situation that prompted it |
| `decision` | TEXT | What was decided |
| `reasoning` | TEXT | Why |
| `tags` | JSON array | Domain categorization |
| `run_id` | UUID | Nullable FK to job run |
| `created_at` | TIMESTAMP | |

### Artifacts

| Column | Type | Notes |
|--------|------|-------|
| `id` | UUID | PK |
| `name` | TEXT | Filename or label |
| `content_type` | TEXT | MIME type |
| `size` | INTEGER | Bytes |
| `run_id` | UUID | Nullable FK to job run |
| `created_at` | TIMESTAMP | |

Artifact blobs stored on filesystem at `{data_dir}/artifacts/{id}`.

## Embedding Provider

Pluggable trait — start with a stub, plug in real providers later:

```rust
trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn model_name(&self) -> &str;
    fn dimensions(&self) -> usize;
}
```

Each memory stores `embedding_model` to track which model produced its vector. Vectors from different models are not comparable — queries filter by model.

Initial implementation: no-op stub that returns zero vectors. First real provider: AI HAT 2 local model or a remote embedding API.

## MCP Surface

### Tools

| Tool | Parameters | Description |
|------|-----------|-------------|
| `store_memory` | `content`, `tags?`, `links?`, `expires_at?` | Store text, compute embedding, persist |
| `recall_memory` | `query`, `tags?`, `limit?` | Vector similarity search with optional tag filter |
| `delete_memory` | `id` | Remove a memory |
| `log_decision` | `agent`, `context`, `decision`, `reasoning`, `tags?`, `run_id?` | Record a decision |
| `query_decisions` | `agent?`, `tags?`, `from?`, `to?`, `text?`, `limit?` | Search decisions |
| `create_job_definition` | `name`, `description`, `config?` | Register a reusable job template |
| `start_job` | `definition_id`, `triggered_by`, `parent_id?` | Create a run, returns run ID |
| `update_job_run` | `id`, `status?`, `result?`, `error?` | Update run status/result |
| `create_task` | `subject`, `run_id?`, `assigned_to?` | Create a task |
| `update_task` | `id`, `status?`, `assigned_to?`, `output?` | Claim or update a task |
| `list_tasks` | `status?`, `assigned_to?`, `run_id?` | Filter tasks |
| `store_artifact` | `name`, `content_type`, `data` (base64), `run_id?` | Upload a blob |
| `get_artifact` | `id` | Retrieve artifact content |

### Resources

| URI | Description |
|-----|-------------|
| `memory://search?q={text}&tags={csv}&limit={n}` | Semantic memory search |
| `jobs://definitions` | All registered job definitions |
| `jobs://runs?status={status}` | Job runs filtered by status |
| `tasks://active` | Uncompleted tasks |
| `decisions://recent?limit={n}` | Latest decisions |
| `artifacts://list?run_id={id}` | Artifacts for a job run |

## HTTP API

Mirrors the MCP surface as REST. All endpoints under `/api/`.

| Method | Path | Maps to |
|--------|------|---------|
| POST | `/api/memories` | `store_memory` |
| GET | `/api/memories/search` | `recall_memory` |
| DELETE | `/api/memories/{id}` | `delete_memory` |
| POST | `/api/decisions` | `log_decision` |
| GET | `/api/decisions` | `query_decisions` |
| POST | `/api/jobs/definitions` | `create_job_definition` |
| GET | `/api/jobs/definitions` | list definitions |
| POST | `/api/jobs/runs` | `start_job` |
| PATCH | `/api/jobs/runs/{id}` | `update_job_run` |
| GET | `/api/jobs/runs` | list runs |
| POST | `/api/tasks` | `create_task` |
| PATCH | `/api/tasks/{id}` | `update_task` |
| GET | `/api/tasks` | `list_tasks` |
| POST | `/api/artifacts` | `store_artifact` |
| GET | `/api/artifacts/{id}` | `get_artifact` |

## Configuration

Single TOML file (`overseer.toml`):

```toml
[server]
http_port = 3100
mcp_transport = "stdio"  # or "sse" with a port

[storage]
database_path = "data/overseer.db"
artifact_path = "data/artifacts"

[embedding]
provider = "stub"  # "local", "remote" etc.

[logging]
level = "info"
```

## Error Handling

Single `OverseerError` enum covering all failure modes:
- `Storage` — SQLite errors, filesystem errors
- `NotFound` — entity not found
- `Validation` — bad input
- `Embedding` — embedding provider failures
- `Transport` — MCP/HTTP protocol errors

Each transport maps `OverseerError` to its error format (HTTP status codes, MCP error objects). All errors logged via `tracing`.

## Crate Dependencies

- **axum** — HTTP server
- **sqlx** — async SQLite (pooled connections, compile-time query checking)
- **sqlite-vec** — vector similarity search extension
- **serde / serde_json** — serialization
- **tokio** — async runtime
- **uuid** — ID generation
- **toml** — config parsing
- **tracing / tracing-subscriber** — structured logging
- **rust mcp** - mcp library for rust https://github.com/modelcontextprotocol/rust-sdk

## Hardware Constraints

Target: Raspberry Pi with AI HAT 2. Keep memory footprint low — single SQLite connection pool, no background threads beyond tokio's runtime. Artifact storage avoids putting blobs in the database to keep SQLite compact.
