pub mod artifacts;
pub mod decisions;
pub mod jobs;
pub mod memory;

use std::path::PathBuf;
use std::sync::Arc;

use crate::db::Database;
use crate::embedding::EmbeddingRegistry;

pub struct AppState {
    pub memory: memory::MemoryService,
    pub jobs: jobs::JobService,
    pub decisions: decisions::DecisionService,
    pub artifacts: artifacts::ArtifactService,
}

impl AppState {
    pub fn new(db: Arc<dyn Database>, registry: EmbeddingRegistry, artifact_path: PathBuf) -> Self {
        Self {
            memory: memory::MemoryService::new(db.clone(), registry),
            jobs: jobs::JobService::new(db.clone()),
            decisions: decisions::DecisionService::new(db.clone()),
            artifacts: artifacts::ArtifactService::new(db, artifact_path),
        }
    }
}
