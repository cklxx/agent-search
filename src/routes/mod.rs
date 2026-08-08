//! HTTP routes.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Sse};
use axum::Json;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::aggregator;
use crate::cache::{QueryCache, cache_key};
use crate::engine::{EngineRegistry, EngineSuspensionManager};
use crate::index::LocalIndex;
use crate::models::query::SearchQuery;
use crate::ranking::{RankingStrategy, get_strategy, strategy_names};

#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<EngineRegistry>,
    pub cache: QueryCache,
    pub suspension: Arc<EngineSuspensionManager>,
    pub local_index: Arc<LocalIndex>,
    pub strategy: Arc<dyn RankingStrategy>,
}

/// POST /search
pub async fn search(
    State(state): State<AppState>,
    Json(query): Json<SearchQuery>,
) -> impl IntoResponse {
    let key = cache_key(&query.query, query.page, query.max_results);

    if let Some(cached) = state.cache.get(&key).await {
        return (StatusCode::OK, Json((*cached).clone())).into_response();
    }

    if let Some(local_results) = state.local_index.search_cached(&query.query) {
        let response = crate::models::result::SearchResponse {
            query: query.query.clone(),
            results: local_results,
            errors: Vec::new(),
        };
        let response = Arc::new(response);
        state.cache.insert(key, response.clone()).await;
        return (StatusCode::OK, Json((*response).clone())).into_response();
    }

    match aggregator::aggregate(&query, &state.registry, &state.suspension, state.strategy.as_ref()).await {
        Ok(response) => {
            let _ = state.local_index.cache_results(&query.query, &response.results);
            let response = Arc::new(response);
            state.cache.insert(key, response.clone()).await;
            (StatusCode::OK, Json((*response).clone())).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /search/ab — run two strategies side-by-side.
///
/// Body: `{"query": "...", "strategy_a": "bm25", "strategy_b": "tfidf"}`
pub async fn search_ab(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let query_str = match body.get("query").and_then(|v| v.as_str()) {
        Some(q) => q.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "query is required"})),
            )
                .into_response();
        }
    };

    let strategy_a_name = body.get("strategy_a").and_then(|v| v.as_str()).unwrap_or("bm25");
    let strategy_b_name = body.get("strategy_b").and_then(|v| v.as_str()).unwrap_or("tfidf");
    let max_results = body.get("max_results").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

    let query = SearchQuery {
        query: query_str,
        max_results,
        ..Default::default()
    };

    let strategy_a = get_strategy(strategy_a_name);
    let strategy_b = get_strategy(strategy_b_name);

    // Run aggregator twice (once per strategy). Simpler than sharing raw results.
    let resp_a = aggregator::aggregate(&query, &state.registry, &state.suspension, strategy_a.as_ref()).await;
    let resp_b = aggregator::aggregate(&query, &state.registry, &state.suspension, strategy_b.as_ref()).await;

    match (resp_a, resp_b) {
        (Ok(a), Ok(b)) => {
            let urls_a: std::collections::HashSet<&str> = a.results.iter().map(|r| r.url.as_str()).collect();
            let urls_b: std::collections::HashSet<&str> = b.results.iter().map(|r| r.url.as_str()).collect();
            let overlap = urls_a.intersection(&urls_b).count();

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "query": query.query,
                    "strategy_a": strategy_a_name,
                    "strategy_b": strategy_b_name,
                    "results_a": a.results,
                    "results_b": b.results,
                    "overlap": overlap,
                    "overlap_ratio": overlap as f64 / urls_a.len().max(1) as f64,
                })),
            )
                .into_response()
        }
        (Err(e), _) | (_, Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /strategies
pub async fn list_strategies() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"strategies": strategy_names()}))).into_response()
}

/// GET /engines
pub async fn list_engines(State(state): State<AppState>) -> impl IntoResponse {
    let names = state.registry.names();
    (StatusCode::OK, Json(serde_json::json!({"engines": names}))).into_response()
}

/// GET /health
pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
}

/// POST /search/stream — SSE stream of results.
pub async fn search_stream(
    State(state): State<AppState>,
    Json(query): Json<SearchQuery>,
) -> Sse<impl tokio_stream::Stream<Item = Result<axum::response::sse::Event, axum::Error>>> {
    let (tx, _rx) = broadcast::channel(100);
    let tx_clone = tx.clone();
    let registry = state.registry.clone();
    let suspension = state.suspension.clone();
    let strategy = state.strategy.clone();

    tokio::spawn(async move {
        let response = aggregator::aggregate(&query, &registry, &suspension, strategy.as_ref()).await;
        match response {
            Ok(resp) => {
                for result in resp.results {
                    let _ = tx_clone.send(serde_json::to_string(&result).unwrap_or_default());
                }
            }
            Err(e) => {
                let _ = tx_clone.send(format!(r#"{{"error":"{}"}}"#, e));
            }
        }
    });

    let rx = tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|r| r.ok()).map(|msg| {
        Ok(axum::response::sse::Event::default().data(msg))
    });

    Sse::new(stream)
}

/// GET /content?url=... — fetch and extract page text.
pub async fn fetch_content(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let url = match params.get("url") {
        Some(u) => u.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "url parameter is required"})),
            )
                .into_response();
        }
    };

    match crate::fetcher::fetch_content(&url).await {
        Ok(content) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "url": content.url,
                "title": content.title,
                "content": content.content,
                "fetched_at": content.fetched_at.to_rfc3339(),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
