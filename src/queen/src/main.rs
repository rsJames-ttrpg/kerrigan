mod actors;
mod config;
mod messages;
mod notifier;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use config::{Cli, Config};
use notifier::log::LogNotifier;
use nydus::NydusClient;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let mut config = Config::load(&cli.config)?;
    config.apply_overrides(&cli);

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!(name = %config.queen.name, "queen starting");

    let client = NydusClient::new(config.queen.overseer_url.clone());
    let notifier: Arc<dyn notifier::Notifier> = Arc::new(LogNotifier);
    let hatchery_id: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));

    // 1. Register with Overseer (blocks until successful)
    let hatchery = actors::registrar::run(
        client.clone(),
        config.queen.name.clone(),
        config.queen.max_concurrency,
        notifier.clone(),
    )
    .await?;
    *hatchery_id.write().await = Some(hatchery.id.clone());

    // 2. Create cancellation token for graceful shutdown
    let token = CancellationToken::new();

    // 3. Start Creep in background (non-blocking)
    let creep_notifier = notifier.clone();
    let creep_config = config.creep;
    let creep_token = token.clone();
    tokio::spawn(async move {
        actors::creep_manager::run(creep_config, creep_notifier, creep_token).await;
    });

    // 4. Channels
    let (spawn_tx, spawn_rx) = tokio::sync::mpsc::channel(32);
    let (status_query_tx, status_query_rx) = tokio::sync::mpsc::channel(8);

    // 5. Start Heartbeat actor
    let heartbeat_client = client.clone();
    let heartbeat_interval = config.queen.heartbeat_interval;
    let heartbeat_token = token.clone();
    let heartbeat_hatchery_id = hatchery_id.clone();
    tokio::spawn(async move {
        actors::heartbeat::run(
            heartbeat_client,
            heartbeat_interval,
            heartbeat_hatchery_id,
            status_query_tx,
            heartbeat_token,
        )
        .await;
    });

    // 6. Start Poller actor
    let poller_client = client.clone();
    let poll_interval = config.queen.poll_interval;
    let poller_token = token.clone();
    let poller_hatchery_id = hatchery_id.clone();
    tokio::spawn(async move {
        actors::poller::run(
            poller_client,
            poll_interval,
            poller_hatchery_id,
            spawn_tx,
            poller_token,
        )
        .await;
    });

    // 7. Parse timeout duration
    let default_timeout = parse_duration(&config.queen.drone_timeout)?;
    let stall_threshold = Duration::from_secs(config.queen.stall_threshold);

    // 8. Start Supervisor actor
    let supervisor_client = client.clone();
    let supervisor_notifier = notifier.clone();
    let max_concurrency = config.queen.max_concurrency;
    let drone_dir = PathBuf::from(&config.queen.drone_dir);
    let supervisor_token = token.clone();
    let supervisor_handle = tokio::spawn(async move {
        actors::supervisor::run(
            supervisor_client,
            supervisor_notifier,
            max_concurrency,
            default_timeout,
            stall_threshold,
            drone_dir,
            spawn_rx,
            status_query_rx,
            supervisor_token,
        )
        .await;
    });

    // 9. Await Ctrl+C
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutdown signal received");

    // Cancel all actors
    token.cancel();

    // Wait for supervisor to finish (it will kill drones and exit)
    let _ = supervisor_handle.await;

    // Deregister from Overseer
    if let Some(id) = hatchery_id.read().await.as_ref() {
        if let Err(e) = client.deregister_hatchery(id).await {
            tracing::warn!(error = %e, "failed to deregister from overseer");
        }
    }

    notifier.notify(notifier::QueenEvent::ShuttingDown).await;

    Ok(())
}

fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    let s = s.trim();
    if let Some(hours) = s.strip_suffix('h') {
        Ok(Duration::from_secs(hours.parse::<u64>()? * 3600))
    } else if let Some(mins) = s.strip_suffix('m') {
        Ok(Duration::from_secs(mins.parse::<u64>()? * 60))
    } else if let Some(secs) = s.strip_suffix('s') {
        Ok(Duration::from_secs(secs.parse::<u64>()?))
    } else {
        Ok(Duration::from_secs(s.parse::<u64>()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
    }

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("300s").unwrap(), Duration::from_secs(300));
    }

    #[test]
    fn test_parse_duration_bare_number() {
        assert_eq!(parse_duration("60").unwrap(), Duration::from_secs(60));
    }
}
