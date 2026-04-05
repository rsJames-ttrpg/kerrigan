mod config;
mod index;
pub mod lsp;
mod lsp_service;
mod parser;
mod symbol_index;
mod proto {
    tonic::include_proto!("creep.v1");
}
mod service;
mod watcher;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use tonic::transport::Server;

use crate::config::Config;
use crate::index::FileIndex;
use crate::lsp::manager::{LspManager, LspServerConfig};
use crate::lsp_service::LspServiceImpl;
use crate::proto::file_index_server::FileIndexServer;
use crate::proto::lsp_service_server::LspServiceServer;
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

    let port = std::env::var("CREEP_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(config.creep.grpc_port);
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;
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
        if config.creep.symbol_index {
            let si = symbol_index.clone();
            let ws_clone = ws.clone();
            match tokio::task::spawn_blocking(move || si.scan_workspace(&ws_clone)).await {
                Ok(Ok(n)) => tracing::info!("parsed {n} symbols in {}", ws.display()),
                Ok(Err(e)) => tracing::warn!("symbol scan failed for {}: {e}", ws.display()),
                Err(e) => tracing::warn!("symbol scan task panicked for {}: {e}", ws.display()),
            }
        }
    }

    // Build LSP server configs from creep config.
    let lsp_configs: Vec<LspServerConfig> = config
        .creep
        .lsp
        .iter()
        .map(|(name, lsp_cfg)| LspServerConfig {
            name: name.clone(),
            command: lsp_cfg.command.clone(),
            args: lsp_cfg.args.clone(),
            extensions: lsp_cfg.extensions.clone(),
            language_id: lsp_cfg.language_id.clone(),
        })
        .collect();
    if !lsp_configs.is_empty() {
        tracing::info!("configured {} LSP server(s)", lsp_configs.len());
    }
    let lsp_manager = Arc::new(Mutex::new(LspManager::new(lsp_configs)));

    // Spawn event processor task.
    tokio::spawn(process_events(
        index.clone(),
        symbol_index.clone(),
        watcher.clone(),
        lsp_manager.clone(),
        event_rx,
    ));

    // Set up health reporter.
    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<FileIndexServer<FileIndexServiceImpl>>()
        .await;

    // Create service implementations.
    let file_index_svc = FileIndexServiceImpl::new(index, symbol_index, watcher);
    let lsp_svc = LspServiceImpl::new(lsp_manager);

    // Start tonic gRPC server with graceful shutdown on Ctrl+C.
    tracing::info!("gRPC server listening on {addr}");
    Server::builder()
        .add_service(health_service)
        .add_service(FileIndexServer::new(file_index_svc))
        .add_service(LspServiceServer::new(lsp_svc))
        .serve_with_shutdown(addr, async {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to listen for ctrl-c");
            tracing::info!("shutting down");
        })
        .await?;

    Ok(())
}
