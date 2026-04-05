mod cache;
mod config;
mod drone;
mod event_bridge;
mod exit_conditions;
mod git_workflow;
mod health;
mod pipeline;
mod prompt;
mod resolve;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tracing::info!("Native drone starting");
    drone_sdk::harness::run(drone::NativeDrone).await
}
