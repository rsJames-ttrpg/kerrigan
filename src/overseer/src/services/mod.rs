pub mod artifacts;
pub mod auth;
pub mod credentials;
pub mod decisions;
pub mod hatchery;
pub mod jobs;
pub mod memory;
pub mod pipeline;

use std::sync::Arc;

use object_store::ObjectStore;

use crate::db::Database;
use crate::embedding::EmbeddingRegistry;

pub struct AppState {
    pub memory: memory::MemoryService,
    pub jobs: jobs::JobService,
    pub decisions: decisions::DecisionService,
    pub artifacts: artifacts::ArtifactService,
    pub pipeline: pipeline::PipelineService,
    pub hatchery: hatchery::HatcheryService,
    pub auth: auth::AuthService,
    pub credentials: credentials::CredentialService,
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
            pipeline: pipeline::PipelineService::new(db.clone()),
            hatchery: hatchery::HatcheryService::new(db.clone()),
            auth: auth::AuthService::new(),
            credentials: credentials::CredentialService::new(db),
        }
    }
}
