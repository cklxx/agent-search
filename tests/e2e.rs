//! End-to-end tests for the HTTP API and MCP server.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use tower::ServiceExt;

use agent_search::cache::QueryCache;
use agent_search::engine::engines::builtin_registry;
use agent_search::engine::EngineSuspensionManager;
use agent_search::index::LocalIndex;
use agent_search::ranking::get_strategy;
use agent_search::routes::AppState;

fn test_state() -> AppState {
    let registry = builtin_registry(None, None);
    AppState {
        registry: Arc::new(registry),
        cache: QueryCache::new(100, Duration::from_secs(30)),
        suspension: Arc::new(EngineSuspensionManager::default()),
        local_index: Arc::new(LocalIndex::new_in_ram().unwrap()),
        strategy: get_strategy("bm25").unwrap(),
        request_timeout: Duration::from_secs(10),
        upstream_search_url: None,
        upstream_api_key: None,
        http_client: reqwest::Client::new(),
    }
}

fn app() -> Router {
    let state = test_state();
    axum::Router::new()
        .route("/health", axum::routing::get(agent_search::routes::health))
        .route("/engines", axum::routing::get(agent_search::routes::list_engines))
        .route("/strategies", axum::routing::get(agent_search::routes::list_strategies))
        .route("/search", axum::routing::post(agent_search::routes::search))
        .route("/search/ab", axum::routing::post(agent_search::routes::search_ab))
        .route("/search/stream", axum::routing::post(agent_search::routes::search_stream))
        .route("/web_search", axum::routing::post(agent_search::routes::web_search))
        .route("/content", axum::routing::get(agent_search::routes::fetch_content))
        .route("/mcp", axum::routing::post(agent_search::mcp::mcp_post))
        .with_state(state)
}

async fn post_json(router: Router, path: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn get(router: Router, path: &str) -> (StatusCode, serde_json::Value) {
    let resp = router
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();

    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

// --- Health & metadata ---

#[tokio::test]
async fn health_returns_ok() {
    let (status, body) = get(app(), "/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn engines_lists_registered_engines() {
    let (status, body) = get(app(), "/engines").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["engines"].is_array());
    assert!(!body["engines"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn strategies_lists_available_strategies() {
    let (status, body) = get(app(), "/strategies").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["strategies"].is_array());
}

// --- /web_search ---

#[tokio::test]
async fn web_search_requires_query() {
    let (status, body) = post_json(app(), "/web_search", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["type"], "https://agent-search.dev/errors/validation_error");
    assert_eq!(body["title"], "VALIDATION_ERROR");
    assert_eq!(body["status"], 422);
    assert!(body["invalid_params"].is_array());
}

#[tokio::test]
async fn web_search_rejects_empty_query() {
    let (status, _) = post_json(app(), "/web_search", serde_json::json!({"query": ""})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

// --- /search/ab ---

#[tokio::test]
async fn search_ab_requires_query() {
    let (status, body) = post_json(app(), "/search/ab", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["title"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn search_ab_rejects_unknown_strategy() {
    let (status, body) = post_json(
        app(),
        "/search/ab",
        serde_json::json!({"query": "rust", "strategy_a": "nonexistent"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["detail"].as_str().unwrap().contains("nonexistent"));
}

// --- /content ---

#[tokio::test]
async fn content_requires_url() {
    let (status, body) = get(app(), "/content").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["title"], "VALIDATION_ERROR");
}

// --- MCP ---

#[tokio::test]
async fn mcp_initialize_returns_server_info() {
    let (status, body) = post_json(
        app(),
        "/mcp",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["serverInfo"]["name"], "agent-search");
    assert_eq!(body["result"]["protocolVersion"], "2025-03-26");
}

#[tokio::test]
async fn mcp_tools_list_has_web_search_with_strict_schema() {
    let (status, body) = post_json(
        app(),
        "/mcp",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let tools = body["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);

    let tool = &tools[0];
    assert_eq!(tool["name"], "web_search");
    assert_eq!(tool["inputSchema"]["additionalProperties"], false);
    assert!(tool["inputSchema"]["required"].as_array().unwrap().contains(&"query".into()));
    assert!(tool["inputExamples"].is_array());
}

#[tokio::test]
async fn mcp_web_search_returns_is_error_on_missing_query() {
    let (status, body) = post_json(
        app(),
        "/mcp",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "web_search", "arguments": {} }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true);
    assert_eq!(body["result"]["structuredContent"]["error_code"], "VALIDATION_ERROR");
    assert_eq!(body["result"]["structuredContent"]["field"], "query");
}

#[tokio::test]
async fn mcp_unknown_tool_returns_is_error() {
    let (status, body) = post_json(
        app(),
        "/mcp",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "nonexistent", "arguments": {} }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true);
    assert_eq!(body["result"]["structuredContent"]["error_code"], "NOT_FOUND");
}

// --- Error response format (RFC 9457) ---

#[tokio::test]
async fn errors_use_rfc_9457_format() {
    let (status, body) = post_json(app(), "/web_search", serde_json::json!({})).await;

    // RFC 9457 required fields
    assert!(body["type"].is_string());
    assert!(body["title"].is_string());
    assert!(body["status"].is_number());
    assert!(body["detail"].is_string());
    assert_eq!(body["status"], status.as_u16());
}
