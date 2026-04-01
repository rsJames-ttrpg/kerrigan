# Multi-Embedder Support

## Problem

Overseer hardcodes a single 384-dimension vec0 table for memory embeddings. Switching embedding models (e.g., from stub to Voyage AI) requires recreating the table and losing existing data. There's no way to configure embedding providers beyond a single `provider = "stub"` string.

## Design

Named embedding providers configured in TOML. Each provider gets its own vec0 table. One provider is the default for new memories and search. Old memories stay in their original provider's table but become unsearchable when the default changes.

## Config

```toml
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
```

- **`default`** — name of the provider used for `store()` and `recall()`.
- **`source`** — selects the implementation. Initial sources: `stub`, `voyage`. Future: `openai`, `cohere`.
- **`dimensions`** — vector size for the vec0 table. Must match what the model produces.
- **`model`** — model identifier sent to the API.
- **`api_key_env`** — environment variable name containing the API key. Not the key itself.

Provider names must match `[a-z0-9_]+` (validated at config load time, used in SQL table names).

## Schema

The static `memory_embeddings` table is replaced by dynamic per-provider tables created on startup:

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS memory_embeddings_{name} USING vec0(
    embedding float[{dimensions}]
);
```

The `memories` table is unchanged. Its `embedding_model` column already records which provider produced each memory's embedding.

## Embedding Providers

### Trait

```rust
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn model_name(&self) -> &str;
    fn dimensions(&self) -> usize;
}
```

Changed from sync to async to support HTTP calls.

### Stub

Unchanged behavior — returns zero vectors. Used for testing and when no real provider is configured.

### Voyage

HTTP POST to `https://api.voyageai.com/v1/embeddings`:

```json
{
  "input": ["text to embed"],
  "model": "voyage-3-lite"
}
```

Response:

```json
{
  "data": [{ "embedding": [0.1, 0.2, ...] }]
}
```

Bearer token from the env var specified by `api_key_env`.

### Future presets

OpenAI and Cohere follow the same pattern — each preset knows its URL and request/response shape. Only `model`, `dimensions`, and `api_key_env` needed in config.

## Embedding Registry

```rust
pub struct EmbeddingRegistry {
    providers: HashMap<String, Arc<dyn EmbeddingProvider>>,
    default: String,
}
```

Constructed at startup from config. Provides:
- `get_default() -> &Arc<dyn EmbeddingProvider>` — the default provider
- `get(name) -> Option<&Arc<dyn EmbeddingProvider>>` — lookup by name
- `default_name() -> &str` — the default provider's config name

## Service Layer

`MemoryService` takes an `EmbeddingRegistry` instead of a single `Arc<dyn EmbeddingProvider>`.

- **`store()`** — embeds with the default provider, inserts into `memory_embeddings_{default_name}`.
- **`recall()`** — embeds the query with the default provider, searches `memory_embeddings_{default_name}`.
- **`delete()`** — deletes from `memories` table. Cleans up the embedding row from the correct provider's table. The `embedding_model` column is changed to store the provider config name (e.g., `"voyage"`) instead of the model identifier, since that's what maps to the table name.

## DB Layer

`insert_memory` and `search_memories` take a `provider_name: &str` parameter to construct the target table name. Table names are built via format string, not SQL parameters. Provider name validation at config load prevents injection.

On startup, `init_pool` runs the static schema (all non-embedding tables), then creates vec0 tables for each configured provider.

## What Gets Embedded

Raw memory `content` only. No metadata, tags, or source included in the embedded text.

## Error Handling

| Condition | Error |
|-----------|-------|
| `default` names a provider not in `[embedding.providers]` | Startup panic |
| `api_key_env` env var not set | Startup panic |
| Provider name fails `[a-z0-9_]+` validation | Startup panic |
| REST embedding call fails (network, auth, rate limit) | `OverseerError::Embedding` |
| Unknown provider name passed to store | `OverseerError::Validation` |
| Embedding dimension mismatch (API returns wrong size) | `OverseerError::Embedding` |

## Migration from Current Schema

The hardcoded `CREATE VIRTUAL TABLE IF NOT EXISTS memory_embeddings` in `schema.sql` is removed. If an existing `memory_embeddings` table exists from a previous version, it is left alone (not dropped). Users migrating would need to manually re-embed or accept that old memories are unsearchable under the new default.

## Testing

- Stub provider tests remain unchanged (they use the `stub` source).
- Voyage provider gets a unit test with a mock HTTP server or is tested manually against the real API.
- `EmbeddingRegistry` tests: default lookup, named lookup, missing name returns error.
- DB-level tests: create vec0 table dynamically, insert/search against named tables.
- Config tests: valid multi-provider config, missing default, invalid provider name.
