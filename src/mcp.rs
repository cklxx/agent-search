//! MCP (Model Context Protocol) server over Streamable HTTP.
//!
//! Exposes a `web_search` tool backed by the local search aggregator.
//! Supports both the modern Streamable HTTP transport (POST returns JSON)
//! and the legacy HTTP+SSE transport (GET /mcp/sse + POST /mcp/messages).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response, Sse};
use axum::Json;
use serde_json::{Value, json};
use tokio_stream::StreamExt;

use crate::models::error::{ErrorCode, ToolError};
use crate::models::query::SearchQuery;
use crate::routes::AppState;

const MCP_PROTOCOL_VERSION: &str = "2025-03-26";
const SERVER_NAME: &str = "agent-search";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Handle MCP JSON-RPC requests (Streamable HTTP POST transport).
pub async fn mcp_post(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Response {
    if let Some(arr) = body.as_array() {
        let mut responses = Vec::new();
        for msg in arr {
            if let Some(resp) = handle_message(&state, msg).await {
                responses.push(resp);
            }
        }
        return (StatusCode::OK, Json(json!(responses))).into_response();
    }

    match handle_message(&state, &body).await {
        Some(resp) => (StatusCode::OK, Json(resp)).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// Handle a single JSON-RPC message. Returns `None` for notifications.
async fn handle_message(state: &AppState, msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(|m| m.as_str())?;
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(json!({}));

    let result = match method {
        "initialize" => Some(handle_initialize(&params)),
        "tools/list" => Some(handle_tools_list()),
        "tools/call" => Some(handle_tools_call(state, &params).await),
        "ping" => Some(json!({})),
        "notifications/initialized" | "notifications/cancelled" | "notifications/progress" => None,
        _ => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("method not found: {}", method) }
        })),
    };

    match result {
        Some(mut resp) => {
            if let Some(id_val) = id {
                if resp.get("id").is_none() {
                    resp["id"] = id_val;
                }
            }
            resp["jsonrpc"] = json!("2.0");
            Some(resp)
        }
        None => None,
    }
}

fn handle_initialize(_params: &Value) -> Value {
    json!({
        "result": {
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {
                "tools": { "listChanged": false }
            },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": SERVER_VERSION
            }
        }
    })
}

fn handle_tools_list() -> Value {
    json!({
        "result": {
            "tools": [
                {
                    "name": "web_search",
                    "description": "Search the web for a query and return ranked results. Use this when you need current information from the internet, facts, references, or sources. Returns up to 10 results with title, URL, snippet, and relevance score. Do not use for queries that can be answered from training data.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "The search query string."
                            }
                        },
                        "required": ["query"],
                        "additionalProperties": false
                    },
                    "inputExamples": [
                        { "query": "rust async runtime comparison" }
                    ]
                }
            ]
        }
    })
}

async fn handle_tools_call(state: &AppState, params: &Value) -> Value {
    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    if name != "web_search" {
        return tool_error(ToolError::new(
            ErrorCode::NotFound,
            format!("unknown tool: {}", name),
        ));
    }

    let query_str = match args.get("query").and_then(|q| q.as_str()) {
        Some(q) if !q.is_empty() => q.to_string(),
        _ => {
            return tool_error(
                ToolError::new(ErrorCode::ValidationError, "missing required argument: query")
                    .with_field("query", "rust async runtime"),
            );
        }
    };

    let query = SearchQuery {
        query: query_str,
        ..Default::default()
    };

    let results = crate::routes::search_with_fallback(state, &query).await;

    let results_json: Vec<Value> = results
        .iter()
        .map(|r| {
            json!({
                "title": r.title,
                "url": r.url,
                "snippet": r.snippet,
                "score": r.score,
            })
        })
        .collect();

    json!({
        "result": {
            "resultType": "complete",
            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&results_json).unwrap_or_default() }],
            "structuredContent": { "results": results_json }
        }
    })
}

/// Build an MCP tool error response with `isError: true`.
fn tool_error(err: ToolError) -> Value {
    let text = serde_json::to_string(&err).unwrap_or_else(|_| err.message.clone());
    json!({
        "result": {
            "resultType": "complete",
            "isError": true,
            "content": [{ "type": "text", "text": text }],
            "structuredContent": err
        }
    })
}

/// Legacy HTTP+SSE transport: GET /mcp/sse opens an SSE stream.
pub async fn mcp_sse() -> Sse<impl tokio_stream::Stream<Item = Result<axum::response::sse::Event, axum::Error>>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(16);
    let _ = tx.send("endpoint: /mcp/messages".to_string()).await;

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|data| {
        Ok(axum::response::sse::Event::default().data(data))
    });

    Sse::new(stream)
}

/// Legacy HTTP+SSE transport: POST /mcp/messages handles JSON-RPC.
pub async fn mcp_messages(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Response {
    match handle_message(&state, &body).await {
        Some(resp) => (StatusCode::OK, Json(resp)).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}
