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

## SBS: vs SearXNG-only (30 queries, top-10)

| Metric | agent-search | searxng_only | Delta |
|--------|-------------|-------------|-------|
| Precision@10 | **0.84** | 0.24 | +0.60 |
| MRR | **0.975** | 0.406 | +0.57 |
| NDCG@10 | **0.956** | 0.602 | +0.35 |

## Ranking Pipeline

1. **Coarse**: BM25F (title=3.0, url=1.5, snippet=1.0) → top-30
2. **Fine**: jina-reranker-v2-base-multilingual cross-encoder
3. **Mix**: `authority² × relevance × coverage`

## MCP

Built-in MCP server at `/mcp` with `web_search` tool.
