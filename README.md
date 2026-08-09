# Agent Search

High-performance search engine for AI agents. A Rust-based metasearch engine that aggregates results from multiple search engines and returns structured JSON optimized for LLM consumption.

## Features

- **Agent-first API**: Pure JSON API, no HTML UI. Structured results with `title`, `url`, `snippet`, `score`, `engine`.
- **Multi-engine aggregation**: Fans out queries to multiple engines concurrently, deduplicates, scores, and ranks.
- **SearXNG upstream**: Uses SearXNG as the primary upstream for 200+ engine coverage. Native Rust engines (DuckDuckGo, Bing, Brave) as fallbacks.
- **Relevance scoring**: TF-based scoring with position and engine weight.
- **URL deduplication**: Normalizes URLs (removes tracking params, lowercases host, strips trailing slashes).
- **Query caching**: In-memory cache with TTL (moka).
- **Streaming**: SSE endpoint for progressive result delivery.
- **Reverse proxy**: Optionally forward `/search` to an upstream search API (e.g. a SearXNG instance). No API key required — just set the upstream URL.
- **Built-in MCP server**: Exposes a `web_search` tool over the Model Context Protocol, so AI agents (Claude Code, etc.) can search the web directly.
- **High performance**: async (tokio), parallel sorting (rayon-ready), connection pooling.

## Architecture

```
src/
├── main.rs              # Entry point, HTTP server setup
├── lib.rs               # Library root
├── config.rs            # Configuration (TOML)
├── aggregator.rs        # Core: fan-out, dedup, score, sort
├── ranking.rs           # Relevance scoring
├── dedup.rs             # URL normalization
├── cache.rs             # Query cache (moka)
├── mcp.rs               # MCP server (web_search tool)
├── models/              # Data types
│   ├── query.rs         # SearchQuery
│   ├── result.rs        # SearchResult, RawSearchResult, SearchResponse
│   └── error.rs         # SearchError, EngineResult
├── engine/
│   ├── trait_def.rs     # SearchEngine trait
│   ├── mod.rs           # EngineRegistry
│   └── engines/
│       ├── searxng.rs   # SearXNG upstream engine
│       ├── duckduckgo.rs
│       ├── bing.rs
│       └── brave.rs
└── routes/
    └── mod.rs           # HTTP routes (/search, /search/stream, /mcp, /engines, /health)
```

## API

### POST /search

```json
{
  "query": "rust programming",
  "max_results": 10,
  "search_depth": "basic",
  "include_answer": false,
  "include_raw_content": false,
  "time_range": null,
  "domains": [],
  "language": null,
  "page": 0,
  "safe_search": 0
}
```

Response:

```json
{
  "query": "rust programming",
  "results": [
    {
      "title": "Rust Programming Language",
      "url": "https://rust-lang.org/",
      "snippet": "Rust is blazingly fast...",
      "score": 1.0,
      "engine": "searxng",
      "engines": ["searxng"]
    }
  ],
  "errors": [],
  "answer": null
}
```

### POST /search/stream

SSE stream — each result is sent as a `data:` event.

### GET /engines

Returns list of registered engine names.

### GET /health

Health check.

### MCP server

The service exposes a Model Context Protocol (MCP) server at the path configured by `mcp_path` (default `/mcp`). It implements the **Streamable HTTP** transport (POST returns JSON-RPC responses) and the legacy **HTTP+SSE** transport (`GET /mcp/sse` + `POST /mcp/messages`).

The server exposes one tool:

| Tool | Input | Description |
|------|-------|-------------|
| `web_search` | `{ "query": string }` | Search the web. Returns results with `title`, `url`, `snippet`, `score`. |

Example flow:

```bash
# 1. initialize
curl -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -H "Accept: application/json, text/event-stream" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}'

# 2. list tools
curl -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'

# 3. call web_search
curl -X POST http://127.0.0.1:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"web_search","arguments":{"query":"rust async"}}}'
```

### Reverse proxy

When `upstream_search_url` is set in `config.toml`, `POST /search` forwards the request body to `<upstream_search_url>/search` and returns the upstream response verbatim. When unset, the built-in multi-engine aggregator is used.

```toml
upstream_search_url = "https://your-searxng.example.com"
```

## Configuration

Edit `config.toml`:

```toml
host = "127.0.0.1"
port = 8080
cache_size = 1000
cache_ttl_secs = 30
strategy = "bge_reranker"
request_timeout_secs = 15

# Built-in MCP server.
mcp_enabled = true
mcp_path = "/mcp"
```

### Upstream search API (optional)

To use an upstream LLM relay with a `web_search` tool (e.g. super-relay)
instead of the local multi-engine aggregator, set environment variables:

```bash
export UPSTREAM_SEARCH_URL="https://super-relay.byted.org"
export UPSTREAM_API_KEY="your-auth-token"
```

When `UPSTREAM_SEARCH_URL` is set, both `POST /search` and the MCP `web_search`
tool forward queries to the upstream and parse the returned JSON results.

### MCP server

The MCP endpoint is `http://{host}:{port}{mcp_path}` (default `http://127.0.0.1:8080/mcp`).

To use it with Claude Code, add to your MCP config:

```json
{
  "mcpServers": {
    "agent-search": {
      "url": "http://127.0.0.1:8080/mcp"
    }
  }
}
```

The server exposes a `web_search` tool: `{ "query": string }` → results with
`title`, `url`, `snippet`, `score`.

## Run

```bash
cargo run
# or
cargo build --release && ./target/release/agent-search
```

## Test

```bash
curl -X POST http://127.0.0.1:8080/search \
  -H "Content-Type: application/json" \
  -d '{"query":"rust","max_results":5}'
```

## Roadmap

- [ ] Full-text content fetching (spider)
- [ ] BM25 scoring via Tantivy
- [ ] Local index for cached queries
- [ ] Engine-level rate limiting
- [ ] Proxy pool rotation
- [ ] More native Rust engines
- [ ] Answer generation (LLM summary)
