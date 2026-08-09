# Agent-Search Architecture: DX & Agent-Friendly Optimization

## Current State

- Multi-engine search aggregator (SearXNG + 80+ engines from engines.yaml)
- BM25F coarse ranking + jina-reranker-v2 cross-encoder fine ranking
- MCP server at `/mcp` with `web_search` tool
- HTTP endpoints: `/search`, `/web_search`, `/search/ab`, `/search/stream`, `/content`
- Upstream super-relay fallback for search
- Engine suspension with exponential backoff

## Target: SOTA Agent-Friendly Service

### 1. Error Model (RFC 9457 + MCP isError)

**Current:** `SearchError` enum → `{"error": "..."}` string.

**Target:**
- HTTP: `application/problem+json` with `type`, `title`, `status`, `detail`, `instance`, `invalid_params[]`
- MCP tool: `isError: true` with `error_code`, `field`, `message`, `example`
- Error codes: `VALIDATION_ERROR`, `NOT_FOUND`, `TIMEOUT`, `UPSTREAM_ERROR`, `INTERNAL_ERROR`, `ENGINE_SUSPENDED`

### 2. MCP Tool Schema

**Current:** `web_search` takes `{query: string}`, no `additionalProperties: false`.

**Target:**
- `additionalProperties: false` on all objects
- `input_examples` for the query parameter
- Description passes Intern Test: what it does, when to use, params, output

### 3. Streaming (SSE)

**Current:** Raw JSON strings, no event types, no sentinel.

**Target:**
- Typed events: `event: result`, `event: done`, `event: error`
- Terminal sentinel: `event: done` (not connection close)
- Heartbeats: `: ping` every 15s
- In-band `error` events with `code` + `message`

### 4. HTTP Conventions

**Current:** Inconsistent error shapes, no idempotency.

**Target:**
- All errors use RFC 9457 format
- 422 for validation with `invalid_params`
- 400 for bad request, 504 for upstream timeout, 502 for upstream error

### 5. Response Design

**Current:** Full results with score, engines, published_date.

**Target:**
- High-signal by default (title, url, snippet, score)
- Bounded size (already truncates to max_results)
- `engines` field kept for transparency

### 6. Observability

**Current:** Basic tracing logs.

**Target:**
- `x-trace-id` header on every response (generated if not provided)
- Structured logging with trace_id, query, engine, duration

## File Changes

| File | Change |
|------|--------|
| `src/models/error.rs` | Add `ApiError` (RFC 9457), `ToolError`, error codes, `IntoResponse` impl |
| `src/mcp.rs` | Strict schema, `input_examples`, Intern Test description, `isError` structured errors |
| `src/routes/mod.rs` | RFC 9457 errors, typed SSE events, heartbeats, terminal sentinel |
| `src/main.rs` | Add trace-id middleware |
| `tests/e2e.rs` | E2E tests for all endpoints, error cases, MCP, streaming |

## Non-Goals

- Changing ranking strategy or engine config
- Removing the `/v1/chat/completions` and `/v1/messages` proxy endpoints (kept as-is)
