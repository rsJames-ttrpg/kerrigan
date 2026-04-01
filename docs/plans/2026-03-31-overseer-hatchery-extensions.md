# Overseer Hatchery Extensions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend Overseer with Hatchery registration, heartbeat, and job routing so Hatcheries can register, stay alive, advertise drone capabilities, and receive job assignments.

**Architecture:** New `HatcheryStore` trait + `HatcheryService` following the existing pattern (trait → db module → sqlite impl → service → API + MCP). New `hatcheries` table tracks registered Hatcheries, their status, capabilities, and last heartbeat. Job runs gain an optional `hatchery_id` column for assignment. A simple capability-matching router assigns unassigned jobs to live Hatcheries.

**Tech Stack:** Rust (edition 2024), sqlx, sea-query, axum, rmcp, chrono, serde_json, uuid

---

### Task 1: Migration — hatcheries table

**Files:**
- Create: `src/overseer/migrations/sqlite/20260331000000_hatcheries.sql`
- Create: `src/overseer/migrations/postgres/20260331000000_hatcheries.sql`

- [ ] **Step 1: Write SQLite migration**

```sql
-- src/overseer/migrations/sqlite/20260331000000_hatcheries.sql

CREATE TABLE hatcheries (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    status TEXT NOT NULL DEFAULT 'online' CHECK (status IN ('online', 'degraded', 'offline')),
    capabilities TEXT NOT NULL DEFAULT '{}',
    max_concurrency INTEGER NOT NULL DEFAULT 1,
    active_drones INTEGER NOT NULL DEFAULT 0,
    last_heartbeat_at TEXT NOT NULL DEFAULT (datetime('now')),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

ALTER TABLE job_runs ADD COLUMN hatchery_id TEXT REFERENCES hatcheries(id);
```

- [ ] **Step 2: Write Postgres migration**

```sql
-- src/overseer/migrations/postgres/20260331000000_hatcheries.sql

CREATE TABLE hatcheries (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    status TEXT NOT NULL DEFAULT 'online' CHECK (status IN ('online', 'degraded', 'offline')),
    capabilities JSONB NOT NULL DEFAULT '{}',
    max_concurrency INTEGER NOT NULL DEFAULT 1,
    active_drones INTEGER NOT NULL DEFAULT 0,
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE job_runs ADD COLUMN hatchery_id TEXT REFERENCES hatcheries(id);
```

- [ ] **Step 3: Verify migrations compile**

Run: `cd src/overseer && cargo check`
Expected: compiles (sqlx checks migrations at compile time)

- [ ] **Step 4: Commit**

```bash
git add src/overseer/migrations/
git commit -m "feat(overseer): add hatcheries table and job_runs.hatchery_id column"
```

---

### Task 2: Models and table enums for Hatchery

**Files:**
- Modify: `src/overseer/src/db/models.rs`
- Modify: `src/overseer/src/db/tables.rs`

- [ ] **Step 1: Write failing test for HatcheryStatus parsing**

Add to the bottom of `src/overseer/src/db/models.rs`, inside a new `#[cfg(test)] mod tests` block:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hatchery_status_roundtrip() {
        for s in ["online", "degraded", "offline"] {
            let status: HatcheryStatus = s.parse().unwrap();
            assert_eq!(status.to_string(), s);
        }
    }

    #[test]
    fn test_hatchery_status_invalid() {
        assert!("bogus".parse::<HatcheryStatus>().is_err());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/overseer && cargo test db::models::tests`
Expected: FAIL — `HatcheryStatus` does not exist

- [ ] **Step 3: Add HatcheryStatus enum and Hatchery model**

Add to `src/overseer/src/db/models.rs` after the `TaskStatus` impl block:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HatcheryStatus {
    Online,
    Degraded,
    Offline,
}

impl fmt::Display for HatcheryStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Online => write!(f, "online"),
            Self::Degraded => write!(f, "degraded"),
            Self::Offline => write!(f, "offline"),
        }
    }
}

impl FromStr for HatcheryStatus {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "online" => Ok(Self::Online),
            "degraded" => Ok(Self::Degraded),
            "offline" => Ok(Self::Offline),
            other => Err(format!("invalid hatchery status: {other}")),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Hatchery {
    pub id: String,
    pub name: String,
    pub status: HatcheryStatus,
    pub capabilities: serde_json::Value,
    pub max_concurrency: i32,
    pub active_drones: i32,
    pub last_heartbeat_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

- [ ] **Step 4: Add Hatcheries table enum**

Add to `src/overseer/src/db/tables.rs`:

```rust
#[derive(Iden)]
pub enum Hatcheries {
    Table,
    Id,
    Name,
    Status,
    Capabilities,
    MaxConcurrency,
    ActiveDrones,
    LastHeartbeatAt,
    CreatedAt,
    UpdatedAt,
}
```

- [ ] **Step 5: Add HatcheryId column to JobRuns enum**

In `src/overseer/src/db/tables.rs`, add `HatcheryId` to the existing `JobRuns` enum:

```rust
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
    HatcheryId,
}
```

- [ ] **Step 6: Run tests**

Run: `cd src/overseer && cargo test db::models::tests`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src/overseer/src/db/models.rs src/overseer/src/db/tables.rs
git commit -m "feat(overseer): add Hatchery model, HatcheryStatus enum, and table enums"
```

