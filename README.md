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

## MCP Server (recommended)

Built-in MCP server at `/mcp` with a `web_search` tool. Point any MCP client
(Claude Code, Claude Desktop, etc.) at it to get web search backed by the
local multi-engine pipeline.

### Claude Code config

Add to `~/.claude.json` or project `.claude/settings.json`:

```json
{
  "mcpServers": {
    "agent-search": {
      "url": "http://127.0.0.1:18789/mcp"
    }
  }
}
```

The `web_search` tool takes `{ "query": "..." }` and returns results with
title, URL, snippet, and score.

### Direct HTTP

`POST /web_search` with `{ "query": "..." }` returns the same results as a
plain JSON array — useful for scripts or non-MCP clients.

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
