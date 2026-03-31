use async_trait::async_trait;

use super::models::*;
use crate::error::Result;

#[async_trait]
pub trait MemoryStore: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    async fn insert_memory(
        &self,
        provider_name: &str,
        content: &str,
        embedding: &[f32],
        embedding_model: &str,
        source: &str,
        tags: &[String],
        expires_at: Option<&str>,
    ) -> Result<Memory>;

    async fn get_memory(&self, id: &str) -> Result<Option<Memory>>;

    async fn delete_memory(&self, provider_name: &str, id: &str) -> Result<()>;

    async fn search_memories(
        &self,
        provider_name: &str,
        query_embedding: &[f32],
        tags_filter: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>>;

    async fn insert_memory_link(
        &self,
        memory_id: &str,
        linked_id: &str,
        linked_type: &str,
        relation_type: &str,
    ) -> Result<()>;

    async fn create_embedding_table(&self, provider_name: &str, dimensions: usize) -> Result<()>;
}

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn create_job_definition(
        &self,
        name: &str,
        description: &str,
        config: serde_json::Value,
    ) -> Result<JobDefinition>;

    async fn get_job_definition(&self, id: &str) -> Result<Option<JobDefinition>>;

    async fn list_job_definitions(&self) -> Result<Vec<JobDefinition>>;

    async fn start_job_run(
        &self,
        definition_id: &str,
        triggered_by: &str,
        parent_id: Option<&str>,
    ) -> Result<JobRun>;

    async fn get_job_run(&self, id: &str) -> Result<Option<JobRun>>;

    async fn update_job_run(
        &self,
        id: &str,
        status: Option<&str>,
        result: Option<serde_json::Value>,
        error: Option<&str>,
    ) -> Result<JobRun>;

    async fn list_job_runs(&self, status: Option<&str>) -> Result<Vec<JobRun>>;

    async fn create_task(
        &self,
        subject: &str,
        run_id: Option<&str>,
        assigned_to: Option<&str>,
    ) -> Result<Task>;

    async fn get_task(&self, id: &str) -> Result<Option<Task>>;

    async fn update_task(
        &self,
        id: &str,
        status: Option<&str>,
        assigned_to: Option<&str>,
        output: Option<serde_json::Value>,
    ) -> Result<Task>;

    async fn list_tasks(
        &self,
        status: Option<&str>,
        assigned_to: Option<&str>,
        run_id: Option<&str>,
    ) -> Result<Vec<Task>>;
}

#[async_trait]
pub trait DecisionStore: Send + Sync {
    async fn log_decision(
        &self,
        agent: &str,
        context: &str,
        decision: &str,
        reasoning: &str,
        tags: &[String],
        run_id: Option<&str>,
    ) -> Result<Decision>;

    async fn get_decision(&self, id: &str) -> Result<Option<Decision>>;

    async fn query_decisions(
        &self,
        agent: Option<&str>,
        tags: Option<&[String]>,
        limit: i64,
    ) -> Result<Vec<Decision>>;
}

#[async_trait]
pub trait ArtifactStore: Send + Sync {
    async fn insert_artifact(
        &self,
        id: &str,
        name: &str,
        content_type: &str,
        size: i64,
        run_id: Option<&str>,
    ) -> Result<ArtifactMetadata>;

    async fn get_artifact(&self, id: &str) -> Result<Option<ArtifactMetadata>>;

    async fn list_artifacts(&self, run_id: Option<&str>) -> Result<Vec<ArtifactMetadata>>;
}

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

    async fn assign_job_to_hatchery(&self, job_run_id: &str, hatchery_id: &str) -> Result<JobRun>;

    async fn list_hatchery_job_runs(
        &self,
        hatchery_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<JobRun>>;
}

/// Convenience supertrait combining all domain stores.
/// Blanket-implemented for any type that implements all five.
pub trait Database: MemoryStore + JobStore + DecisionStore + ArtifactStore + HatcheryStore {}
impl<T: MemoryStore + JobStore + DecisionStore + ArtifactStore + HatcheryStore> Database for T {}