---

### Task 3: HatcheryStore trait

**Files:**
- Modify: `src/overseer/src/db/trait_def.rs`

- [ ] **Step 1: Add HatcheryStore trait**

Add after the `ArtifactStore` trait in `src/overseer/src/db/trait_def.rs`:

```rust
#[async_trait]
pub trait HatcheryStore: Send + Sync {
    async fn register_hatchery(
        &self,
        name: &str,
        capabilities: serde_json::Value,
        max_concurrency: i32,
    ) -> Result<Hatchery>;

    async fn get_hatchery(&self, id: &str) -> Result<Option<Hatchery>>;

    async fn get_hatchery_by_name(&self, name: &str) -> Result<Option<Hatchery>>;

    async fn heartbeat_hatchery(
        &self,
        id: &str,
        status: &str,
        active_drones: i32,
    ) -> Result<Hatchery>;

    async fn list_hatcheries(&self, status: Option<&str>) -> Result<Vec<Hatchery>>;

    async fn deregister_hatchery(&self, id: &str) -> Result<()>;

    async fn assign_job_to_hatchery(
        &self,
        job_run_id: &str,
        hatchery_id: &str,
    ) -> Result<JobRun>;

    async fn list_hatchery_job_runs(
        &self,
        hatchery_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<JobRun>>;
}
```

- [ ] **Step 2: Update Database supertrait**

Change the `Database` trait and blanket impl:

```rust
pub trait Database: MemoryStore + JobStore + DecisionStore + ArtifactStore + HatcheryStore {}
impl<T: MemoryStore + JobStore + DecisionStore + ArtifactStore + HatcheryStore> Database for T {}
```

- [ ] **Step 3: Verify it compiles (expect errors in sqlite.rs/postgres.rs)**

Run: `cd src/overseer && cargo check 2>&1 | head -20`
Expected: compile errors — `SqliteDatabase` and `PostgresDatabase` don't impl `HatcheryStore` yet. This confirms the trait is wired in.

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/db/trait_def.rs
git commit -m "feat(overseer): add HatcheryStore trait to Database supertrait"
```

---

### Task 4: SQLite hatchery database implementation

**Files:**
- Create: `src/overseer/src/db/hatcheries.rs`
- Modify: `src/overseer/src/db/mod.rs`
- Modify: `src/overseer/src/db/sqlite.rs`

- [ ] **Step 1: Write failing tests for hatchery CRUD**

Create `src/overseer/src/db/hatcheries.rs` with tests first:

```rust
use chrono::NaiveDateTime;
use sea_query::{Expr, Order, Query, SqliteQueryBuilder};
use sea_query_binder::SqlxBinder;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

pub use super::models::{Hatchery, HatcheryStatus, JobRun, JobRunStatus};
use super::tables::{Hatcheries, JobRuns};
use crate::error::{OverseerError, Result};

