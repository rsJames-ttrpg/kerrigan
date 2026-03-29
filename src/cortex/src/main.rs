mod config;
mod db;
mod embedding;
mod error;

use config::Config;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cortex.toml"));

    let config = Config::load(&config_path)?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.logging.level)),
        )
        .init();

    tracing::info!("cortex starting");
    tracing::info!("config loaded from {:?}", config_path);

    // TODO: DB init, services, HTTP + MCP servers will be wired in subsequent tasks

    Ok(())
}
