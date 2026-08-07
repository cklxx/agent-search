use std::sync::Arc;
use std::time::Duration;

use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use agent_search::cache::QueryCache;
use agent_search::config::Config;
use agent_search::engine::engines::builtin_registry;
use agent_search::routes::{AppState, health, list_engines, search, search_stream};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::load(std::path::Path::new("config.toml"));
    tracing::info!("starting agent-search on {}:{}", config.host, config.port);

    let registry = builtin_registry(&config.searxng_url);
    tracing::info!("registered engines: {:?}", registry.names());

    let cache = QueryCache::new(config.cache_size, Duration::from_secs(config.cache_ttl_secs));

    let state = AppState {
        registry: Arc::new(registry),
        cache,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/engines", get(list_engines))
        .route("/search", post(search))
        .route("/search/stream", post(search_stream))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
