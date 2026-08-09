# Agent Search

Search engine for AI agents. Multi-engine aggregation + cross-encoder reranking.

## Quick Start

```bash
cargo build --release
./target/release/agent-search
```

Search:

```bash
curl -X POST http://127.0.0.1:18789/search \
  -H "Content-Type: application/json" \
  -d '{"query":"rust async runtime","max_results":10}'
```

## Architecture

- **46 engines** across general, IT, science, packages, news, etc.
- **Concurrency limit**: 16 simultaneous engine requests (semaphore) to prevent
  timeouts under load. Per-engine timeout scales with weight (6–10s).
- **Dedup**: normalized URL (lowercase host, strip tracking params, trailing
  slash, fragment). Weight = max across engines returning the same URL.
- **Suspension**: exponential backoff per engine per error type (403/429 → 180s).
- **Cache**: two layers — moka `QueryCache` (exact `query:page:max_results`)
  and Tantivy `LocalIndex` (exact query-string match via STRING field).

## Ranking Pipeline

1. **Coarse**: BM25F (title=3.0, url=1.5, snippet=1.0) → top-50
2. **Fine**: jina-reranker-v2-base-multilingual cross-encoder
3. **Mix**: `authority² × relevance × coverage` → sigmoid → [0, 1]

## API

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/health` | Liveness |
| GET | `/engines` | Registered engine names |
| GET | `/strategies` | Ranking strategy names |
| POST | `/search` | Search (`SearchQuery` body) |
| POST | `/search/ab` | A/B two strategies |
| POST | `/search/stream` | SSE stream |
| GET | `/content?url=` | Extract page main text |
| POST | `/web_search` | Simple JSON array (scripts) |

`SearchQuery` fields: `query`, `max_results` (default 10), `page`, `language`,
`time_range` (`day`/`week`/`month`/`year`), `safe_search` (0/1/2), `category`.

## MCP Server

Built-in MCP server at `/mcp` with a `web_search` tool.

```json
{
  "mcpServers": {
    "agent-search": {
      "url": "http://127.0.0.1:18789/mcp"
    }
  }
}
```

## Eval (bge_reranker vs bm25, 73 queries)

| Metric | bm25 | bge_reranker | Delta |
|--------|------|-------------|-------|
| MRR | 0.82 | **1.00** | +0.18 |
| NDCG@10 | 0.87 | **0.91** | +0.04 |
