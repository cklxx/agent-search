# Ranking Baseline

Best results as of commit `02b18d9`.

## Model

- **Reranker**: jina-reranker-v2-base-multilingual (~300MB)
- **Coarse rank**: BM25F (title=3.0, url=1.5, snippet=1.0), top-30
- **Fine rank**: cross-encoder (query-document relevance)
- **Mix rank**: `authority² × relevance × coverage`

## Metrics (vs searxng_only baseline, 30 queries)

| Metric | bge_reranker | searxng_only | Delta |
|--------|-------------|-------------|-------|
| Precision@10 | 0.84 | 0.24 | +0.60 |
| MRR | 0.975 | 0.406 | +0.57 |
| NDCG@10 | 0.956 | 0.602 | +0.35 |
| mean_relevance | 0.57 | 0.24 | +0.33 |

## Notes

- Model: jina-reranker-v2-base-multilingual (300MB, multilingual)
- Documents truncated to 512 chars
- Reranker rebuilt every call to reclaim onnx runtime memory
- archive.org snapshots unwrapped to original URLs and downranked (authority 0.5)
