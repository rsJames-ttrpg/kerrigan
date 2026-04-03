# Repo Credentials Design

Store repository credentials in Overseer so they can be pre-configured at deploy time and automatically injected into jobs, eliminating the need to pass `--set secrets.github_pat=...` on every `kerrigan submit`.

## Data Model

New `credentials` table:

| Column | Type | Notes |
|--------|------|-------|
| `id` | TEXT PK | UUID |
| `pattern` | TEXT NOT NULL | e.g. `github.com/rsJames-ttrpg/*` or `github.com/rsJames-ttrpg/kerrigan.git` |
| `credential_type` | TEXT NOT NULL | `github_pat` initially, extensible |
| `secret` | TEXT NOT NULL | Plaintext for now, encryption at rest later |
| `created_at` | TIMESTAMP | |
| `updated_at` | TIMESTAMP | |

UNIQUE constraint on `(pattern, credential_type)`.

### Pattern Matching

Given a `repo_url`, normalize it (strip protocol, `git@`, convert `:` to `/`, strip `.git` suffix, strip trailing `/`) to produce a canonical form like `github.com/rsJames-ttrpg/kerrigan`. Match against stored patterns where `*` is a trailing wildcard only. Most-specific match wins *per credential_type* (longest pattern sans wildcard). Multiple credential types can match the same repo URL — all are returned.

### URL Normalization

All of these normalize to `github.com/rsJames-ttrpg/kerrigan`:

- `git@github.com:rsJames-ttrpg/kerrigan.git`
- `https://github.com/rsJames-ttrpg/kerrigan.git`
- `https://github.com/rsJames-ttrpg/kerrigan`

Rules: strip `https://`, `http://`, `git@`, convert `:` to `/` (SSH URLs), strip `.git` suffix, strip trailing `/`.

Normalization runs on both the input `repo_url` and stored patterns at match time, so patterns work regardless of whether jobs use SSH or HTTPS URLs.

## Overseer REST API

| Method | Path | Purpose |
|--------|------|---------|
| `POST /api/credentials` | Create a credential (`{ pattern, credential_type, secret }`) |
| `GET /api/credentials` | List all (secrets redacted) |
| `GET /api/credentials/:id` | Get one (secret redacted) |
| `DELETE /api/credentials/:id` | Remove a credential |
| `GET /api/credentials/match?repo_url=...` | All best-matching credentials for a repo URL (returns full secrets) |

- List/get endpoints redact the secret value.
- The `/match` endpoint returns a `Vec` of matched credentials (best match per credential_type). Intended for Queen consumption at claim time.
- No REST update endpoint. To change a credential via CLI/API, delete and re-create. Internal upsert is used for deploy-time seeding (idempotent restarts).

## Credential Injection Flow

At job claim time in Queen's poller:

1. Queen claims a run and merges `config_overrides` on top of definition config (existing behavior).
2. Queen reads `repo_url` from the merged config.
3. Queen calls `GET /api/credentials/match?repo_url=<repo_url>` on Overseer.
4. For each matched credential, Queen injects it into the config under the appropriate secrets key.
5. Explicit `--set secrets.*` from the operator takes precedence over auto-injected credentials (per key).
6. Config is passed to the drone as today.

### Credential Type to Secrets Key Mapping

Hardcoded for now:

- `github_pat` → `secrets.github_pat`
- Anything else → skipped with a warning log (unsupported credential types are not injected)

## CLI: `kerrigan creds`

```
kerrigan creds add --pattern "github.com/rsJames-ttrpg/*" --type github_pat --secret ghp_...
kerrigan creds list
kerrigan creds rm <id>
```

`--type` defaults to `github_pat`.

## Deploy-time Seeding

`overseer.toml` supports a `[[credentials]]` array:

```toml
[[credentials]]
pattern = "github.com/rsJames-ttrpg/*"
credential_type = "github_pat"
secret_env = "GITHUB_PAT_RSJAMES"
```

`secret_env` reads the secret from an environment variable at startup. Overseer upserts on startup (match on `pattern + credential_type`), making restarts idempotent.

No `secret = "literal"` field in TOML — secrets always come from env vars, never plaintext on disk.

## Nydus Client Additions

- `create_credential(pattern, credential_type, secret)` → `Credential`
- `list_credentials()` → `Vec<Credential>` (redacted)
- `delete_credential(id)`
- `match_credentials(repo_url)` → `Vec<MatchedCredential>` (with secrets)

URL normalization lives in Nydus as a shared utility since both Queen and the kerrigan CLI use it.

## Layers Affected

| Layer | Change |
|-------|--------|
| `migrations/sqlite/` | New `credentials` table |
| `migrations/postgres/` | New `credentials` table |
| `db/models.rs` | `Credential`, `CredentialType` types |
| `db/tables.rs` | `Credentials` table/column enums |
| `db/trait_def.rs` | CRUD + match methods on `Database` trait |
| `db/sqlite.rs` | SQLite implementation |
| `db/postgres.rs` | Postgres implementation |
| `services/credentials.rs` | Credential service (CRUD + match with normalization) |
| `services/mod.rs` | Add to `AppState` |
| `api/credentials.rs` | REST endpoints |
| `api/mod.rs` | Nest credentials router |
| `config.rs` | Parse `[[credentials]]` from TOML |
| `main.rs` | Seed credentials on startup |
| `nydus/client.rs` | New credential methods |
| `nydus/types.rs` | Response types |
| `queen/actors/poller.rs` | Credential lookup + injection at claim time |
| `kerrigan/main.rs` | `kerrigan creds` subcommand |
