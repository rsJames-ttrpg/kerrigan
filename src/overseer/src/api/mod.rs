mod artifacts;
mod auth;
mod decisions;
mod hatchery;
mod jobs;
mod memory;

use axum::Router;
use std::sync::Arc;

use crate::services::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .nest("/api/memories", memory::router())
        .nest("/api/decisions", decisions::router())
        .nest("/api/jobs", jobs::router())
        .nest("/api/tasks", jobs::task_router())
        .nest("/api/artifacts", artifacts::router())
        .nest("/api/hatcheries", hatchery::router())
        .nest("/api/jobs/runs", auth::router())
        .with_state(state)
}
