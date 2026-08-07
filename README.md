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
    └── mod.rs           # HTTP routes (/search, /search/stream, /engines, /health)
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

## Configuration

Edit `config.toml`:

```toml
host = "127.0.0.1"
port = 8080
request_timeout = 10
cache_size = 1000
cache_ttl_secs = 30
searxng_url = "http://127.0.0.1:39217"
```

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
