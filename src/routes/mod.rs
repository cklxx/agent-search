//! HTTP routes for the search API.

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
use crate::models::query::SearchQuery;

/// Application state shared across requests.
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<EngineRegistry>,
    pub cache: QueryCache,
    pub suspension: Arc<EngineSuspensionManager>,
}

/// POST /search
pub async fn search(
    State(state): State<AppState>,
    Json(query): Json<SearchQuery>,
) -> impl IntoResponse {
    let key = cache_key(&query.query, query.page, query.max_results);

    // Check cache
    if let Some(cached) = state.cache.get(&key).await {
        return (StatusCode::OK, Json((*cached).clone())).into_response();
    }

    match aggregator::aggregate(&query, &state.registry, &state.suspension).await {
        Ok(response) => {
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

/// GET /engines
pub async fn list_engines(State(state): State<AppState>) -> impl IntoResponse {
    let names = state.registry.names();
    (StatusCode::OK, Json(serde_json::json!({"engines": names}))).into_response()
}

/// GET /health
pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
}

/// POST /search/stream — streaming search results via SSE.
pub async fn search_stream(
    State(state): State<AppState>,
    Json(query): Json<SearchQuery>,
) -> Sse<impl tokio_stream::Stream<Item = Result<axum::response::sse::Event, axum::Error>>> {
    let (tx, _rx) = broadcast::channel(100);
    let tx_clone = tx.clone();
    let registry = state.registry.clone();
    let suspension = state.suspension.clone();

    tokio::spawn(async move {
        let response = aggregator::aggregate(&query, &registry, &suspension).await;
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
