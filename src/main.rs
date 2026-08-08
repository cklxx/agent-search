use std::sync::Arc;
use std::time::Duration;

use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use agent_search::cache::QueryCache;
use agent_search::config::Config;
use agent_search::engine::engines::builtin_registry;
use agent_search::engine::EngineSuspensionManager;
use agent_search::index::LocalIndex;
use agent_search::ranking::get_strategy;
use agent_search::routes::{AppState, fetch_content, health, list_engines, list_strategies, search, search_ab, search_stream};

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

    let proxy_manager = Arc::new(config.proxy_manager());
    let registry = builtin_registry(&config.searxng_url, Some(proxy_manager.clone()));
    tracing::info!("registered engines: {:?}", registry.names());

    let cache = QueryCache::new(config.cache_size, Duration::from_secs(config.cache_ttl_secs));
    let suspension = Arc::new(EngineSuspensionManager::default());

    // Local index: try persistent disk index, fall back to in-memory
    let local_index = Arc::new(
        LocalIndex::open_or_create(std::path::Path::new("data/index"))
            .unwrap_or_else(|_| LocalIndex::new_in_ram().expect("failed to create in-memory index")),
    );

    // Default ranking strategy from config
    let strategy = get_strategy(&config.strategy);
    tracing::info!("ranking strategy: {}", strategy.name());

    let state = AppState {
        registry: Arc::new(registry),
        cache,
        suspension,
        local_index,
        strategy: Arc::from(strategy),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/engines", get(list_engines))
        .route("/strategies", get(list_strategies))
        .route("/search", post(search))
        .route("/search/ab", post(search_ab))
        .route("/search/stream", post(search_stream))
        .route("/content", get(fetch_content))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
