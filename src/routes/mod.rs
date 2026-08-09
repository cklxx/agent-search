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
    /// Upstream search API base URL. When set, /search uses this upstream.
    pub upstream_search_url: Option<String>,
    /// API key for the upstream search API.
    pub upstream_api_key: Option<String>,
    /// Shared HTTP client for upstream search (avoids per-call connection setup).
    pub http_client: reqwest::Client,
}

/// Try upstream search first; fall back to the local aggregator on empty
/// results. Shared by `/search`, `/web_search`, and the MCP `web_search` tool.
pub async fn search_with_fallback(
    state: &AppState,
    query: &SearchQuery,
) -> Vec<crate::models::result::SearchResult> {
    if let Some(ref upstream) = state.upstream_search_url {
        let upstream_results = tokio::time::timeout(
            state.request_timeout,
            search_upstream(&state.http_client, upstream, state.upstream_api_key.as_deref(), &query.query),
        )
        .await
        .unwrap_or_default();

        if !upstream_results.is_empty() {
            return upstream_results;
        }
        tracing::warn!("upstream search returned no results, falling back to local aggregator");
    }

    match tokio::time::timeout(
        state.request_timeout,
        aggregator::aggregate(query, &state.registry, &state.suspension, state.strategy.clone()),
    )
    .await
    {
        Ok(Ok(response)) => response.results,
        _ => Vec::new(),
    }
}

/// POST /search
pub async fn search(
    State(state): State<AppState>,
    Json(query): Json<SearchQuery>,
) -> impl IntoResponse {
    let key = cache_key(&query);

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

    let results = search_with_fallback(&state, &query).await;
    let response = crate::models::result::SearchResponse {
        query: query.query.clone(),
        results,
        errors: Vec::new(),
    };
    let _ = state.local_index.cache_results(&query.query, &response.results);
    let response = Arc::new(response);
    state.cache.insert(key, response.clone()).await;
    (StatusCode::OK, Json((*response).clone())).into_response()
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
    let fetch_result = tokio::time::timeout(
        state.request_timeout,
        aggregator::fetch_raw_results(&query, &state.registry, &state.suspension, aggregator::ENGINE_FANOUT_DEADLINE),
    )
    .await;

    let (dedup_map, errors) = match fetch_result {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::GATEWAY_TIMEOUT,
                Json(serde_json::json!({"error": "search timed out"})),
            )
                .into_response();
        }
    };

    // Score both strategies in parallel on blocking threads.
    let query_clone = query.clone();
    let dedup_map_clone = dedup_map.clone();
    let a_handle = tokio::task::spawn_blocking(move || {
        aggregator::score_results(dedup_map_clone, &query_clone, strategy_a.as_ref())
    });
    let query_clone = query.clone();
    let b_handle = tokio::task::spawn_blocking(move || {
        aggregator::score_results(dedup_map, &query_clone, strategy_b.as_ref())
    });

    let (results_a, results_b) = match tokio::join!(a_handle, b_handle) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "scoring task panicked"})),
            )
                .into_response();
        }
    };

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

    let request_timeout = state.request_timeout;
    tokio::spawn(async move {
        let response = tokio::time::timeout(
            request_timeout,
            aggregator::aggregate(&query, &registry, &suspension, strategy.clone()),
        )
        .await;
        match response {
            Ok(Ok(resp)) => {
                for result in resp.results {
                    let _ = tx.send(serde_json::to_string(&result).unwrap_or_default()).await;
                }
            }
            Ok(Err(e)) => {
                let _ = tx
                    .send(serde_json::json!({"error": e.to_string()}).to_string())
                    .await;
            }
            Err(_) => {
                let _ = tx
                    .send(serde_json::json!({"error": "search timed out"}).to_string())
                    .await;
            }
        }
    });

    let stream = ReceiverStream::new(rx).map(|msg| {
        Ok(axum::response::sse::Event::default().data(msg))
    });

    Sse::new(stream)
}

/// POST /web_search — simple search endpoint that can replace a relay's
/// web_search tool backend. Takes `{"query": "..."}` and returns results.
pub async fn web_search(
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

    let query = SearchQuery {
        query: query_str.clone(),
        ..Default::default()
    };
    let results = search_with_fallback(&state, &query).await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "query": query_str,
            "results": results,
        })),
    )
        .into_response()
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

