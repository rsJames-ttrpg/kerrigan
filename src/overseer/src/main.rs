#![allow(dead_code)]

mod api;
mod config;
mod db;
mod embedding;
mod error;
mod mcp;
mod services;

use config::Config;
use embedding::stub::StubEmbedding;
use mcp::OverseerMcp;
use services::AppState;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("overseer.toml"));

    let config = Config::load(&config_path)?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.logging.level)),
        )
        .init();

    tracing::info!("overseer starting");

    let db_path = config.storage.database_path.to_string_lossy();
    let pool = db::open(&db_path).await?;
    tracing::info!("database opened at {:?}", db_path);

    config.embedding.validate()?;

    let mut providers: std::collections::HashMap<String, Arc<dyn embedding::EmbeddingProvider>> =
        std::collections::HashMap::new();

    for (name, provider_config) in &config.embedding.providers {
        let provider: Arc<dyn embedding::EmbeddingProvider> = match provider_config.source.as_str()
        {
            "stub" => Arc::new(StubEmbedding::new(provider_config.dimensions)),
            "voyage" => {
                let model = provider_config
                    .model
                    .as_deref()
                    .expect("voyage provider requires 'model' in config");
                let api_key_env = provider_config
                    .api_key_env
                    .as_deref()
                    .expect("voyage provider requires 'api_key_env' in config");
                let api_key = std::env::var(api_key_env)
                    .unwrap_or_else(|_| panic!("env var '{api_key_env}' not set"));
                Arc::new(embedding::voyage::VoyageEmbedding::new(
                    model.to_string(),
                    api_key,
                    provider_config.dimensions,
                ))
            }
            other => anyhow::bail!("unknown embedding source: {other}"),
        };
        tracing::info!(
            "embedding provider '{name}': {} ({}d)",
            provider.model_name(),
            provider.dimensions()
        );
        db::create_embedding_table(&pool, name, provider_config.dimensions).await?;
        providers.insert(name.clone(), provider);
    }

    let registry = embedding::EmbeddingRegistry::new(providers, config.embedding.default.clone())?;
    tracing::info!("default embedding provider: {}", config.embedding.default);

    let state = Arc::new(AppState::new(
        pool,
        registry,
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
            let mcp_server = OverseerMcp::new(state);
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
