use async_trait::async_trait;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;

use super::models::*;
use super::trait_def::{ArtifactStore, DecisionStore, HatcheryStore, JobStore, MemoryStore};
use crate::error::{OverseerError, Result};

/// Load the sqlite-vec extension into every new SQLite connection.
///
/// SAFETY: `sqlite3_vec_init` is the standard entry-point exported by the
/// sqlite-vec shared library.  `sqlite3_auto_extension` expects a function
/// pointer with the C signature `int(*)(sqlite3*,char**,const sqlite3_api_routines*)`,
/// but we register it via the opaque-pointer / transmute pattern that SQLite's
/// own extension-loading infrastructure uses.
fn register_vec_extension() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        #[allow(clippy::missing_transmute_annotations)]
        libsqlite3_sys::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

async fn init_pool(opts: SqliteConnectOptions) -> std::result::Result<SqlitePool, OverseerError> {
    register_vec_extension();

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .map_err(OverseerError::Storage)?;

    sqlx::migrate!("migrations/sqlite")
        .run(&pool)
        .await
        .map_err(|e| OverseerError::Storage(e.into()))?;

    Ok(pool)
}

/// A SQLite-backed implementation of the `Database` trait.
pub struct SqliteDatabase {
    pub(crate) pool: SqlitePool,
}

impl SqliteDatabase {
    /// Open (or create) a SQLite database at the given file path.
    pub async fn open(path: &str) -> std::result::Result<Self, OverseerError> {
        let opts = SqliteConnectOptions::from_str(&format!("sqlite:{path}"))
            .map_err(OverseerError::Storage)?
            .create_if_missing(true)
            .pragma("journal_mode", "WAL")
            .pragma("foreign_keys", "ON");

        Ok(Self {
            pool: init_pool(opts).await?,
        })
    }

    /// Open an in-memory SQLite database (useful for tests).
    pub async fn open_in_memory() -> std::result::Result<Self, OverseerError> {
        Self::open_in_memory_named("overseer_test").await
    }

    /// Open a named in-memory SQLite database. Each unique name gets its own
    /// isolated in-memory database, which is useful for test isolation.
    pub async fn open_in_memory_named(name: &str) -> std::result::Result<Self, OverseerError> {
        let opts =
            SqliteConnectOptions::from_str(&format!("sqlite:file:{name}?mode=memory&cache=shared"))
                .map_err(OverseerError::Storage)?
                .pragma("journal_mode", "WAL")
                .pragma("foreign_keys", "ON");

        Ok(Self {
            pool: init_pool(opts).await?,
        })
    }
}

#[async_trait]
impl MemoryStore for SqliteDatabase {
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
            &self.pool,
            provider_name,
            content,
            embedding,
            embedding_model,
            source,
            tags,
            expires_at,
        )
        .await
    }

    async fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
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
            &self.pool,
            provider_name,
            query_embedding,
            tags_filter,
            limit,
        )
        .await
    }

    async fn insert_memory_link(
        &self,
        memory_id: &str,
        linked_id: &str,
        linked_type: &str,
        relation_type: &str,
    ) -> Result<()> {
        super::memory::insert_memory_link(
            &self.pool,
            memory_id,
            linked_id,
            linked_type,
            relation_type,
        )
        .await
    }

    async fn create_embedding_table(&self, provider_name: &str, dimensions: usize) -> Result<()> {
        super::create_embedding_table(&self.pool, provider_name, dimensions).await
    }
}

#[async_trait]
impl JobStore for SqliteDatabase {
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
        config_overrides: Option<serde_json::Value>,
    ) -> Result<JobRun> {
        super::jobs::start_job_run(
            &self.pool,
            definition_id,
            triggered_by,
            parent_id,
            config_overrides,
        )
        .await
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

    async fn list_pending_unassigned_runs(&self) -> Result<Vec<JobRun>> {
        super::jobs::list_pending_unassigned_runs(&self.pool).await
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
}

#[async_trait]
impl DecisionStore for SqliteDatabase {
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
        )
        .await
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
}

#[async_trait]
impl ArtifactStore for SqliteDatabase {
    async fn insert_artifact(
        &self,
        id: &str,
        name: &str,
        content_type: &str,
        size: i64,
        run_id: Option<&str>,
        artifact_type: &ArtifactType,
    ) -> Result<ArtifactMetadata> {
        super::artifacts::insert_artifact(
            &self.pool,
            id,
            name,
            content_type,
            size,
            run_id,
            artifact_type,
        )
        .await
    }

    async fn get_artifact(&self, id: &str) -> Result<Option<ArtifactMetadata>> {
        super::artifacts::get_artifact(&self.pool, id).await
    }

    async fn list_artifacts(
        &self,
        filter: &crate::db::ArtifactFilter,
    ) -> Result<Vec<ArtifactMetadata>> {
        super::artifacts::list_artifacts(&self.pool, filter).await
    }
}

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

    async fn assign_job_to_hatchery(&self, job_run_id: &str, hatchery_id: &str) -> Result<JobRun> {
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
