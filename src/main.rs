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
use agent_search::mcp::{mcp_messages, mcp_post, mcp_sse};
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
    let registry = builtin_registry(Some(proxy_manager.clone()));
    tracing::info!("registered engines: {:?}", registry.names());

    let cache = QueryCache::new(config.cache_size, Duration::from_secs(config.cache_ttl_secs));
    let suspension = Arc::new(EngineSuspensionManager::default());

    // Disk index, fall back to in-memory.
    let local_index = Arc::new(
        LocalIndex::open_or_create(std::path::Path::new("data/index"))
            .unwrap_or_else(|_| LocalIndex::new_in_ram().expect("failed to create in-memory index")),
    );

    let strategy = get_strategy(&config.strategy).unwrap_or_else(|| {
        tracing::warn!("unknown strategy '{}', falling back to bm25", config.strategy);
        Box::new(agent_search::ranking::Bm25Strategy)
    });
    tracing::info!("ranking strategy: {}", strategy.name());

    let request_timeout = Duration::from_secs(config.request_timeout_secs);

    let state = AppState {
        registry: Arc::new(registry),
        cache,
        suspension,
        local_index,
        strategy: Arc::from(strategy),
        request_timeout,
        upstream_search_url: config.upstream_search_url.clone(),
        upstream_api_key: config.upstream_api_key.clone(),
    };

    // Warmup: preheat the ranking model and populate caches for common queries.
    warmup(&state, &config.warmup_queries).await;

    let app = Router::new()
        .route("/health", get(health))
        .route("/engines", get(list_engines))
        .route("/strategies", get(list_strategies))
        .route("/search", post(search))
        .route("/search/ab", post(search_ab))
        .route("/search/stream", post(search_stream))
        .route("/content", get(fetch_content));

    // MCP server (Streamable HTTP + legacy SSE transport).
    let app = if config.mcp_enabled {
        let mcp_path = config.mcp_path.trim_end_matches('/').to_string();
        let sse_path = format!("{}/sse", mcp_path);
        let messages_path = format!("{}/messages", mcp_path);
        app.route(&mcp_path, axum::routing::post(mcp_post))
            .route(&sse_path, get(mcp_sse))
            .route(&messages_path, post(mcp_messages))
    } else {
        app
    };

    let app = app
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

/// Preheat the ranking model and populate caches for common queries.
async fn warmup(state: &AppState, queries: &[String]) {
    tracing::info!("warming up ranking model...");
    // Run a dummy query through the strategy to load/initialize the model.
    let dummy = agent_search::models::query::SearchQuery {
        query: "warmup".to_string(),
        ..Default::default()
    };
    let dummy_raw = agent_search::models::result::RawSearchResult {
        title: "warmup".to_string(),
        url: "https://example.com".to_string(),
        snippet: "warmup".to_string(),
        published_date: None,
        position: 1,
    };
    let _ = state.strategy.score(&dummy_raw, &dummy, 1.0, &[]);

    // Preheat common queries into the cache.
    for q in queries {
        let query = agent_search::models::query::SearchQuery {
            query: q.clone(),
            ..Default::default()
        };
        match agent_search::aggregator::aggregate(
            &query,
            &state.registry,
            &state.suspension,
            state.strategy.as_ref(),
        )
        .await
        {
            Ok(response) => {
                let _ = state.local_index.cache_results(q, &response.results);
                let key = agent_search::cache::cache_key(q, 0, 10);
                state.cache.insert(key, Arc::new(response)).await;
                tracing::info!("warmed up query: {}", q);
            }
            Err(e) => tracing::warn!("warmup failed for '{}': {}", q, e),
        }
    }
    tracing::info!("warmup complete");
}
