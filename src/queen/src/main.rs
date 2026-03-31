mod config;

use anyhow::Result;
use clap::Parser;
use config::{Cli, Config};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut cfg = Config::load(&cli.config)?;
    cfg.apply_overrides(&cli);

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!(name = %cfg.queen.name, overseer_url = %cfg.queen.overseer_url, "queen starting");

    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");

    Ok(())
}
