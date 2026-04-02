# Repo Credentials Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Store repo credentials in Overseer so they are automatically injected into jobs at claim time, eliminating repeated `--set secrets.github_pat=...` on every `kerrigan submit`.

**Architecture:** New `CredentialStore` trait added to the Database supertrait, with CRUD + pattern-matching query. A `CredentialService` wraps it with URL normalization. REST endpoints expose it, Nydus client consumes it, Queen's poller calls the match endpoint at claim time, and the kerrigan CLI provides `creds` subcommands. Deploy-time seeding reads `[[credentials]]` from `overseer.toml`.

**Tech Stack:** Rust (edition 2024), axum, sqlx, sea-query, serde, nydus (reqwest), clap

**Spec:** `docs/specs/2026-04-02-repo-credentials-design.md`

---

### Task 1: URL Normalization Utility in Nydus

URL normalization is a pure function needed by multiple later tasks. Build and test it first in nydus since both Queen and kerrigan CLI depend on it.

**Files:**
- Create: `src/nydus/src/normalize.rs`
- Modify: `src/nydus/src/lib.rs` (add `pub mod normalize;`)

- [ ] **Step 1: Write failing tests for URL normalization**

In `src/nydus/src/normalize.rs`:

```rust
/// Normalize a repo URL or credential pattern to a canonical form.
///
/// Strips protocol (`https://`, `http://`, `git@`), converts SSH `:` to `/`,
/// strips `.git` suffix, and strips trailing `/`.
///
/// Examples:
/// - `git@github.com:rsJames-ttrpg/kerrigan.git` → `github.com/rsJames-ttrpg/kerrigan`
/// - `https://github.com/rsJames-ttrpg/kerrigan.git` → `github.com/rsJames-ttrpg/kerrigan`
/// - `github.com/rsJames-ttrpg/*` → `github.com/rsJames-ttrpg/*` (patterns pass through)
pub fn normalize_repo_url(url: &str) -> String {
    todo!()
}

/// Check if a normalized URL matches a normalized pattern.
/// Patterns support trailing `*` wildcard only.
/// Returns the specificity score (length of pattern without wildcard) for ranking.
pub fn pattern_matches(normalized_url: &str, normalized_pattern: &str) -> Option<usize> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_https() {
        assert_eq!(
            normalize_repo_url("https://github.com/rsJames-ttrpg/kerrigan.git"),
            "github.com/rsJames-ttrpg/kerrigan"
        );
    }

    #[test]
    fn test_normalize_ssh() {
        assert_eq!(
            normalize_repo_url("git@github.com:rsJames-ttrpg/kerrigan.git"),
            "github.com/rsJames-ttrpg/kerrigan"
        );
    }

    #[test]
    fn test_normalize_no_git_suffix() {
        assert_eq!(
            normalize_repo_url("https://github.com/rsJames-ttrpg/kerrigan"),
            "github.com/rsJames-ttrpg/kerrigan"
        );
    }

    #[test]
    fn test_normalize_trailing_slash() {
        assert_eq!(
            normalize_repo_url("https://github.com/rsJames-ttrpg/kerrigan/"),
            "github.com/rsJames-ttrpg/kerrigan"
        );
    }

    #[test]
    fn test_normalize_pattern_passthrough() {
        assert_eq!(
            normalize_repo_url("github.com/rsJames-ttrpg/*"),
            "github.com/rsJames-ttrpg/*"
        );
    }

    #[test]
    fn test_normalize_http() {
        assert_eq!(
            normalize_repo_url("http://gitlab.example.com/group/repo.git"),
            "gitlab.example.com/group/repo"
        );
    }

    #[test]
    fn test_pattern_matches_exact() {
        let url = normalize_repo_url("git@github.com:rsJames-ttrpg/kerrigan.git");
        let pattern = normalize_repo_url("github.com/rsJames-ttrpg/kerrigan");
        assert_eq!(pattern_matches(&url, &pattern), Some(38));
    }

    #[test]
    fn test_pattern_matches_wildcard() {
        let url = normalize_repo_url("git@github.com:rsJames-ttrpg/kerrigan.git");
        let pattern = normalize_repo_url("github.com/rsJames-ttrpg/*");
        assert!(pattern_matches(&url, &pattern).is_some());
    }

    #[test]
    fn test_pattern_no_match() {
        let url = normalize_repo_url("git@github.com:other-org/repo.git");
        let pattern = normalize_repo_url("github.com/rsJames-ttrpg/*");
        assert_eq!(pattern_matches(&url, &pattern), None);
    }

    #[test]
    fn test_more_specific_pattern_scores_higher() {
        let url = normalize_repo_url("git@github.com:rsJames-ttrpg/kerrigan.git");
        let broad = normalize_repo_url("github.com/rsJames-ttrpg/*");
        let exact = normalize_repo_url("github.com/rsJames-ttrpg/kerrigan");
        let broad_score = pattern_matches(&url, &broad).unwrap();
        let exact_score = pattern_matches(&url, &exact).unwrap();
        assert!(exact_score > broad_score);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/nydus && cargo test normalize -- --nocapture`
Expected: FAIL with "not yet implemented"

- [ ] **Step 3: Implement normalize_repo_url and pattern_matches**

Replace the `todo!()` bodies in `src/nydus/src/normalize.rs`:

```rust
pub fn normalize_repo_url(url: &str) -> String {
    let mut s = url.to_string();

    // Strip protocol
    for prefix in &["https://", "http://"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_string();
            break;
        }
    }

    // Strip git@ and convert : to / (SSH URLs)
    if let Some(rest) = s.strip_prefix("git@") {
        s = rest.replacen(':', "/", 1);
    }

    // Strip .git suffix
    if let Some(rest) = s.strip_suffix(".git") {
        s = rest.to_string();
    }

    // Strip trailing slash
    s = s.trim_end_matches('/').to_string();

    s
}