fn row_to_hatchery(row: &sqlx::sqlite::SqliteRow) -> Hatchery {
    let caps_json: String = row.get("capabilities");
    let capabilities: serde_json::Value =
        serde_json::from_str(&caps_json).unwrap_or(serde_json::Value::Object(Default::default()));
    Hatchery {
        id: row.get("id"),
        name: row.get("name"),
        status: row
            .get::<String, _>("status")
            .parse()
            .unwrap_or(HatcheryStatus::Offline),
        capabilities,
        max_concurrency: row.get("max_concurrency"),
        active_drones: row.get("active_drones"),
        last_heartbeat_at: row.get::<NaiveDateTime, _>("last_heartbeat_at").and_utc(),
        created_at: row.get::<NaiveDateTime, _>("created_at").and_utc(),
        updated_at: row.get::<NaiveDateTime, _>("updated_at").and_utc(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory_named;

    async fn make_pool(name: &str) -> SqlitePool {
        open_in_memory_named(name).await.expect("pool opens")
    }

    #[tokio::test]
    async fn test_register_and_get_hatchery() {
        let pool = make_pool("hatchery_register").await;
        let h = register_hatchery(
            &pool,
            "rpi-1",
            serde_json::json!({"arch": "aarch64", "drone_types": ["claude/code-reviewer"]}),
            2,
        )
        .await
        .expect("register");

        assert_eq!(h.name, "rpi-1");
        assert_eq!(h.status, HatcheryStatus::Online);
        assert_eq!(h.max_concurrency, 2);
        assert_eq!(h.active_drones, 0);

        let fetched = get_hatchery(&pool, &h.id)
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(fetched.id, h.id);

        let by_name = get_hatchery_by_name(&pool, "rpi-1")
            .await
            .expect("get by name")
            .expect("exists");
        assert_eq!(by_name.id, h.id);
    }

    #[tokio::test]
    async fn test_heartbeat_updates_status() {
        let pool = make_pool("hatchery_heartbeat").await;
        let h = register_hatchery(&pool, "hb-test", serde_json::json!({}), 4)
            .await
            .expect("register");

        let updated = heartbeat_hatchery(&pool, &h.id, "degraded", 2)
            .await
            .expect("heartbeat");
        assert_eq!(updated.status, HatcheryStatus::Degraded);
        assert_eq!(updated.active_drones, 2);
        assert!(updated.last_heartbeat_at >= h.last_heartbeat_at);
    }

    #[tokio::test]
    async fn test_list_hatcheries_with_filter() {
        let pool = make_pool("hatchery_list").await;
        register_hatchery(&pool, "list-a", serde_json::json!({}), 1)
            .await
            .expect("register a");
        let b = register_hatchery(&pool, "list-b", serde_json::json!({}), 1)
            .await
            .expect("register b");
        heartbeat_hatchery(&pool, &b.id, "offline", 0)
            .await
            .expect("set offline");

        let all = list_hatcheries(&pool, None).await.expect("list all");
        assert_eq!(all.len(), 2);

        let online = list_hatcheries(&pool, Some("online"))
            .await
            .expect("list online");
        assert_eq!(online.len(), 1);
        assert_eq!(online[0].name, "list-a");
    }

    #[tokio::test]
    async fn test_deregister_hatchery() {
        let pool = make_pool("hatchery_deregister").await;
        let h = register_hatchery(&pool, "del-me", serde_json::json!({}), 1)
            .await
            .expect("register");
        deregister_hatchery(&pool, &h.id)
            .await
            .expect("deregister");

        let fetched = get_hatchery(&pool, &h.id).await.expect("get");
        assert!(fetched.is_none());
    }

    #[tokio::test]
    async fn test_assign_job_to_hatchery() {
        let pool = make_pool("hatchery_assign_job").await;
        let h = register_hatchery(&pool, "assign-test", serde_json::json!({}), 2)
            .await
            .expect("register");

        // Create a job definition and run
        let def = crate::db::jobs::create_job_definition(
            &pool,
            "assign-job-def",
            "test",
            serde_json::json!({}),
        )
        .await
        .expect("create def");
        let run = crate::db::jobs::start_job_run(&pool, &def.id, "queen", None)
            .await
            .expect("start run");

        let assigned = assign_job_to_hatchery(&pool, &run.id, &h.id)
            .await
            .expect("assign");
        assert_eq!(assigned.id, run.id);

        let runs = list_hatchery_job_runs(&pool, &h.id, None)
            .await
            .expect("list runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, run.id);
    }

    #[tokio::test]
    async fn test_heartbeat_nonexistent() {
        let pool = make_pool("hatchery_hb_notfound").await;
        let result = heartbeat_hatchery(&pool, "no-such-id", "online", 0).await;
        assert!(matches!(result, Err(OverseerError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_duplicate_name_rejected() {
        let pool = make_pool("hatchery_dup_name").await;
        register_hatchery(&pool, "dup", serde_json::json!({}), 1)
            .await
            .expect("first");
        let result = register_hatchery(&pool, "dup", serde_json::json!({}), 1).await;
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/overseer && cargo test db::hatcheries::tests 2>&1 | head -5`
Expected: FAIL — functions `register_hatchery`, etc. don't exist yet

- [ ] **Step 3: Implement hatchery CRUD functions**

Add the implementation functions to `src/overseer/src/db/hatcheries.rs` (above the `#[cfg(test)]` block):

```rust
pub async fn register_hatchery(
    pool: &SqlitePool,
    name: &str,
    capabilities: serde_json::Value,
    max_concurrency: i32,
) -> Result<Hatchery> {
    let id = Uuid::new_v4().to_string();
    let caps_json =
        serde_json::to_string(&capabilities).map_err(|e| OverseerError::Internal(e.to_string()))?;

    let (sql, values) = Query::insert()
        .into_table(Hatcheries::Table)
        .columns([
            Hatcheries::Id,
            Hatcheries::Name,
            Hatcheries::Capabilities,
            Hatcheries::MaxConcurrency,
        ])
        .values([
            id.into(),
            name.into(),
            caps_json.into(),
            max_concurrency.into(),
        ])
        .map_err(|e| OverseerError::Internal(format!("query build error: {e}")))?
        .returning(Query::returning().columns([
            Hatcheries::Id,
            Hatcheries::Name,
            Hatcheries::Status,
            Hatcheries::Capabilities,
            Hatcheries::MaxConcurrency,
            Hatcheries::ActiveDrones,
            Hatcheries::LastHeartbeatAt,
            Hatcheries::CreatedAt,
            Hatcheries::UpdatedAt,
        ]))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_one(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row_to_hatchery(&row))
}

pub async fn get_hatchery(pool: &SqlitePool, id: &str) -> Result<Option<Hatchery>> {
    let (sql, values) = Query::select()
        .columns([
            Hatcheries::Id,
            Hatcheries::Name,
            Hatcheries::Status,
            Hatcheries::Capabilities,
            Hatcheries::MaxConcurrency,
            Hatcheries::ActiveDrones,
            Hatcheries::LastHeartbeatAt,
            Hatcheries::CreatedAt,
            Hatcheries::UpdatedAt,
        ])
        .from(Hatcheries::Table)
        .and_where(Expr::col(Hatcheries::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row.as_ref().map(row_to_hatchery))
}

pub async fn get_hatchery_by_name(pool: &SqlitePool, name: &str) -> Result<Option<Hatchery>> {
    let (sql, values) = Query::select()
        .columns([
            Hatcheries::Id,
            Hatcheries::Name,
            Hatcheries::Status,
            Hatcheries::Capabilities,
            Hatcheries::MaxConcurrency,
            Hatcheries::ActiveDrones,
            Hatcheries::LastHeartbeatAt,
            Hatcheries::CreatedAt,
            Hatcheries::UpdatedAt,
        ])
        .from(Hatcheries::Table)
        .and_where(Expr::col(Hatcheries::Name).eq(name))
        .build_sqlx(SqliteQueryBuilder);

    let row = sqlx::query_with(&sql, values)
        .fetch_optional(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(row.as_ref().map(row_to_hatchery))
}

pub async fn heartbeat_hatchery(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    active_drones: i32,
) -> Result<Hatchery> {
    let _parsed: HatcheryStatus = status
        .parse()
        .map_err(OverseerError::Validation)?;

    let (sql, values) = Query::update()
        .table(Hatcheries::Table)
        .value(Hatcheries::Status, status)
        .value(Hatcheries::ActiveDrones, active_drones)
        .value(Hatcheries::LastHeartbeatAt, Expr::cust("datetime('now')"))
        .value(Hatcheries::UpdatedAt, Expr::cust("datetime('now')"))
        .and_where(Expr::col(Hatcheries::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    let result = sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    if result.rows_affected() == 0 {
        return Err(OverseerError::NotFound(format!("hatchery {id}")));
    }

    get_hatchery(pool, id)
        .await?
        .ok_or_else(|| OverseerError::NotFound(format!("hatchery {id}")))
}

pub async fn list_hatcheries(pool: &SqlitePool, status: Option<&str>) -> Result<Vec<Hatchery>> {
    let mut query = Query::select();
    query
        .columns([
            Hatcheries::Id,
            Hatcheries::Name,
            Hatcheries::Status,
            Hatcheries::Capabilities,
            Hatcheries::MaxConcurrency,
            Hatcheries::ActiveDrones,
            Hatcheries::LastHeartbeatAt,
            Hatcheries::CreatedAt,
            Hatcheries::UpdatedAt,
        ])
        .from(Hatcheries::Table);

    if let Some(s) = status {
        query.and_where(Expr::col(Hatcheries::Status).eq(s));
    }

    query.order_by(Hatcheries::Name, Order::Asc);

    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);

    let rows = sqlx::query_with(&sql, values)
        .fetch_all(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(rows.iter().map(row_to_hatchery).collect())
}

pub async fn deregister_hatchery(pool: &SqlitePool, id: &str) -> Result<()> {
    let (sql, values) = Query::delete()
        .from_table(Hatcheries::Table)
        .and_where(Expr::col(Hatcheries::Id).eq(id))
        .build_sqlx(SqliteQueryBuilder);

    sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(())
}

pub async fn assign_job_to_hatchery(
    pool: &SqlitePool,
    job_run_id: &str,
    hatchery_id: &str,
) -> Result<JobRun> {
    let (sql, values) = Query::update()
        .table(JobRuns::Table)
        .value(JobRuns::HatcheryId, hatchery_id)
        .and_where(Expr::col(JobRuns::Id).eq(job_run_id))
        .build_sqlx(SqliteQueryBuilder);

    let result = sqlx::query_with(&sql, values)
        .execute(pool)
        .await
        .map_err(OverseerError::Storage)?;

    if result.rows_affected() == 0 {
        return Err(OverseerError::NotFound(format!("job_run {job_run_id}")));
    }

    crate::db::jobs::get_job_run(pool, job_run_id)
        .await?
        .ok_or_else(|| OverseerError::NotFound(format!("job_run {job_run_id}")))
}

pub async fn list_hatchery_job_runs(
    pool: &SqlitePool,
    hatchery_id: &str,
    status: Option<&str>,
) -> Result<Vec<JobRun>> {
    let mut query = Query::select();
    query
        .columns([
            JobRuns::Id,
            JobRuns::DefinitionId,
            JobRuns::ParentId,
            JobRuns::Status,
            JobRuns::TriggeredBy,
            JobRuns::Result,
            JobRuns::Error,
            JobRuns::StartedAt,
            JobRuns::CompletedAt,
        ])
        .from(JobRuns::Table)
        .and_where(Expr::col(JobRuns::HatcheryId).eq(hatchery_id));

    if let Some(s) = status {
        query.and_where(Expr::col(JobRuns::Status).eq(s));
    }

    query.order_by(JobRuns::StartedAt, Order::Asc);

    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);

    let rows = sqlx::query_with(&sql, values)
        .fetch_all(pool)
        .await
        .map_err(OverseerError::Storage)?;

    Ok(rows.iter().map(crate::db::jobs::row_to_job_run).collect())
}
```

- [ ] **Step 4: Export hatcheries module from db/mod.rs**

Add `pub mod hatcheries;` to `src/overseer/src/db/mod.rs` after the existing module declarations.

- [ ] **Step 5: Make `row_to_job_run` public in db/jobs.rs**

In `src/overseer/src/db/jobs.rs`, change:
```rust
fn row_to_job_run(row: &sqlx::sqlite::SqliteRow) -> JobRun {
```
to:
```rust
pub fn row_to_job_run(row: &sqlx::sqlite::SqliteRow) -> JobRun {
```

- [ ] **Step 6: Implement HatcheryStore for SqliteDatabase**

Add to `src/overseer/src/db/sqlite.rs`:

```rust
#[async_trait]
impl HatcheryStore for SqliteDatabase {
    async fn register_hatchery(
        &self,
        name: &str,
        capabilities: serde_json::Value,
        max_concurrency: i32,
    ) -> Result<Hatchery> {
        super::hatcheries::register_hatchery(&self.pool, name, capabilities, max_concurrency).await
    }

    async fn get_hatchery(&self, id: &str) -> Result<Option<Hatchery>> {
        super::hatcheries::get_hatchery(&self.pool, id).await
    }

    async fn get_hatchery_by_name(&self, name: &str) -> Result<Option<Hatchery>> {
        super::hatcheries::get_hatchery_by_name(&self.pool, name).await
    }

    async fn heartbeat_hatchery(
        &self,
        id: &str,
        status: &str,
        active_drones: i32,
    ) -> Result<Hatchery> {
        super::hatcheries::heartbeat_hatchery(&self.pool, id, status, active_drones).await
    }

    async fn list_hatcheries(&self, status: Option<&str>) -> Result<Vec<Hatchery>> {
        super::hatcheries::list_hatcheries(&self.pool, status).await
    }

    async fn deregister_hatchery(&self, id: &str) -> Result<()> {
        super::hatcheries::deregister_hatchery(&self.pool, id).await
    }

    async fn assign_job_to_hatchery(
        &self,
        job_run_id: &str,
        hatchery_id: &str,
    ) -> Result<JobRun> {
        super::hatcheries::assign_job_to_hatchery(&self.pool, job_run_id, hatchery_id).await
    }

    async fn list_hatchery_job_runs(
        &self,
        hatchery_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<JobRun>> {
        super::hatcheries::list_hatchery_job_runs(&self.pool, hatchery_id, status).await
    }
}
```

Also add the import at the top of sqlite.rs:
```rust
use super::trait_def::{ArtifactStore, DecisionStore, HatcheryStore, JobStore, MemoryStore};
```

- [ ] **Step 7: Stub HatcheryStore for PostgresDatabase**

In `src/overseer/src/db/postgres.rs`, add a stub implementation that returns `Err(OverseerError::Internal("not yet implemented".into()))` for all methods. This keeps the code compiling while Postgres support is deferred.

- [ ] **Step 8: Update trait_def.rs exports**

In `src/overseer/src/db/mod.rs`, update the re-export line:
```rust
pub use trait_def::{ArtifactStore, Database, DecisionStore, HatcheryStore, JobStore, MemoryStore};
```

- [ ] **Step 9: Add hatchery tests to trait conformance suite**

In `src/overseer/src/db/mod.rs`, add to the `trait_conformance_suite` function:

```rust
    // Hatcheries
    let hatchery = db
        .register_hatchery("conformance-hatchery", serde_json::json!({"arch": "x86_64"}), 4)
        .await
        .expect("register hatchery");
    assert_eq!(hatchery.name, "conformance-hatchery");
    assert_eq!(hatchery.status, models::HatcheryStatus::Online);

    let fetched_h = db.get_hatchery(&hatchery.id).await.expect("get hatchery");
    assert!(fetched_h.is_some());

    let by_name = db
        .get_hatchery_by_name("conformance-hatchery")
        .await
        .expect("get by name");
    assert!(by_name.is_some());

    let heartbeated = db
        .heartbeat_hatchery(&hatchery.id, "degraded", 2)
        .await
        .expect("heartbeat");
    assert_eq!(heartbeated.status, models::HatcheryStatus::Degraded);

    let hatcheries = db.list_hatcheries(None).await.expect("list hatcheries");
    assert!(!hatcheries.is_empty());

    // Assign a job to the hatchery
    let assigned = db
        .assign_job_to_hatchery(&run.id, &hatchery.id)
        .await
        .expect("assign job");
    assert_eq!(assigned.id, run.id);

    let h_runs = db
        .list_hatchery_job_runs(&hatchery.id, None)
        .await
        .expect("list hatchery runs");
    assert!(!h_runs.is_empty());

    db.deregister_hatchery(&hatchery.id)
        .await
        .expect("deregister");
    let gone = db.get_hatchery(&hatchery.id).await.expect("get after delete");
    assert!(gone.is_none());
```

- [ ] **Step 10: Run all tests**

Run: `cd src/overseer && cargo test`
Expected: ALL PASS

- [ ] **Step 11: Commit**

```bash
git add src/overseer/src/db/
git commit -m "feat(overseer): implement HatcheryStore for SQLite with full CRUD and job assignment"
```

---

### Task 5: Hatchery service layer

**Files:**
- Create: `src/overseer/src/services/hatchery.rs`
- Modify: `src/overseer/src/services/mod.rs`

- [ ] **Step 1: Write failing test for HatcheryService**

Create `src/overseer/src/services/hatchery.rs`:

```rust
use std::sync::Arc;

use crate::db::Database;
use crate::db::models::{Hatchery, JobRun};
use crate::error::Result;

pub struct HatcheryService {
    db: Arc<dyn Database>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDatabase;

    #[tokio::test]
    async fn test_hatchery_service_register_and_heartbeat() {
        let sqlite_db = SqliteDatabase::open_in_memory_named("svc_hatchery_test")
            .await
            .expect("db opens");
        let svc = HatcheryService::new(Arc::new(sqlite_db));

        let h = svc
            .register("test-svc-hatchery", serde_json::json!({"arch": "x86_64"}), 4)
            .await
            .expect("register");
        assert_eq!(h.name, "test-svc-hatchery");

        let updated = svc
            .heartbeat(&h.id, "online", 1)
            .await
            .expect("heartbeat");
        assert_eq!(updated.active_drones, 1);

        let all = svc.list(None).await.expect("list");
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn test_hatchery_service_deregister() {
        let sqlite_db = SqliteDatabase::open_in_memory_named("svc_hatchery_dereg")
            .await
            .expect("db opens");
        let svc = HatcheryService::new(Arc::new(sqlite_db));

        let h = svc
            .register("dereg-svc", serde_json::json!({}), 1)
            .await
            .expect("register");

        svc.deregister(&h.id).await.expect("deregister");

        let all = svc.list(None).await.expect("list");
        assert!(all.is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/overseer && cargo test services::hatchery::tests`
Expected: FAIL — `HatcheryService::new`, `register`, etc. don't exist

- [ ] **Step 3: Implement HatcheryService**

Add the implementation above the tests in `src/overseer/src/services/hatchery.rs`:

```rust
impl HatcheryService {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }

    pub async fn register(
        &self,
        name: &str,
        capabilities: serde_json::Value,
        max_concurrency: i32,
    ) -> Result<Hatchery> {
        self.db
            .register_hatchery(name, capabilities, max_concurrency)
            .await
    }

    pub async fn get(&self, id: &str) -> Result<Option<Hatchery>> {
        self.db.get_hatchery(id).await
    }

    pub async fn get_by_name(&self, name: &str) -> Result<Option<Hatchery>> {
        self.db.get_hatchery_by_name(name).await
    }

    pub async fn heartbeat(
        &self,
        id: &str,
        status: &str,
        active_drones: i32,
    ) -> Result<Hatchery> {
        self.db
            .heartbeat_hatchery(id, status, active_drones)
            .await
    }

    pub async fn list(&self, status: Option<&str>) -> Result<Vec<Hatchery>> {
        self.db.list_hatcheries(status).await
    }

    pub async fn deregister(&self, id: &str) -> Result<()> {
        self.db.deregister_hatchery(id).await
    }

    pub async fn assign_job(
        &self,
        job_run_id: &str,
        hatchery_id: &str,
    ) -> Result<JobRun> {
        self.db
            .assign_job_to_hatchery(job_run_id, hatchery_id)
            .await
    }

    pub async fn list_job_runs(
        &self,
        hatchery_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<JobRun>> {
        self.db
            .list_hatchery_job_runs(hatchery_id, status)
            .await
    }
}
```

- [ ] **Step 4: Wire into AppState**

In `src/overseer/src/services/mod.rs`, add:

```rust
pub mod hatchery;
```

And update `AppState`:

```rust
pub struct AppState {
    pub memory: memory::MemoryService,
    pub jobs: jobs::JobService,
    pub decisions: decisions::DecisionService,
    pub artifacts: artifacts::ArtifactService,
    pub hatchery: hatchery::HatcheryService,
}

impl AppState {
    pub fn new(
        db: Arc<dyn Database>,
        registry: EmbeddingRegistry,
        store: Arc<dyn ObjectStore>,
    ) -> Self {
        Self {
            memory: memory::MemoryService::new(db.clone(), registry),
            jobs: jobs::JobService::new(db.clone()),
            decisions: decisions::DecisionService::new(db.clone()),
            artifacts: artifacts::ArtifactService::new(db.clone(), store),
            hatchery: hatchery::HatcheryService::new(db),
        }
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cd src/overseer && cargo test`
Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add src/overseer/src/services/
git commit -m "feat(overseer): add HatcheryService and wire into AppState"
```

---

### Task 6: HTTP API endpoints

**Files:**
- Create: `src/overseer/src/api/hatchery.rs`
- Modify: `src/overseer/src/api/mod.rs`

- [ ] **Step 1: Create hatchery API module**

Create `src/overseer/src/api/hatchery.rs`:

```rust
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, patch, post},
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::error::Result;
use crate::services::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", post(register_hatchery))
        .route("/", get(list_hatcheries))
        .route("/{id}", get(get_hatchery))
        .route("/{id}/heartbeat", post(heartbeat_hatchery))
        .route("/{id}", delete(deregister_hatchery))
        .route("/{id}/jobs", get(list_hatchery_jobs))
        .route("/{id}/jobs/{job_run_id}", post(assign_job))
}

#[derive(Deserialize)]
struct RegisterHatcheryRequest {
    name: String,
    capabilities: Option<Value>,
    max_concurrency: Option<i32>,
}

async fn register_hatchery(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RegisterHatcheryRequest>,
) -> Result<Json<Value>> {
    let capabilities = body.capabilities.unwrap_or(serde_json::json!({}));
    let max_concurrency = body.max_concurrency.unwrap_or(1);
    let result = state
        .hatchery
        .register(&body.name, capabilities, max_concurrency)
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

async fn get_hatchery(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let result = state
        .hatchery
        .get(&id)
        .await?
        .ok_or_else(|| crate::error::OverseerError::NotFound(format!("hatchery {id}")))?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

#[derive(Deserialize)]
struct ListHatcheriesQuery {
    status: Option<String>,
}

async fn list_hatcheries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListHatcheriesQuery>,
) -> Result<Json<Value>> {
    let results = state
        .hatchery
        .list(params.status.as_deref())
        .await?;
    Ok(Json(serde_json::to_value(results).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

#[derive(Deserialize)]
struct HeartbeatRequest {
    status: String,
    active_drones: i32,
}

async fn heartbeat_hatchery(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<HeartbeatRequest>,
) -> Result<Json<Value>> {
    let result = state
        .hatchery
        .heartbeat(&id, &body.status, body.active_drones)
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

async fn deregister_hatchery(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    state.hatchery.deregister(&id).await?;
    Ok(Json(serde_json::json!({"deleted": true})))
}

#[derive(Deserialize)]
struct ListHatcheryJobsQuery {
    status: Option<String>,
}

async fn list_hatchery_jobs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<ListHatcheryJobsQuery>,
) -> Result<Json<Value>> {
    let results = state
        .hatchery
        .list_job_runs(&id, params.status.as_deref())
        .await?;
    Ok(Json(serde_json::to_value(results).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

async fn assign_job(
    State(state): State<Arc<AppState>>,
    Path((id, job_run_id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    let result = state
        .hatchery
        .assign_job(&job_run_id, &id)
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}
```

- [ ] **Step 2: Wire into API router**

In `src/overseer/src/api/mod.rs`, add:

```rust
mod hatchery;
```

And update the router:

```rust
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .nest("/api/memories", memory::router())
        .nest("/api/decisions", decisions::router())
        .nest("/api/jobs", jobs::router())
        .nest("/api/tasks", jobs::task_router())
        .nest("/api/artifacts", artifacts::router())
        .nest("/api/hatcheries", hatchery::router())
        .with_state(state)
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cd src/overseer && cargo check`
Expected: compiles

- [ ] **Step 4: Run all tests**

Run: `cd src/overseer && cargo test`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/overseer/src/api/
git commit -m "feat(overseer): add hatchery HTTP API endpoints"
```

---

### Task 7: MCP tools for hatchery management

**Files:**
- Modify: `src/overseer/src/mcp/mod.rs`

- [ ] **Step 1: Add MCP parameter structs**

Add to `src/overseer/src/mcp/mod.rs` after the existing parameter structs:

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RegisterHatcheryParams {
    #[schemars(description = "Unique name for this hatchery instance")]
    pub name: String,
    #[serde(default = "serde_json::Value::default")]
    #[schemars(description = "JSON describing available architectures, drone types, etc.")]
    pub capabilities: serde_json::Value,
    #[serde(default = "default_max_concurrency")]
    #[schemars(description = "Maximum number of concurrent drones this hatchery can run (default 1)")]
    pub max_concurrency: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct HeartbeatHatcheryParams {
    #[schemars(description = "Hatchery ID")]
    pub id: String,
    #[schemars(description = "Current status: online, degraded, or offline")]
    pub status: String,
    #[schemars(description = "Number of currently running drone sessions")]
    pub active_drones: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListHatcheriesParams {
    #[schemars(description = "Filter by status (online, degraded, offline)")]
    pub status: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeregisterHatcheryParams {
    #[schemars(description = "Hatchery ID to deregister")]
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AssignJobToHatcheryParams {
    #[schemars(description = "Job run ID to assign")]
    pub job_run_id: String,
    #[schemars(description = "Hatchery ID to assign the job to")]
    pub hatchery_id: String,
}
```

Add the default function:

```rust
fn default_max_concurrency() -> i32 {
    1
}
```

- [ ] **Step 2: Add MCP tool methods**

Add to the `#[tool_router] impl OverseerMcp` block:

```rust
    // ── hatcheries ─────────────────────────────────────────────────────────

    #[tool(description = "Register a new hatchery instance with Overseer")]
    async fn register_hatchery(
        &self,
        Parameters(p): Parameters<RegisterHatcheryParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .state
            .hatchery
            .register(&p.name, p.capabilities, p.max_concurrency)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Send a heartbeat from a hatchery, updating its status and active drone count")]
    async fn heartbeat_hatchery(
        &self,
        Parameters(p): Parameters<HeartbeatHatcheryParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .state
            .hatchery
            .heartbeat(&p.id, &p.status, p.active_drones)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List registered hatcheries, optionally filtered by status")]
    async fn list_hatcheries(
        &self,
        Parameters(p): Parameters<ListHatcheriesParams>,
    ) -> Result<CallToolResult, McpError> {
        let results = self
            .state
            .hatchery
            .list(p.status.as_deref())
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&results).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Deregister a hatchery instance")]
    async fn deregister_hatchery(
        &self,
        Parameters(p): Parameters<DeregisterHatcheryParams>,
    ) -> Result<CallToolResult, McpError> {
        self.state
            .hatchery
            .deregister(&p.id)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Hatchery {} deregistered",
            p.id
        ))]))
    }

    #[tool(description = "Assign a job run to a specific hatchery for execution")]
    async fn assign_job_to_hatchery(
        &self,
        Parameters(p): Parameters<AssignJobToHatcheryParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .state
            .hatchery
            .assign_job(&p.job_run_id, &p.hatchery_id)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
```

- [ ] **Step 3: Verify it compiles**

Run: `cd src/overseer && cargo check`
Expected: compiles

- [ ] **Step 4: Run all tests**

Run: `cd src/overseer && cargo test`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/overseer/src/mcp/mod.rs
git commit -m "feat(overseer): add hatchery MCP tools (register, heartbeat, list, deregister, assign)"
```

---

### Task 8: Buck2 build verification

**Files:**
- None modified — verification only

- [ ] **Step 1: Build with Buck2**

Run: `buck2 build root//src/overseer:overseer`
Expected: BUILD SUCCEEDED

- [ ] **Step 2: Run clippy via Buck2**

Run: `buck2 build 'root//src/overseer:overseer[clippy.txt]'`
Expected: No warnings or errors in the output file

- [ ] **Step 3: Run cargo tests**

Run: `cd src/overseer && cargo test`
Expected: ALL PASS

- [ ] **Step 4: Run pre-commit hooks**

Run: `buck2 run root//tools:prek -- run --all-files`
Expected: All hooks pass

- [ ] **Step 5: Commit any formatting fixes**

If `cargo fmt` made changes during the hook run:

```bash
git add -u
git commit -m "style: apply cargo fmt formatting"
```
