pub mod artifacts;
pub mod decisions;
pub mod jobs;
pub mod memory;

use sqlx::SqlitePool;
use std::sync::Arc;

use crate::embedding::EmbeddingProvider;

pub struct AppState {
    pub memory: memory::MemoryService,
    pub jobs: jobs::JobService,
    pub decisions: decisions::DecisionService,
    pub artifacts: artifacts::ArtifactService,
}

impl AppState {
    pub fn new(pool: SqlitePool, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self {
            memory: memory::MemoryService::new(pool.clone(), embedder),
            jobs: jobs::JobService::new(pool.clone()),
            decisions: decisions::DecisionService::new(pool.clone()),
            artifacts: artifacts::ArtifactService::new(pool),
        }
    }
}
