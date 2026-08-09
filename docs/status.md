# Optimization Status

## Phase 1: Error Model ✅
- [x] RFC 9457 `ApiError` struct (`type`, `title`, `status`, `detail`, `instance`, `invalid_params`)
- [x] `ErrorCode` enum with HTTP status mapping
- [x] `IntoResponse` for `ApiError` (sets `application/problem+json`)
- [x] `From<SearchError> for ApiError`
- [x] MCP `ToolError` (`error_code`, `field`, `message`, `example`) for `isError: true`

## Phase 2: MCP Schema ✅
- [x] `additionalProperties: false` on `web_search` input schema
- [x] `inputExamples` with sample query
- [x] Intern Test description (what, when, params, output)
- [x] Structured `isError` tool errors via `ToolError`

## Phase 3: HTTP Routes ✅
- [x] RFC 9457 error responses on all endpoints (`/search/ab`, `/web_search`, `/content`, `/v1/*`)
- [x] 422 for validation errors with `invalid_params`
- [x] Typed SSE events: `event: result`, `event: done`, `event: error`
- [x] Terminal sentinel (`done` event)
- [x] Heartbeats via `Sse::keep_alive` (15s ping)

## Phase 4: Observability ✅
- [x] `x-trace-id` response header (generated if not in request)
- [x] Trace ID stored in request extensions

## Phase 5: E2E Tests ✅
- [x] `/health`, `/engines`, `/strategies`
- [x] `/web_search` validation (missing query, empty query)
- [x] `/search/ab` validation (missing query, unknown strategy)
- [x] `/content` validation (missing url)
- [x] MCP `initialize`, `tools/list` (strict schema check)
- [x] MCP `tools/call` error cases (missing query, unknown tool)
- [x] RFC 9457 error format verification

## Verification
- [x] `cargo check` — passes
- [x] `cargo test` — 14 unit + 13 e2e = 27 tests pass
- [x] `cargo clippy -- -D warnings` — clean
