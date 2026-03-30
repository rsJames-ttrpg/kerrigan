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
              Storage (sqlx + sqlite-vec)
              Filesystem (artifact blobs)
```

Both transport layers are thin adapters. No business logic outside services.

## Module Structure

| Module | Responsibility |
|--------|---------------|
| `config.rs` | TOML config parsing with defaults |
| `error.rs` | `OverseerError` enum, maps to HTTP status + MCP errors |
| `db/mod.rs` | `SqlitePool` init, schema, sqlite-vec extension loading |
| `db/memory.rs` | Memory CRUD + vector KNN search |
| `db/jobs.rs` | Job definitions, runs, tasks CRUD |
| `db/decisions.rs` | Decision insert + query |
| `db/artifacts.rs` | Artifact metadata CRUD |
| `embedding/mod.rs` | `EmbeddingProvider` trait + `EmbeddingRegistry` named provider lookup |
| `embedding/stub.rs` | Zero-vector stub (placeholder) |
| `embedding/voyage.rs` | Voyage AI HTTP embedding provider |
| `services/mod.rs` | `AppState` holding all services |
| `services/memory.rs` | Embeds text then delegates to db |
| `services/jobs.rs` | Delegates to db/jobs |
| `services/decisions.rs` | Delegates to db/decisions |
| `services/artifacts.rs` | Metadata in db + blob on filesystem |
| `api/mod.rs` | axum router construction |
| `api/{memory,jobs,decisions,artifacts}.rs` | REST endpoint handlers |
| `mcp/mod.rs` | MCP tool definitions (13 tools) |

## Adding a New Feature

1. **New table** → add to `db/schema.sql`, create `db/<feature>.rs` with async sqlx functions
2. **New service** → create `services/<feature>.rs`, add to `AppState`
3. **HTTP endpoint** → create `api/<feature>.rs`, nest in `api/mod.rs` router
4. **MCP tool** → add `#[tool]` method to `OverseerMcp` in `mcp/mod.rs`

## Testing

```bash
cargo test                    # all tests
cargo test db::memory         # memory storage tests
cargo test services::memory   # memory service tests
```

Tests use `db::open_in_memory()` for isolated SQLite instances. Each test module gets its own named in-memory DB to avoid interference.

## Key Patterns

- **sqlx async everywhere** — all db functions are `async fn` taking `&SqlitePool`
- **JSON in TEXT columns** — tags, config, result, output stored as JSON strings
- **sqlite-vec vectors** — bound as `&[u8]` via `zerocopy::IntoBytes`, KNN via `WHERE embedding MATCH ?1 AND k = ?2`
- **Memory embeddings** — rowid in `memory_embeddings_{provider}` matches rowid in `memories` table
- **OverseerError** — single error type, implements `IntoResponse` for HTTP and maps to `McpError` in MCP layer

## Embedding Providers

Named providers configured in `overseer.toml`. Each gets a `memory_embeddings_{name}` vec0 table.

Supported sources:
- `stub` — zero vectors for testing (no API key needed)
- `voyage` — Voyage AI REST API (needs `VOYAGE_API_KEY` env var)

To switch providers, change `[embedding] default = "new_provider"`. Old memories stay in their original table but won't appear in search until re-embedded.