pub fn pattern_matches(normalized_url: &str, normalized_pattern: &str) -> Option<usize> {
    if let Some(prefix) = normalized_pattern.strip_suffix('*') {
        if normalized_url.starts_with(prefix) {
            Some(prefix.len())
        } else {
            None
        }
    } else if normalized_url == normalized_pattern {
        Some(normalized_pattern.len())
    } else {
        None
    }
}
```

- [ ] **Step 4: Add module to lib.rs**

In `src/nydus/src/lib.rs`, add:

```rust
pub mod normalize;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src/nydus && cargo test normalize -- --nocapture`
Expected: all PASS

- [ ] **Step 6: Commit**

```bash
git add src/nydus/src/normalize.rs src/nydus/src/lib.rs
git commit -m "feat(nydus): add URL normalization for credential pattern matching"
```

---

### Task 2: Data Model — Migrations, Models, Tables, Trait

Add the `credentials` table and the `CredentialStore` trait to Overseer's database layer.

**Files:**
- Create: `src/overseer/migrations/sqlite/20260403000000_credentials.sql`
- Create: `src/overseer/migrations/postgres/20260403000000_credentials.sql`
- Modify: `src/overseer/src/db/models.rs` — add `Credential` and `CredentialType`
- Modify: `src/overseer/src/db/tables.rs` — add `Credentials` table enum
- Modify: `src/overseer/src/db/trait_def.rs` — add `CredentialStore` trait, add to `Database` supertrait

- [ ] **Step 1: Create SQLite migration**

Create `src/overseer/migrations/sqlite/20260403000000_credentials.sql`:

```sql
CREATE TABLE credentials (
    id TEXT PRIMARY KEY,
    pattern TEXT NOT NULL,
    credential_type TEXT NOT NULL,
    secret TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(pattern, credential_type)
);
```

- [ ] **Step 2: Create Postgres migration**

Create `src/overseer/migrations/postgres/20260403000000_credentials.sql`:

```sql
CREATE TABLE credentials (
    id TEXT PRIMARY KEY,
    pattern TEXT NOT NULL,
    credential_type TEXT NOT NULL,
    secret TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(pattern, credential_type)
);
```

- [ ] **Step 3: Add model types to `src/overseer/src/db/models.rs`**

Add after the `Hatchery` struct (at the end of the file, before `#[cfg(test)]`):

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialType {
    GithubPat,
}

impl fmt::Display for CredentialType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GithubPat => write!(f, "github_pat"),
        }
    }
}

impl FromStr for CredentialType {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "github_pat" => Ok(Self::GithubPat),
            other => Err(format!("unsupported credential type: {other}")),
        }
    }
}

