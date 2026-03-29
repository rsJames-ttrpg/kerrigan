mod api;
mod config;
mod db;
mod embedding;
mod error;
mod mcp;
mod services;

use config::Config;
use embedding::stub::StubEmbedding;
use mcp::CortexMcp;
use services::AppState;
use std::path::PathBuf;
use std::sync::Arc;
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

    let db_path = config.storage.database_path.to_string_lossy();
    let pool = db::open(&db_path).await?;
    tracing::info!("database opened at {:?}", db_path);

    let embedder: Arc<dyn embedding::EmbeddingProvider> = Arc::new(StubEmbedding::new(384));
    tracing::info!("embedding provider: {}", embedder.model_name());

    let state = Arc::new(AppState::new(
        pool,
        embedder,
        config.storage.artifact_path.clone(),
    ));

    // HTTP server
    let http_router = api::router(state.clone());
    let http_addr = format!("0.0.0.0:{}", config.server.http_port);
    let listener = tokio::net::TcpListener::bind(&http_addr).await?;
    tracing::info!("HTTP server listening on {}", http_addr);

    match config.server.mcp_transport.as_str() {
        "stdio" => {
            // Run HTTP in background, MCP on stdio in foreground
            tokio::spawn(async move {
                axum::serve(listener, http_router).await.unwrap();
            });

            tracing::info!("MCP server starting on stdio");
            let mcp_server = CortexMcp::new(state);
            let service = rmcp::ServiceExt::serve(mcp_server, rmcp::transport::stdio()).await?;
            service.waiting().await?;
        }
        _ => {
            // HTTP only mode
            tracing::info!("running in HTTP-only mode");
            axum::serve(listener, http_router)
                .with_graceful_shutdown(async {
                    tokio::signal::ctrl_c().await.unwrap();
                    tracing::info!("shutting down");
                })
                .await?;
        }
    }

    Ok(())
}
