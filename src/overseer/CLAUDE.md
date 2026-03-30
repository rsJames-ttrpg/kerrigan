# Overseer CLAUDE.md

## What is Overseer

Overseer is the foundational service for the Kerrigan agentic platform. It provides persistent memory (vector search), job orchestration, decision logging, and artifact storage via HTTP REST and MCP interfaces.

## Running

```bash
# HTTP-only mode (for dev/testing)
cargo run -- overseer.toml

# With MCP on stdio (for Claude Code)
# Set mcp_transport = "stdio" in overseer.toml
cargo run -- overseer.toml
```

Config: `overseer.toml` at repo root. Defaults to port 3100, SQLite at `data/overseer.db`.

## Architecture

Layered monolith — three layers sharing a service core:

```
MCP (rmcp)  →  Service Layer  ←  HTTP (axum)
                    ↓
              Database Trait (Arc<dyn Database>)
              ├── SQLite (sqlx + sqlite-vec)
              └── PostgreSQL (sqlx + pgvector)

              ObjectStore (Arc<dyn ObjectStore>)
              ├── Local filesystem
              └── S3/GCS/Azure
```

Both transport layers are thin adapters. No business logic outside services.

## Module Structure

| Module | Responsibility |
|--------|---------------|
| `config.rs` | TOML config parsing with defaults |
| `error.rs` | `OverseerError` enum, maps to HTTP status + MCP errors |
| `db/mod.rs` | DB init — dispatches to SQLite or Postgres based on URL |
| `db/trait_def.rs` | `Database` trait (`Arc<dyn Database + Send + Sync>`) |
| `db/models.rs` | Shared domain model types (Memory, Job, Decision, Artifact, …) |
| `db/tables.rs` | sea-query table/column enums for type-safe SQL building |
| `db/sqlite.rs` | SQLite + sqlite-vec implementation of `Database` |
| `db/postgres.rs` | PostgreSQL + pgvector implementation of `Database` |
| `db/memory.rs` | Memory CRUD + vector KNN search (called by impls above) |
| `db/jobs.rs` | Job definitions, runs, tasks CRUD |
| `db/decisions.rs` | Decision insert + query |
| `db/artifacts.rs` | Artifact metadata CRUD |
| `storage.rs` | `ObjectStore` wrapper — local filesystem or S3 |
| `embedding/mod.rs` | `EmbeddingProvider` trait + `EmbeddingRegistry` named provider lookup |
| `embedding/stub.rs` | Zero-vector stub (placeholder) |
| `embedding/voyage.rs` | Voyage AI HTTP embedding provider |
| `services/mod.rs` | `AppState` holding all services |
| `services/memory.rs` | Embeds text then delegates to db |
| `services/jobs.rs` | Delegates to db/jobs |
| `services/decisions.rs` | Delegates to db/decisions |
| `services/artifacts.rs` | Metadata in db + blob via ObjectStore |
| `api/mod.rs` | axum router construction |
| `api/{memory,jobs,decisions,artifacts}.rs` | REST endpoint handlers |
| `mcp/mod.rs` | MCP tool definitions (13 tools) |

## Configuration

```toml
database_url = "sqlite://data/overseer.db"   # or postgres://user:pass@host/db
artifact_url = "file://data/artifacts"       # or s3://bucket/prefix
```

Backend is selected at startup based on the URL scheme. Migrations are applied automatically via `sqlx::migrate!()`.

## Adding a New Feature

1. **New table** → add a new migration to `migrations/sqlite/` and `migrations/postgres/`, add methods to `Database` trait and both impls
2. **New service** → create `services/<feature>.rs`, add to `AppState`
3. **HTTP endpoint** → create `api/<feature>.rs`, nest in `api/mod.rs` router
4. **MCP tool** → add `#[tool]` method to `OverseerMcp` in `mcp/mod.rs`

## Testing

```bash
cargo test                    # all tests (SQLite only)
cargo test db::memory         # memory storage tests
cargo test services::memory   # memory service tests

# Postgres integration tests (requires a running Postgres instance)
TEST_DATABASE_URL=postgres://user:pass@localhost/overseer_test cargo test
```

SQLite tests use `db::open_in_memory()` for isolated instances. Each test module gets its own named in-memory DB to avoid interference. Postgres integration tests are gated behind the `TEST_DATABASE_URL` environment variable and are skipped if it is not set.

## Key Patterns

- **Database trait** — `Arc<dyn Database + Send + Sync>` passed through `AppState`; implement all methods for both SQLite and Postgres backends
- **sea-query** — SQL built via `sea_query::Query::select()` etc. with typed column enums from `db/tables.rs`; avoids raw SQL strings in most CRUD
- **sqlx async everywhere** — all db functions are `async fn`; SQLite uses `SqlitePool`, Postgres uses `PgPool`
- **object_store** — `Arc<dyn ObjectStore>` for artifact blobs; `PutPayload`/`GetResult` API is the same across local and cloud backends
- **JSON in TEXT/JSONB columns** — tags, config, result, output stored as JSON strings (TEXT in SQLite, JSONB in Postgres)
- **sqlite-vec vectors** — bound as `&[u8]` via `zerocopy::IntoBytes`, KNN via `WHERE embedding MATCH ?1 AND k = ?2`
- **pgvector vectors** — stored as `pgvector::Vector`, KNN via `ORDER BY embedding <-> $1 LIMIT $2`
- **Memory embeddings** — rowid in `memory_embeddings_{provider}` matches rowid in `memories` table (SQLite); Postgres uses a single `memory_embeddings` table with a `provider` column
- **OverseerError** — single error type, implements `IntoResponse` for HTTP and maps to `McpError` in MCP layer

## Embedding Providers

Named providers configured in `overseer.toml`. Each gets a `memory_embeddings_{name}` vec0 table.

Supported sources:
- `stub` — zero vectors for testing (no API key needed)
- `voyage` — Voyage AI REST API (needs `VOYAGE_API_KEY` env var)

To switch providers, change `[embedding] default = "new_provider"`. Old memories stay in their original table but won't appear in search until re-embedded.