impl CredentialType {
    /// Map this credential type to the secrets key injected into job config.
    /// Only `github_pat` is implemented; others panic.
    pub fn secrets_key(&self) -> &'static str {
        match self {
            Self::GithubPat => "github_pat",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Credential {
    pub id: String,
    pub pattern: String,
    pub credential_type: CredentialType,
    #[serde(skip_serializing)]
    pub secret: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

Note: `#[serde(skip_serializing)]` on `secret` ensures the secret is never accidentally serialized in list/get responses. The match endpoint will construct a separate response type that includes it.

- [ ] **Step 4: Add table enum to `src/overseer/src/db/tables.rs`**

Add at the end of the file:

```rust
#[derive(Iden)]
pub enum Credentials {
    Table,
    Id,
    Pattern,
    CredentialType,
    Secret,
    CreatedAt,
    UpdatedAt,
}
```

- [ ] **Step 5: Add `CredentialStore` trait to `src/overseer/src/db/trait_def.rs`**

Add after the `HatcheryStore` trait (before the `Database` supertrait):

```rust
#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn create_credential(
        &self,
        pattern: &str,
        credential_type: &str,
        secret: &str,
    ) -> Result<Credential>;

    async fn get_credential(&self, id: &str) -> Result<Option<Credential>>;

    async fn delete_credential(&self, id: &str) -> Result<()>;

    async fn list_credentials(&self) -> Result<Vec<Credential>>;

    /// Upsert: insert or update secret if (pattern, credential_type) already exists.
    async fn upsert_credential(
        &self,
        pattern: &str,
        credential_type: &str,
        secret: &str,
    ) -> Result<Credential>;

    /// Return all credentials matching the given repo URL, one per credential_type
    /// (best/most-specific match per type).
    async fn match_credentials(&self, repo_url: &str) -> Result<Vec<Credential>>;
}
```

Update the `Database` supertrait to include `CredentialStore`:

```rust
pub trait Database:
    MemoryStore + JobStore + DecisionStore + ArtifactStore + HatcheryStore + CredentialStore
{
}
impl<
    T: MemoryStore + JobStore + DecisionStore + ArtifactStore + HatcheryStore + CredentialStore,
> Database for T
{
}
```

- [ ] **Step 6: Verify it compiles (expected: fails because SQLite/Postgres don't impl CredentialStore yet)**

Run: `cd src/overseer && cargo check 2>&1 | head -20`
Expected: errors about `CredentialStore` not implemented for `SqliteDatabase` and `PostgresDatabase`

- [ ] **Step 7: Commit**

```bash
git add src/overseer/migrations/ src/overseer/src/db/models.rs src/overseer/src/db/tables.rs src/overseer/src/db/trait_def.rs
git commit -m "feat(overseer): add credentials table schema, models, and CredentialStore trait"
```

---

### Task 3: SQLite CredentialStore Implementation

**Files:**
- Create: `src/overseer/src/db/credentials.rs`
- Modify: `src/overseer/src/db/sqlite.rs` — impl `CredentialStore` for `SqliteDatabase`
- Modify: `src/overseer/src/db/mod.rs` — add `pub mod credentials;`

- [ ] **Step 1: Write failing test for credential CRUD**

Create `src/overseer/src/db/credentials.rs`:

```rust
use chrono::NaiveDateTime;
use sea_query::{Expr, Order, Query, SqliteQueryBuilder};
use sea_query_binder::SqlxBinder;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::models::{Credential, CredentialType};
use super::tables::Credentials;
use crate::error::{OverseerError, Result};

pub(crate) fn row_to_credential(row: &sqlx::sqlite::SqliteRow) -> Credential {
    Credential {
        id: row.get("id"),
        pattern: row.get("pattern"),
        credential_type: row
            .get::<String, _>("credential_type")
            .parse()
            .unwrap_or(CredentialType::GithubPat),
        secret: row.get("secret"),
        created_at: row.get::<NaiveDateTime, _>("created_at").and_utc(),
        updated_at: row.get::<NaiveDateTime, _>("updated_at").and_utc(),
    }
}

pub(crate) async fn create_credential(
    pool: &SqlitePool,
    pattern: &str,
    credential_type: &str,
    secret: &str,
) -> Result<Credential> {
    let id = Uuid::new_v4().to_string();
    let (sql, values) = Query::insert()
        .into_table(Credentials::Table)
        .columns([
            Credentials::Id,
            Credentials::Pattern,
            Credentials::CredentialType,
            Credentials::Secret,
        ])
        .values_panic([id.clone().into(), pattern.into(), credential_type.into(), secret.into()])
        .build_sqlx(SqliteQueryBuilder);

    sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    get_credential(pool, &id)
        .await?
        .ok_or_else(|| OverseerError::Internal("credential not found after insert".into()))
}

pub(crate) async fn get_credential(pool: &SqlitePool, id: &str) -> Result<Option<Credential>> {
    let (sql, values) = Query::select()
        .column(sea_query::Asterisk)
        .from(Credentials::Table)
        .and_where(Expr::col(Credentials::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row.as_ref().map(row_to_credential))
}

pub(crate) async fn delete_credential(pool: &SqlitePool, id: &str) -> Result<()> {
    let (sql, values) = Query::delete()
        .from_table(Credentials::Table)
        .and_where(Expr::col(Credentials::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(())
}

pub(crate) async fn list_credentials(pool: &SqlitePool) -> Result<Vec<Credential>> {
    let (sql, values) = Query::select()
        .column(sea_query::Asterisk)
        .from(Credentials::Table)
        .order_by(Credentials::CreatedAt, Order::Desc)
        .build_sqlx(SqliteQueryBuilder);

    let rows = sqlx::query_with(&sql, values)
        .fetch_all(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(rows.iter().map(row_to_credential).collect())
}

pub(crate) async fn upsert_credential(
    pool: &SqlitePool,
    pattern: &str,
    credential_type: &str,
    secret: &str,
) -> Result<Credential> {
    // Check if exists
    let (sql, values) = Query::select()
        .column(sea_query::Asterisk)
        .from(Credentials::Table)
        .and_where(Expr::col(Credentials::Pattern).eq(pattern))
        .and_where(Expr::col(Credentials::CredentialType).eq(credential_type))
        .build_sqlx(SqliteQueryBuilder);

    let existing = sqlx::query_with(&sql, values)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?;

    if let Some(row) = existing {
        let id: String = row.get("id");
        let (sql, values) = Query::update()
            .table(Credentials::Table)
            .value(Credentials::Secret, secret)
            .value(Credentials::UpdatedAt, sea_query::Expr::current_timestamp())
            .and_where(Expr::col(Credentials::Id).eq(&id))
            .build_sqlx(SqliteQueryBuilder);

        sqlx::query_with(&sql, values)
            .execute(pool)
            .await
            .map_err(OverseerError::Storage)?;

        get_credential(pool, &id)
            .await?
            .ok_or_else(|| OverseerError::Internal("credential not found after upsert".into()))
    } else {
        create_credential(pool, pattern, credential_type, secret).await
    }
}

pub(crate) async fn match_credentials(
    pool: &SqlitePool,
    repo_url: &str,
) -> Result<Vec<Credential>> {
    use nydus::normalize::{normalize_repo_url, pattern_matches};

    let normalized = normalize_repo_url(repo_url);
    let all = list_credentials(pool).await?;

    // Group by credential_type, keep best (highest specificity) match per type
    let mut best: std::collections::HashMap<String, (usize, Credential)> =
        std::collections::HashMap::new();

    for cred in all {
        let normalized_pattern = normalize_repo_url(&cred.pattern);
        if let Some(score) = pattern_matches(&normalized, &normalized_pattern) {
            let type_key = cred.credential_type.to_string();
            match best.entry(type_key) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert((score, cred));
                }
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    if score > e.get().0 {
                        e.insert((score, cred));
                    }
                }
            }
        }
    }

    Ok(best.into_values().map(|(_, cred)| cred).collect())
}

#[cfg(test)]
mod tests {
    use crate::db::SqliteDatabase;

    #[tokio::test]
    async fn test_credential_crud() {
        let db = SqliteDatabase::open_in_memory_named("cred_crud_test")
            .await
            .expect("db opens");

        let cred = super::create_credential(
            &db.pool,
            "github.com/rsJames-ttrpg/*",
            "github_pat",
            "ghp_test123",
        )
        .await
        .expect("create");
        assert_eq!(cred.pattern, "github.com/rsJames-ttrpg/*");
        assert_eq!(cred.secret, "ghp_test123");

        let fetched = super::get_credential(&db.pool, &cred.id)
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(fetched.id, cred.id);

        let all = super::list_credentials(&db.pool).await.expect("list");
        assert_eq!(all.len(), 1);

        super::delete_credential(&db.pool, &cred.id)
            .await
            .expect("delete");

        let gone = super::get_credential(&db.pool, &cred.id)
            .await
            .expect("get after delete");
        assert!(gone.is_none());
    }

    #[tokio::test]
    async fn test_credential_upsert() {
        let db = SqliteDatabase::open_in_memory_named("cred_upsert_test")
            .await
            .expect("db opens");

        let cred1 = super::upsert_credential(
            &db.pool,
            "github.com/org/*",
            "github_pat",
            "old_secret",
        )
        .await
        .expect("first upsert");

        let cred2 = super::upsert_credential(
            &db.pool,
            "github.com/org/*",
            "github_pat",
            "new_secret",
        )
        .await
        .expect("second upsert");

        assert_eq!(cred1.id, cred2.id);
        assert_eq!(cred2.secret, "new_secret");

        let all = super::list_credentials(&db.pool).await.expect("list");
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn test_credential_match_best_per_type() {
        let db = SqliteDatabase::open_in_memory_named("cred_match_test")
            .await
            .expect("db opens");

        // Broad wildcard
        super::create_credential(
            &db.pool,
            "github.com/rsJames-ttrpg/*",
            "github_pat",
            "broad_pat",
        )
        .await
        .expect("create broad");

        // Exact match (more specific)
        super::create_credential(
            &db.pool,
            "github.com/rsJames-ttrpg/kerrigan",
            "github_pat",
            "exact_pat",
        )
        .await
        .expect("create exact");

        let matches = super::match_credentials(
            &db.pool,
            "git@github.com:rsJames-ttrpg/kerrigan.git",
        )
        .await
        .expect("match");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].secret, "exact_pat");
    }

    #[tokio::test]
    async fn test_credential_match_multiple_types() {
        let db = SqliteDatabase::open_in_memory_named("cred_match_multi_test")
            .await
            .expect("db opens");

        super::create_credential(
            &db.pool,
            "github.com/org/*",
            "github_pat",
            "my_pat",
        )
        .await
        .expect("create pat");

        // Simulate a second credential type by inserting directly
        // (CredentialType only has GithubPat for now, but the DB stores strings)
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO credentials (id, pattern, credential_type, secret) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind("github.com/org/*")
        .bind("deployment_key")
        .bind("my_deploy_key")
        .execute(&db.pool)
        .await
        .expect("insert deployment_key");

        let matches = super::match_credentials(
            &db.pool,
            "https://github.com/org/repo.git",
        )
        .await
        .expect("match");

        assert_eq!(matches.len(), 2);
    }
}
```

- [ ] **Step 2: Add module to `src/overseer/src/db/mod.rs`**

Add `pub mod credentials;` to the module list at the top of the file (after `pub mod artifacts;`).

- [ ] **Step 3: Implement `CredentialStore` for `SqliteDatabase` in `src/overseer/src/db/sqlite.rs`**

Add at the end of the file (before any `#[cfg(test)]` block):

```rust
use super::trait_def::CredentialStore;

#[async_trait]
impl CredentialStore for SqliteDatabase {
    async fn create_credential(
        &self,
        pattern: &str,
        credential_type: &str,
        secret: &str,
    ) -> Result<Credential> {
        super::credentials::create_credential(&self.pool, pattern, credential_type, secret).await
    }

    async fn get_credential(&self, id: &str) -> Result<Option<Credential>> {
        super::credentials::get_credential(&self.pool, id).await
    }

    async fn delete_credential(&self, id: &str) -> Result<()> {
        super::credentials::delete_credential(&self.pool, id).await
    }

    async fn list_credentials(&self) -> Result<Vec<Credential>> {
        super::credentials::list_credentials(&self.pool).await
    }

    async fn upsert_credential(
        &self,
        pattern: &str,
        credential_type: &str,
        secret: &str,
    ) -> Result<Credential> {
        super::credentials::upsert_credential(&self.pool, pattern, credential_type, secret).await
    }

    async fn match_credentials(&self, repo_url: &str) -> Result<Vec<Credential>> {
        super::credentials::match_credentials(&self.pool, repo_url).await
    }
}
```

- [ ] **Step 4: Implement `CredentialStore` for `PostgresDatabase` in `src/overseer/src/db/postgres.rs`**

Add at the end of the file (before any `#[cfg(test)]` block). The Postgres implementation uses the same pattern-matching logic but with `PgPool`:

```rust
use super::trait_def::CredentialStore;

fn row_to_credential(row: &sqlx::postgres::PgRow) -> Credential {
    Credential {
        id: row.get("id"),
        pattern: row.get("pattern"),
        credential_type: row
            .get::<String, _>("credential_type")
            .parse()
            .unwrap_or(CredentialType::GithubPat),
        secret: row.get("secret"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

#[async_trait]
impl CredentialStore for PostgresDatabase {
    async fn create_credential(
        &self,
        pattern: &str,
        credential_type: &str,
        secret: &str,
    ) -> Result<Credential> {
        let id = Uuid::new_v4().to_string();
        let (sql, values) = Query::insert()
            .into_table(Credentials::Table)
            .columns([
                Credentials::Id,
                Credentials::Pattern,
                Credentials::CredentialType,
                Credentials::Secret,
            ])
            .values_panic([id.clone().into(), pattern.into(), credential_type.into(), secret.into()])
            .build_sqlx(PostgresQueryBuilder);

        sqlx::query_with(&sql, values)
            .execute(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        self.get_credential(&id)
            .await?
            .ok_or_else(|| OverseerError::Internal("credential not found after insert".into()))
    }

    async fn get_credential(&self, id: &str) -> Result<Option<Credential>> {
        let (sql, values) = Query::select()
            .column(sea_query::Asterisk)
            .from(Credentials::Table)
            .and_where(Expr::col(Credentials::Id).eq(id))
            .build_sqlx(PostgresQueryBuilder);

        let row = sqlx::query_with(&sql, values)
            .fetch_optional(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(row.as_ref().map(row_to_credential))
    }

    async fn delete_credential(&self, id: &str) -> Result<()> {
        let (sql, values) = Query::delete()
            .from_table(Credentials::Table)
            .and_where(Expr::col(Credentials::Id).eq(id))
            .build_sqlx(PostgresQueryBuilder);

        sqlx::query_with(&sql, values)
            .execute(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(())
    }

    async fn list_credentials(&self) -> Result<Vec<Credential>> {
        let (sql, values) = Query::select()
            .column(sea_query::Asterisk)
            .from(Credentials::Table)
            .order_by(Credentials::CreatedAt, Order::Desc)
            .build_sqlx(PostgresQueryBuilder);

        let rows = sqlx::query_with(&sql, values)
            .fetch_all(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        Ok(rows.iter().map(row_to_credential).collect())
    }

    async fn upsert_credential(
        &self,
        pattern: &str,
        credential_type: &str,
        secret: &str,
    ) -> Result<Credential> {
        let (sql, values) = Query::select()
            .column(sea_query::Asterisk)
            .from(Credentials::Table)
            .and_where(Expr::col(Credentials::Pattern).eq(pattern))
            .and_where(Expr::col(Credentials::CredentialType).eq(credential_type))
            .build_sqlx(PostgresQueryBuilder);

        let existing = sqlx::query_with(&sql, values)
            .fetch_optional(&self.pool)
            .await
            .map_err(OverseerError::Storage)?;

        if let Some(row) = existing {
            let id: String = row.get("id");
            let (sql, values) = Query::update()
                .table(Credentials::Table)
                .value(Credentials::Secret, secret)
                .value(Credentials::UpdatedAt, Expr::current_timestamp())
                .and_where(Expr::col(Credentials::Id).eq(&id))
                .build_sqlx(PostgresQueryBuilder);

            sqlx::query_with(&sql, values)
                .execute(&self.pool)
                .await
                .map_err(OverseerError::Storage)?;

            self.get_credential(&id)
                .await?
                .ok_or_else(|| OverseerError::Internal("credential not found after upsert".into()))
        } else {
            self.create_credential(pattern, credential_type, secret).await
        }
    }

    async fn match_credentials(&self, repo_url: &str) -> Result<Vec<Credential>> {
        use nydus::normalize::{normalize_repo_url, pattern_matches};

        let normalized = normalize_repo_url(repo_url);
        let all = self.list_credentials().await?;

        let mut best: std::collections::HashMap<String, (usize, Credential)> =
            std::collections::HashMap::new();

        for cred in all {
            let normalized_pattern = normalize_repo_url(&cred.pattern);
            if let Some(score) = pattern_matches(&normalized, &normalized_pattern) {
                let type_key = cred.credential_type.to_string();
                match best.entry(type_key) {
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert((score, cred));
                    }
                    std::collections::hash_map::Entry::Occupied(mut e) => {
                        if score > e.get().0 {
                            e.insert((score, cred));
                        }
                    }
                }
            }
        }

        Ok(best.into_values().map(|(_, cred)| cred).collect())
    }
}
```

Note: `row_to_credential` in postgres.rs will conflict with the existing function name in that file — rename it to `row_to_credential_pg` or put it in a nested scope, or prefix with the module. Given the existing pattern in postgres.rs already has `row_to_job_definition`, `row_to_job_run`, etc., just name it `row_to_credential` — it won't conflict since the one in `credentials.rs` is for SQLite rows and this one is for Postgres rows.

- [ ] **Step 5: Add nydus as a dependency for overseer if not already present**

Check `src/overseer/Cargo.toml` for `nydus`. If it's not there, add `nydus = { path = "../nydus" }` to `[dependencies]`. The `match_credentials` function imports from `nydus::normalize`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cd src/overseer && cargo test db::credentials -- --nocapture`
Expected: all PASS

- [ ] **Step 7: Add credential conformance to `trait_conformance_suite` in `src/overseer/src/db/mod.rs`**

Add at the end of the `trait_conformance_suite` function (before the closing `}`):

```rust
    // Credentials
    let cred = db
        .create_credential("github.com/test-org/*", "github_pat", "ghp_test")
        .await
        .expect("create credential");
    assert_eq!(cred.pattern, "github.com/test-org/*");

    let fetched_cred = db.get_credential(&cred.id).await.expect("get credential");
    assert!(fetched_cred.is_some());

    let creds = db.list_credentials().await.expect("list credentials");
    assert!(!creds.is_empty());

    let matched = db
        .match_credentials("https://github.com/test-org/repo.git")
        .await
        .expect("match credentials");
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].secret, "ghp_test");

    let upserted = db
        .upsert_credential("github.com/test-org/*", "github_pat", "ghp_updated")
        .await
        .expect("upsert credential");
    assert_eq!(upserted.id, cred.id);
    assert_eq!(upserted.secret, "ghp_updated");

    db.delete_credential(&cred.id)
        .await
        .expect("delete credential");
    let gone_cred = db.get_credential(&cred.id).await.expect("get after delete");
    assert!(gone_cred.is_none());
```

- [ ] **Step 8: Run full test suite**

Run: `cd src/overseer && cargo test -- --nocapture`
Expected: all PASS including trait conformance

- [ ] **Step 9: Commit**

```bash
git add src/overseer/src/db/ src/overseer/migrations/
git commit -m "feat(overseer): implement CredentialStore for SQLite and Postgres"
```

---

### Task 4: Credential Service

**Files:**
- Create: `src/overseer/src/services/credentials.rs`
- Modify: `src/overseer/src/services/mod.rs` — add module and to `AppState`

- [ ] **Step 1: Write the credential service**

Create `src/overseer/src/services/credentials.rs`:

```rust
use std::sync::Arc;

use crate::db::Database;
use crate::db::models::Credential;
use crate::error::Result;

pub struct CredentialService {
    db: Arc<dyn Database>,
}

impl CredentialService {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }

    pub async fn create_credential(
        &self,
        pattern: &str,
        credential_type: &str,
        secret: &str,
    ) -> Result<Credential> {
        self.db
            .create_credential(pattern, credential_type, secret)
            .await
    }

    pub async fn get_credential(&self, id: &str) -> Result<Option<Credential>> {
        self.db.get_credential(id).await
    }

    pub async fn delete_credential(&self, id: &str) -> Result<()> {
        self.db.delete_credential(id).await
    }

    pub async fn list_credentials(&self) -> Result<Vec<Credential>> {
        self.db.list_credentials().await
    }

    pub async fn upsert_credential(
        &self,
        pattern: &str,
        credential_type: &str,
        secret: &str,
    ) -> Result<Credential> {
        self.db
            .upsert_credential(pattern, credential_type, secret)
            .await
    }

    pub async fn match_credentials(&self, repo_url: &str) -> Result<Vec<Credential>> {
        self.db.match_credentials(repo_url).await
    }
}
```

- [ ] **Step 2: Add to `AppState` in `src/overseer/src/services/mod.rs`**

Add `pub mod credentials;` to the module list.

Add `pub credentials: credentials::CredentialService,` to the `AppState` struct.

In `AppState::new`, add before the closing brace of the `Self { ... }`:

```rust
            credentials: credentials::CredentialService::new(db.clone()),
```

Adjust the `db` cloning: the last usage of `db` in the constructor should use `db` without `.clone()`. Currently `hatchery` is last and uses `db` without clone. Move `credentials` before `hatchery`, or clone `db` for `credentials` too. The simplest fix: change `hatchery: hatchery::HatcheryService::new(db),` to `hatchery: hatchery::HatcheryService::new(db.clone()),` and add `credentials: credentials::CredentialService::new(db),` at the end.

- [ ] **Step 3: Verify it compiles**

Run: `cd src/overseer && cargo check`
Expected: OK

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/services/
git commit -m "feat(overseer): add CredentialService to AppState"
```

---

### Task 5: REST API Endpoints

**Files:**
- Create: `src/overseer/src/api/credentials.rs`
- Modify: `src/overseer/src/api/mod.rs` — add module and nest router

- [ ] **Step 1: Create the credentials API handler**

Create `src/overseer/src/api/credentials.rs`:

```rust
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, post},
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::error::Result;
use crate::services::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", post(create_credential))
        .route("/", get(list_credentials))
        .route("/match", get(match_credentials))
        .route("/{id}", get(get_credential))
        .route("/{id}", delete(delete_credential))
}

#[derive(Deserialize)]
struct CreateCredentialRequest {
    pattern: String,
    credential_type: String,
    secret: String,
}

async fn create_credential(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateCredentialRequest>,
) -> Result<Json<Value>> {
    let cred = state
        .credentials
        .create_credential(&body.pattern, &body.credential_type, &body.secret)
        .await?;
    Ok(Json(redacted_credential(&cred)))
}

async fn list_credentials(State(state): State<Arc<AppState>>) -> Result<Json<Value>> {
    let creds = state.credentials.list_credentials().await?;
    let redacted: Vec<Value> = creds.iter().map(redacted_credential).collect();
    Ok(Json(serde_json::to_value(redacted).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

async fn get_credential(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let cred = state
        .credentials
        .get_credential(&id)
        .await?
        .ok_or_else(|| crate::error::OverseerError::NotFound(format!("credential {id}")))?;
    Ok(Json(redacted_credential(&cred)))
}

async fn delete_credential(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    state.credentials.delete_credential(&id).await?;
    Ok(Json(serde_json::json!({"deleted": true})))
}

#[derive(Deserialize)]
struct MatchQuery {
    repo_url: String,
}

async fn match_credentials(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MatchQuery>,
) -> Result<Json<Value>> {
    let matches = state
        .credentials
        .match_credentials(&params.repo_url)
        .await?;
    // Match endpoint returns full secrets (for Queen consumption)
    let result: Vec<Value> = matches
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "pattern": c.pattern,
                "credential_type": c.credential_type,
                "secret": c.secret,
            })
        })
        .collect();
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

fn redacted_credential(cred: &crate::db::models::Credential) -> Value {
    serde_json::json!({
        "id": cred.id,
        "pattern": cred.pattern,
        "credential_type": cred.credential_type,
        "created_at": cred.created_at,
        "updated_at": cred.updated_at,
    })
}
```

- [ ] **Step 2: Nest the router in `src/overseer/src/api/mod.rs`**

Add `mod credentials;` to the module list at the top.

Add `.nest("/api/credentials", credentials::router())` to the `router()` function, before `.with_state(state)`.

- [ ] **Step 3: Verify it compiles**

Run: `cd src/overseer && cargo check`
Expected: OK

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/api/
git commit -m "feat(overseer): add /api/credentials REST endpoints"
```

---

### Task 6: Nydus Client Methods

**Files:**
- Modify: `src/nydus/src/client.rs` — add credential methods
- Modify: `src/nydus/src/types.rs` — add `Credential` and `MatchedCredential` types

- [ ] **Step 1: Add types to `src/nydus/src/types.rs`**

Add at the end of the file:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub id: String,
    pub pattern: String,
    pub credential_type: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedCredential {
    pub id: String,
    pub pattern: String,
    pub credential_type: String,
    pub secret: String,
}
```

- [ ] **Step 2: Add client methods to `src/nydus/src/client.rs`**

Add the import of new types at the top (update the existing `use crate::types::` line):

```rust
use crate::types::{Artifact, Credential, Hatchery, JobDefinition, JobRun, MatchedCredential, Task};
```

Add the following methods in a `// --- Credentials ---` section before the `// --- Auth ---` section:

```rust
    // --- Credentials ---

    pub async fn create_credential(
        &self,
        pattern: &str,
        credential_type: &str,
        secret: &str,
    ) -> Result<Credential, Error> {
        #[derive(Serialize)]
        struct Body<'a> {
            pattern: &'a str,
            credential_type: &'a str,
            secret: &'a str,
        }
        let resp = self
            .client
            .post(format!("{}/api/credentials", self.base_url))
            .json(&Body {
                pattern,
                credential_type,
                secret,
            })
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn list_credentials(&self) -> Result<Vec<Credential>, Error> {
        let resp = self
            .client
            .get(format!("{}/api/credentials", self.base_url))
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }

    pub async fn delete_credential(&self, id: &str) -> Result<(), Error> {
        let resp = self
            .client
            .delete(format!("{}/api/credentials/{id}", self.base_url))
            .send()
            .await?;
        Self::check_response(resp).await?;
        Ok(())
    }

    pub async fn match_credentials(
        &self,
        repo_url: &str,
    ) -> Result<Vec<MatchedCredential>, Error> {
        let resp = self
            .client
            .get(format!("{}/api/credentials/match", self.base_url))
            .query(&[("repo_url", repo_url)])
            .send()
            .await?;
        Ok(Self::check_response(resp).await?.json().await?)
    }
```

- [ ] **Step 3: Verify it compiles**

Run: `cd src/nydus && cargo check`
Expected: OK

- [ ] **Step 4: Commit**

```bash
git add src/nydus/src/
git commit -m "feat(nydus): add credential client methods and types"
```

---

### Task 7: Deploy-time Seeding via overseer.toml

**Files:**
- Modify: `src/overseer/src/config.rs` — add `[[credentials]]` parsing
- Modify: `src/overseer/src/main.rs` — seed credentials on startup

- [ ] **Step 1: Write failing test for config parsing**

Add to `src/overseer/src/config.rs`, in the `Config` struct:

```rust
    #[serde(default)]
    pub credentials: Vec<CredentialSeed>,
```

Add the `CredentialSeed` struct after `LoggingConfig`:

```rust
#[derive(Debug, Deserialize)]
pub struct CredentialSeed {
    pub pattern: String,
    pub credential_type: String,
    pub secret_env: String,
}
```

Add a test in the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn test_credentials_config() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(
            f,
            r#"
[[credentials]]
pattern = "github.com/org/*"
credential_type = "github_pat"
secret_env = "MY_PAT"
"#
        )
        .unwrap();
        let config = Config::load(f.path()).expect("should parse");
        assert_eq!(config.credentials.len(), 1);
        assert_eq!(config.credentials[0].pattern, "github.com/org/*");
        assert_eq!(config.credentials[0].credential_type, "github_pat");
        assert_eq!(config.credentials[0].secret_env, "MY_PAT");
    }

