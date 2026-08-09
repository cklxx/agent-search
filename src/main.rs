use std::sync::Arc;
use std::time::Duration;

use axum::extract::Request;
use axum::http::HeaderName;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use agent_search::TRACE_ID;

use agent_search::cache::QueryCache;
use agent_search::config::Config;
use agent_search::engine::engines::builtin_registry;
use agent_search::engine::EngineSuspensionManager;
use agent_search::index::LocalIndex;
use agent_search::mcp::{mcp_messages, mcp_post, mcp_sse};
use agent_search::ranking::get_strategy;
use agent_search::routes::{AppState, chat_completions, crawl_url, fetch_content, health, list_engines, list_strategies, messages, search, search_ab, search_stream, web_search};

fn main() -> anyhow::Result<()> {
    // Cross-encoder reranking runs on spawn_blocking (CPU-heavy, must not
    // block the async runtime). Size the blocking pool so concurrent
    // requests don't queue behind a small default. worker_threads defaults
    // to num_cpus; max_blocking_threads raised above the 512 default to
    // absorb bursts of rerank + A/B scoring tasks.
    let worker_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .max_blocking_threads(256)
        .enable_all()
        .build()?;
    runtime.block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::load(std::path::Path::new("config.toml"));
    tracing::info!("starting agent-search on {}:{}", config.host, config.port);

    let proxy_manager = Arc::new(config.proxy_manager());
    let registry = builtin_registry(Some(proxy_manager.clone()), config.stackexchange_api_key.as_deref());
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
        Arc::new(agent_search::ranking::Bm25Strategy)
    });
    tracing::info!("ranking strategy: {}", strategy.name());

    let request_timeout = Duration::from_secs(config.request_timeout_secs);

    let http_client = reqwest::Client::builder()
        .timeout(request_timeout)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .unwrap_or_default();

    let state = AppState {
        registry: Arc::new(registry),
        cache,
        suspension,
        local_index,
        strategy,
        request_timeout,
        upstream_search_url: config.upstream_search_url.clone(),
        upstream_api_key: config.upstream_api_key.clone(),
        http_client,
        engine_semaphore: Arc::new(tokio::sync::Semaphore::new(agent_search::aggregator::MAX_CONCURRENT_ENGINES)),
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
        .route("/web_search", post(web_search))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/messages", post(messages))
        .route("/content", get(fetch_content))
        .route("/crawl", post(crawl_url));

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
        .layer(middleware::from_fn(trace_id_middleware))
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
            state.strategy.clone(),
            &state.engine_semaphore,
        )
        .await
        {
            Ok(response) => {
                let _ = state.local_index.cache_results(q, &response.results);
                let key = agent_search::cache::cache_key(&query);
                state.cache.insert(key, Arc::new(response)).await;
                tracing::info!("warmed up query: {}", q);
            }
            Err(e) => tracing::warn!("warmup failed for '{}': {}", q, e),
        }
    }
    tracing::info!("warmup complete");
}

/// Generate or propagate a trace ID for every request.
///
/// Reads `x-trace-id` from the request (if present), otherwise generates a
/// new UUID v4. The trace ID is set on the response and available via
/// `tracing::Span::current().record("trace_id", ...)`.
async fn trace_id_middleware(mut req: Request, next: Next) -> Response {
    let trace_id = req
        .headers()
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    req.extensions_mut().insert(trace_id.clone());

    // Record trace_id on the current span so it appears in all log lines.
    tracing::Span::current().record("trace_id", tracing::field::display(&trace_id));

    // Set the task-local trace ID so downstream code (engine requests, etc.)
    // can forward it in x-trace-id headers.
    TRACE_ID
        .scope(trace_id.clone(), async move {
            let mut resp = next.run(req).await;
            resp.headers_mut().insert(
                HeaderName::from_static("x-trace-id"),
                trace_id.parse().unwrap(),
            );
            resp
        })
        .await
}
