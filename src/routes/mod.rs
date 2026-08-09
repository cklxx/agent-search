//! HTTP routes.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Sse};
use axum::Json;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
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
    pub request_timeout: std::time::Duration,
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

    let result = tokio::time::timeout(
        state.request_timeout,
        aggregator::aggregate(&query, &state.registry, &state.suspension, state.strategy.as_ref()),
    )
    .await;

    match result {
        Ok(Ok(response)) => {
            let _ = state.local_index.cache_results(&query.query, &response.results);
            let response = Arc::new(response);
            state.cache.insert(key, response.clone()).await;
            (StatusCode::OK, Json((*response).clone())).into_response()
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(serde_json::json!({"error": "search timed out"})),
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

    let strategy_a = match get_strategy(strategy_a_name) {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("unknown strategy_a: {}", strategy_a_name)})),
            )
                .into_response();
        }
    };
    let strategy_b = match get_strategy(strategy_b_name) {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("unknown strategy_b: {}", strategy_b_name)})),
            )
                .into_response();
        }
    };

    // Fetch raw results once, score twice with each strategy.
    let (dedup_map, errors) =
        aggregator::fetch_raw_results(&query, &state.registry, &state.suspension).await;

    let results_a = aggregator::score_results(dedup_map.clone(), &query, strategy_a.as_ref());
    let results_b = aggregator::score_results(dedup_map, &query, strategy_b.as_ref());

    let urls_a: std::collections::HashSet<&str> = results_a.iter().map(|r| r.url.as_str()).collect();
    let urls_b: std::collections::HashSet<&str> = results_b.iter().map(|r| r.url.as_str()).collect();
    let overlap = urls_a.intersection(&urls_b).count();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "query": query.query,
            "strategy_a": strategy_a_name,
            "strategy_b": strategy_b_name,
            "results_a": results_a,
            "results_b": results_b,
            "overlap": overlap,
            "overlap_ratio": overlap as f64 / urls_a.len().max(1) as f64,
            "errors": errors,
        })),
    )
        .into_response()
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
    let (tx, rx) = mpsc::channel(100);
    let registry = state.registry.clone();
    let suspension = state.suspension.clone();
    let strategy = state.strategy.clone();

    tokio::spawn(async move {
        let response = aggregator::aggregate(&query, &registry, &suspension, strategy.as_ref()).await;
        match response {
            Ok(resp) => {
                for result in resp.results {
                    let _ = tx.send(serde_json::to_string(&result).unwrap_or_default()).await;
                }
            }
            Err(e) => {
                let _ = tx.send(format!(r#"{{"error":"{}"}}"#, e)).await;
            }
        }
    });

    let stream = ReceiverStream::new(rx).map(|msg| {
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