    #[test]
    fn test_no_credentials_defaults_empty() {
        let config = Config::load(std::path::Path::new("nonexistent.toml"))
            .expect("should fall back to defaults");
        assert!(config.credentials.is_empty());
    }
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cd src/overseer && cargo test config::tests::test_credentials_config -- --nocapture`
Expected: PASS (the struct and deserialization should work)

- [ ] **Step 3: Add seeding logic to `src/overseer/src/main.rs`**

After the job definition seeding block (after the `for (name, description, def_config) in seed_definitions` loop), add:

```rust
    // Seed credentials from config (deploy-time provisioning)
    for cred_seed in &config.credentials {
        match std::env::var(&cred_seed.secret_env) {
            Ok(secret) => {
                state
                    .credentials
                    .upsert_credential(
                        &cred_seed.pattern,
                        &cred_seed.credential_type,
                        &secret,
                    )
                    .await?;
                tracing::info!(
                    pattern = %cred_seed.pattern,
                    credential_type = %cred_seed.credential_type,
                    "seeded credential from env var {}",
                    cred_seed.secret_env,
                );
            }
            Err(_) => {
                tracing::warn!(
                    pattern = %cred_seed.pattern,
                    env_var = %cred_seed.secret_env,
                    "skipping credential seed: env var not set",
                );
            }
        }
    }
```

- [ ] **Step 4: Verify it compiles**

Run: `cd src/overseer && cargo check`
Expected: OK

- [ ] **Step 5: Commit**

```bash
git add src/overseer/src/config.rs src/overseer/src/main.rs
git commit -m "feat(overseer): seed credentials from overseer.toml at startup"
```

---

### Task 8: Queen Credential Injection at Claim Time

**Files:**
- Modify: `src/queen/src/actors/poller.rs` — add credential lookup + injection after config merge

- [ ] **Step 1: Add credential injection to the poller**

In `src/queen/src/actors/poller.rs`, after the config merge block (after line 85: `}`) and before `let drone_type = config` (line 87), add:

```rust
            // Inject credentials from Overseer for this repo_url
            if let Some(repo_url) = config.get("repo_url").and_then(|v| v.as_str()) {
                match client.match_credentials(repo_url).await {
                    Ok(matched_creds) => {
                        for mc in matched_creds {
                            let secrets_key = match mc.credential_type.as_str() {
                                "github_pat" => "github_pat",
                                other => {
                                    tracing::warn!(
                                        credential_type = %other,
                                        "unsupported credential type, skipping"
                                    );
                                    continue;
                                }
                            };
                            // Only inject if not already set by operator override
                            let secrets = config
                                .as_object_mut()
                                .unwrap()
                                .entry("secrets")
                                .or_insert_with(|| serde_json::json!({}));
                            if secrets.get(secrets_key).is_none() {
                                secrets[secrets_key] =
                                    serde_json::Value::String(mc.secret.clone());
                                tracing::info!(
                                    job_run_id = %run.id,
                                    credential_type = %mc.credential_type,
                                    pattern = %mc.pattern,
                                    "injected credential from Overseer"
                                );
                            } else {
                                tracing::debug!(
                                    job_run_id = %run.id,
                                    credential_type = %mc.credential_type,
                                    "credential already set by operator, skipping injection"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            job_run_id = %run.id,
                            error = %e,
                            "failed to fetch credentials, continuing without injection"
                        );
                    }
                }
            }
```

- [ ] **Step 2: Verify it compiles**

Run: `cd src/queen && cargo check`
Expected: OK

- [ ] **Step 3: Commit**

```bash
git add src/queen/src/actors/poller.rs
git commit -m "feat(queen): inject matched credentials at job claim time"
```

---

### Task 9: Kerrigan CLI `creds` Subcommand

**Files:**
- Modify: `src/kerrigan/src/main.rs` — add `Creds` subcommand with `add`, `list`, `rm`

- [ ] **Step 1: Add the `Creds` subcommand**

In `src/kerrigan/src/main.rs`, add a new variant to the `Command` enum:

```rust
    /// Manage repository credentials
    Creds {
        #[command(subcommand)]
        action: CredsAction,
    },
```

Add the `CredsAction` enum (after `ArtifactsAction`):

```rust
#[derive(Subcommand)]
enum CredsAction {
    /// Add a credential for a repo pattern
    Add {
        /// URL pattern (e.g. "github.com/org/*" or "github.com/org/repo")
        #[arg(long)]
        pattern: String,
        /// Credential type
        #[arg(long = "type", default_value = "github_pat")]
        credential_type: String,
        /// Secret value
        #[arg(long)]
        secret: String,
    },
    /// List all credentials (secrets redacted)
    List,
    /// Remove a credential
    Rm {
        /// Credential ID (prefix ok)
        id: String,
    },
}
```

Add the match arm in `async_main`:

```rust
        Command::Creds { action } => match action {
            CredsAction::Add {
                pattern,
                credential_type,
                secret,
            } => cmd_creds_add(&client, &pattern, &credential_type, &secret).await,
            CredsAction::List => cmd_creds_list(&client).await,
            CredsAction::Rm { id } => cmd_creds_rm(&client, &id).await,
        },
```

Add the handler functions:

```rust
async fn cmd_creds_add(
    client: &NydusClient,
    pattern: &str,
    credential_type: &str,
    secret: &str,
) -> Result<()> {
    let cred = client
        .create_credential(pattern, credential_type, secret)
        .await?;
    println!(
        "Created credential {} for pattern '{}' (type: {})",
        display::short_id(&cred.id),
        cred.pattern,
        cred.credential_type,
    );
    Ok(())
}

async fn cmd_creds_list(client: &NydusClient) -> Result<()> {
    let creds = client.list_credentials().await?;
    if creds.is_empty() {
        println!("No credentials configured.");
        return Ok(());
    }
    for cred in &creds {
        println!(
            "  {} {} [{}]",
            display::short_id(&cred.id),
            cred.pattern,
            cred.credential_type,
        );
    }
    Ok(())
}

async fn cmd_creds_rm(client: &NydusClient, id: &str) -> Result<()> {
    // For prefix matching, list creds and find the one that starts with id
    let creds = client.list_credentials().await?;
    let matching: Vec<_> = creds.iter().filter(|c| c.id.starts_with(id)).collect();
    match matching.len() {
        0 => anyhow::bail!("no credential matching '{id}'"),
        1 => {
            client.delete_credential(&matching[0].id).await?;
            println!("Deleted credential {}", display::short_id(&matching[0].id));
        }
        n => anyhow::bail!("ambiguous prefix '{id}' matches {n} credentials"),
    }
    Ok(())
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd src/kerrigan && cargo check`
Expected: OK

- [ ] **Step 3: Commit**

```bash
git add src/kerrigan/src/main.rs
git commit -m "feat(kerrigan): add creds subcommand for managing repo credentials"
```

---

### Task 10: Integration Smoke Test + Buck2 Build

Verify the full stack compiles under Buck2 and the existing tests pass.

- [ ] **Step 1: Run cargo tests for all crates**

```bash
cd src/nydus && cargo test
cd src/overseer && cargo test
cd src/queen && cargo check
cd src/kerrigan && cargo check
```

Expected: all pass/ok

- [ ] **Step 2: Run buckify if needed**

If you added `nydus` as a dep to overseer's `Cargo.toml`, run:

```bash
./tools/buckify.sh
```

Then update `src/overseer/BUCK` to add `"//src/nydus:nydus"` to the `deps` list if not already there.

- [ ] **Step 3: Run Buck2 build**

```bash
buck2 build root//src/overseer:overseer root//src/queen:queen root//src/kerrigan:kerrigan root//src/nydus:nydus
```

Expected: all succeed

- [ ] **Step 4: Run pre-commit hooks**

```bash
buck2 run root//tools:prek -- run --all-files
```

Expected: all pass

- [ ] **Step 5: Final commit if any fixups were needed**

```bash
git add -A
git commit -m "chore: fix build/lint issues from credentials feature"
```
