mod config;
mod index;
mod parser;
mod symbol_index;
mod proto {
    tonic::include_proto!("creep.v1");
}
mod service;
mod watcher;

use std::net::SocketAddr;
use std::path::PathBuf;

use tonic::transport::Server;

use crate::config::Config;
use crate::index::FileIndex;
use crate::proto::file_index_server::FileIndexServer;
use crate::service::FileIndexServiceImpl;
use crate::watcher::{FileWatcher, process_events};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Init tracing.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Load config.
    let config_path = PathBuf::from("creep.toml");
    let config: Config = if config_path.exists() {
        Config::load(&config_path)?
    } else {
        tracing::info!("no creep.toml found, using defaults");
        toml::from_str("")?
    };

    let addr: SocketAddr = format!("0.0.0.0:{}", config.creep.grpc_port).parse()?;
    tracing::info!("creep starting on {addr}");

    // Create FileIndex and SymbolIndex.
    let index = FileIndex::new();
    let symbol_index = symbol_index::SymbolIndex::new();

    // Create FileWatcher.
    let (watcher, event_rx) = FileWatcher::new(index.clone());

    // Scan configured workspaces + start watching.
    for ws_str in &config.creep.workspaces {
        let ws = PathBuf::from(ws_str);
        tracing::info!("registering workspace {}", ws.display());
        {
            let mut guard = watcher.lock().await;
            if let Err(e) = guard.watch(&ws) {
                tracing::warn!("failed to watch {}: {e}", ws.display());
            }
        }
        match index.scan_workspace(&ws).await {
            Ok(n) => tracing::info!("indexed {n} files in {}", ws.display()),
            Err(e) => tracing::warn!("scan failed for {}: {e}", ws.display()),
        }
        {
            let si = symbol_index.clone();
            let ws_clone = ws.clone();
            match tokio::task::spawn_blocking(move || si.scan_workspace(&ws_clone)).await {
                Ok(Ok(n)) => tracing::info!("parsed {n} symbols in {}", ws.display()),
                Ok(Err(e)) => tracing::warn!("symbol scan failed for {}: {e}", ws.display()),
                Err(e) => tracing::warn!("symbol scan task panicked for {}: {e}", ws.display()),
            }
        }
    }

    // Spawn event processor task.
    tokio::spawn(process_events(
        index.clone(),
        symbol_index.clone(),
        watcher.clone(),
        event_rx,
    ));

    // Set up health reporter.
    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<FileIndexServer<FileIndexServiceImpl>>()
        .await;

    // Create FileIndexServiceImpl.
    let file_index_svc = FileIndexServiceImpl::new(index, symbol_index, watcher);

    // Start tonic gRPC server with graceful shutdown on Ctrl+C.
    tracing::info!("gRPC server listening on {addr}");
    Server::builder()
        .add_service(health_service)
        .add_service(FileIndexServer::new(file_index_svc))
        .serve_with_shutdown(addr, async {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to listen for ctrl-c");
            tracing::info!("shutting down");
        })
        .await?;

    Ok(())
}
