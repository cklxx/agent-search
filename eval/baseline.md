# Ranking Baseline

Best results as of commit `5cdcb2a`.

## Architecture

- **Retrieval**: 47 engines (Brave, Bing, rawweb, StackExchange family, GitHub, arXiv, ...) + local Tantivy full-text index
- **Coarse rank**: BM25 + position weight, top-50 (v2) / top-10 (v3)
- **Fine rank**: jina-reranker-v2 (pointwise cross-encoder, INT8) or jina-reranker-v3 (listwise, INT8)
- **Blend**: BM25 floor + cross-encoder relevance, weighted by domain authority, freshness, engine weight
- **Exact-match queries**: BM25 dominates (0.80/0.20 blend), authority/weight neutralized

## Models

| Model | Size (INT8) | Type | Notes |
|-------|-------------|------|-------|
| jina-reranker-v2-base-multilingual | 266 MB | pointwise cross-encoder | default strategy |
| jina-reranker-v3 | 569 MB | listwise (Qwen3-based) | concurrent session pool |

INT8 quantization via `scripts/quantize.py` (dynamic, QUInt8). fp32 fallback when INT8 model absent.

## Metrics (73 queries, bm25 vs bge_reranker)

| Metric | bm25 | bge_reranker | Delta |
|--------|------|-------------|-------|
| Precision@10 | 0.527 | 0.586 | +0.059 |
| MRR | 0.798 | 0.829 | +0.031 |
| NDCG@10 | 0.845 | 0.860 | +0.015 |
| mean_relevance | 0.394 | 0.423 | +0.028 |
| engine_coverage | 4.44 | 4.00 | -0.44 |

## SBS (vs rawweb/Bing, 20 queries)

- overlap@10: 0.10 — low overlap expected; Brave finds canonical docs (docs.rs, official tutorials) that rawweb misses
- mrr: 0.42 — when results overlap, our top result is often Bing's top result
- The eval harness (keyword/domain relevance) is the primary quality gate; SBS overlap is a diagnostic, not a target

## Notes

- Brave engine rate-limits (429); suspension manager backs off automatically. For stable throughput, use Brave Search API (free tier).
- `request_timeout_secs = 30` to accommodate slow engines.
- Engine weights: Brave/Bing/rawweb = 2.0, most others 1.0-1.5.

## Deployment (digest)

- Code synced via rsync (GitHub unreachable from digest)
- INT8 models rsync'd: `models/jina-reranker-v2-int8/` (266MB), `models/jina-reranker-v3/model.int8.onnx*` (569MB)
- Proxy: `proxy_urls = ["http://sys-proxy-rd-relay.byted.org:8118"]` in config.toml
- `libonnxruntime.so` installed at `/usr/local/lib/`
- Server: `nohup /tmp/start-search.sh > /tmp/agent-search.log 2>&1 &`
- Network: Brave/Wikipedia blocked; rawweb/Bing/HN/GitHub work through proxy. Engine timeouts cause upstream results to be dropped; local Tantivy index serves as fallback.