/// Call the upstream LLM relay's web_search tool and return parsed results.
/// Returns an empty vec if the upstream fails or returns no results; the
/// caller is responsible for falling back to the local aggregator.
pub async fn search_upstream(
    client: &reqwest::Client,
    upstream: &str,
    api_key: Option<&str>,
    query: &str,
) -> Vec<crate::models::result::SearchResult> {
    let url = format!("{}/v1/chat/completions", upstream.trim_end_matches('/'));

    let prompt = format!(
        "Search the web for: {}. List the top results as a numbered list. \
         For each result, provide the title, the URL, and a one-sentence snippet. \
         Format: \\n1. **Title** — https://example.com\\n   - Snippet text here.",
        query
    );

    let body = serde_json::json!({
        "model": "model_api/experimental_0723",
        "messages": [{"role": "user", "content": prompt}],
        "tools": [{"type": "web_search", "search_context_size": "high"}],
        "max_tokens": 4096,
    });

    let mut req = client.post(&url).json(&body);
    if let Some(key) = api_key {
        req = req.header("Authorization", format!("Bearer {}", key));
    }

    let content = match req.send().await {
        Ok(resp) => match resp.text().await {
            Ok(t) => serde_json::from_str::<serde_json::Value>(&t)
                .ok()
                .and_then(|v| {
                    v.pointer("/choices/0/message/content")
                        .and_then(|c| c.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_default(),
            Err(_) => String::new(),
        },
        Err(_) => String::new(),
    };

    parse_search_results(&content)
}

/// Parse search results from the model's text output.
/// Supports both JSON array and numbered list formats.
pub fn parse_search_results(content: &str) -> Vec<crate::models::result::SearchResult> {
    // Try JSON array first.
    if let Some(results) = parse_json_results(content) {
        if !results.is_empty() {
            return results;
        }
    }
    // Fall back to numbered list format.
    parse_numbered_list(content)
}

fn parse_json_results(content: &str) -> Option<Vec<crate::models::result::SearchResult>> {
    let start = content.find('[')?;
    let end = content.rfind(']')?;
    if end <= start {
        return None;
    }
    let json_str = &content[start..=end];
    let items: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;

    let results: Vec<_> = items
        .into_iter()
        .filter_map(|item| {
            let title = item.get("title").and_then(|v| v.as_str())?.to_string();
            let url = item.get("url").and_then(|v| v.as_str())?.to_string();
            let snippet = item
                .get("snippet")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(crate::models::result::SearchResult {
                title,
                url,
                snippet,
                published_date: None,
                score: 0.0,
                engines: vec!["upstream".to_string()],
            })
        })
        .collect();

    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}

/// Parse numbered list format:
/// 1. **Title** — URL
///    - Snippet
fn parse_numbered_list(content: &str) -> Vec<crate::models::result::SearchResult> {
    let re = regex::Regex::new(r"^\d+\.\s+\*\*(.+?)\*\*\s*[—-]\s*(https?://\S+)").unwrap();
    let mut results = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(caps) = re.captures(line) {
            let title = caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            let url = caps.get(2).map(|m| m.as_str().to_string()).unwrap_or_default();
            let mut snippet = String::new();
            // Next line might be the snippet: "   - Snippet"
            if i + 1 < lines.len() {
                let next = lines[i + 1].trim_start();
                if let Some(s) = next.strip_prefix("- ") {
                    snippet = s.to_string();
                    i += 1;
                }
            }
            if !title.is_empty() && !url.is_empty() {
                results.push(crate::models::result::SearchResult {
                    title,
                    url,
                    snippet,
                    published_date: None,
                    score: 0.0,
                    engines: vec!["upstream".to_string()],
                });
            }
        }
        i += 1;
    }
    results
}

/// POST /v1/chat/completions — OpenAI-compatible endpoint.
///
/// Requests with the `web_search` tool are served locally by the search
/// pipeline. All other requests are transparently proxied to the upstream
/// LLM relay (super-relay). This lets cc point its API URL at agent-search:
/// web_search works locally, LLM inference still goes through the relay.
pub async fn chat_completions(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let has_web_search = body
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|tools| {
            tools.iter().any(|t| {
                t.get("type").and_then(|ty| ty.as_str()) == Some("web_search")
            })
        })
        .unwrap_or(false);

    // Non-search requests: proxy to the upstream LLM relay.
    if !has_web_search {
        if let Some(ref upstream) = state.upstream_search_url {
            let url = format!("{}/v1/chat/completions", upstream.trim_end_matches('/'));
            let mut req = state.http_client.post(&url).json(&body);
            if let Some(key) = state.upstream_api_key.as_deref() {
                req = req.header("Authorization", format!("Bearer {}", key));
            }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    return (status, text).into_response();
                }
                Err(e) => {
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({"error": format!("upstream proxy failed: {}", e)})),
                    )
                        .into_response();
                }
            }
        }
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "no upstream LLM configured"})),
        )
            .into_response();
    }

    // web_search tool: serve locally.
    let query = body
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|msgs| msgs.last())
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();

    if query.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "no user message"})),
        )
            .into_response();
    }

    let search_query = SearchQuery {
        query: query.clone(),
        ..Default::default()
    };
    let results = search_with_fallback(&state, &search_query).await;

    let content = if results.is_empty() {
        "No search results found.".to_string()
    } else {
        results
            .iter()
            .enumerate()
            .map(|(i, r)| {
                format!(
                    "{}. **{}** — {}\n   {}",
                    i + 1,
                    r.title,
                    r.url,
                    if r.snippet.is_empty() { "" } else { &r.snippet }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let response = serde_json::json!({
        "id": format!("chatcmpl-{}", rand::random::<u64>()),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": body.get("model").and_then(|m| m.as_str()).unwrap_or("agent-search"),
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0
        }
    });

    (StatusCode::OK, Json(response)).into_response()
}

/// POST /v1/messages — Anthropic-compatible endpoint.
///
/// Requests with the `web_search` tool: run local search, inject results
/// into the user message, then forward to the upstream LLM relay without
/// the web_search tool. All other requests are proxied transparently.
pub async fn messages(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let has_web_search = body
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|tools| {
            tools.iter().any(|t| {
                t.get("type").and_then(|ty| ty.as_str()) == Some("web_search")
            })
        })
        .unwrap_or(false);

    let upstream = match state.upstream_search_url.as_ref() {
        Some(u) => u,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "no upstream LLM configured"})),
            )
                .into_response();
        }
    };

    let mut body = body;

    if has_web_search {
        // Extract query from the last user message.
        let query = body
            .get("messages")
            .and_then(|m| m.as_array())
            .and_then(|msgs| msgs.last())
            .and_then(|m| m.get("content"))
            .and_then(|c| match c {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Array(arr) => arr
                    .iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .next()
                    .map(|s| s.to_string()),
                _ => None,
            })
            .unwrap_or_default();

        if !query.is_empty() {
            let search_query = SearchQuery {
                query: query.clone(),
                ..Default::default()
            };
            let results = search_with_fallback(&state, &search_query).await;

            // Format search results as context.
            let search_context = if results.is_empty() {
                String::new()
            } else {
                let formatted: Vec<String> = results
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        format!(
                            "[{}] {} — {}\n{}",
                            i + 1,
                            r.title,
                            r.url,
                            if r.snippet.is_empty() { "" } else { &r.snippet }
                        )
                    })
                    .collect();
                format!(
                    "\n\n<search_results>\n{}\n</search_results>",
                    formatted.join("\n\n")
                )
            };

            // Inject search results into the last user message.
            if let Some(msgs) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
                if let Some(last) = msgs.last_mut() {
                    if let Some(content) = last.get_mut("content") {
                        match content {
                            serde_json::Value::String(s) => {
                                s.push_str(&search_context);
                            }
                            serde_json::Value::Array(arr) => {
                                arr.push(serde_json::json!({
                                    "type": "text",
                                    "text": search_context
                                }));
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Remove web_search tool before forwarding.
            if let Some(tools) = body.get_mut("tools").and_then(|t| t.as_array_mut()) {
                tools.retain(|t| t.get("type").and_then(|ty| ty.as_str()) != Some("web_search"));
            }
        }
    }

    // Forward to upstream LLM relay.
    let url = format!("{}/v1/messages", upstream.trim_end_matches('/'));
    let mut req = state.http_client.post(&url).json(&body);
    if let Some(key) = state.upstream_api_key.as_deref() {
        req = req.header("Authorization", format!("Bearer {}", key));
        req = req.header("anthropic-version", "2023-06-01");
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            (status, text).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": format!("upstream proxy failed: {}", e)})),
        )
            .into_response(),
    }
}
