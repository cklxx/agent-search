# AB Evaluation Harness

Evaluation set and metrics for comparing search ranking strategies, focused on
**technical** and **academic** queries.

## Files

| File | Purpose |
|------|---------|
| `queries.yaml` | 30 queries across 6 categories with expected keywords/domains |
| `run_eval.py` | Runs queries against the API and computes relevance metrics |

## Query Categories

- **programming** (5): rust, python, go, typescript
- **algorithms** (5): BST, DP, graph, hash, sorting
- **systems** (5): docker, k8s, TCP, CAP, git
- **ai** (5): transformer, BERT, diffusion, RL, embeddings
- **science** (5): quantum, CRISPR, Bayesian, P=NP, relativity
- **tools** (5): react, postgres, systemd, nginx, CI/CD

## Metrics

| Metric | Description |
|--------|-------------|
| **Precision@K** | Fraction of top-K results that are relevant |
| **Recall** | Fraction of expected keywords/domains covered |
| **MRR** | Mean Reciprocal Rank — position of first relevant result |
| **NDCG@K** | Normalized Discounted Cumulative Gain (graded relevance) |
| **Engine Coverage** | Number of distinct engines returning results |
| **Latency** | End-to-end response time |

Relevance is graded 0–1 based on:
- Domain match with `expected_domains` (+0.5)
- Keyword overlap with `expected_keywords` (+0.5 × matched/total)

## Usage

```bash
# Run against local server
python eval/run_eval.py --base-url http://127.0.0.1:8080 --top-k 10

# Save results for comparison
python eval/run_eval.py --output eval/results_baseline.json
```

## AB Testing Workflow

1. **Run baseline**: capture current metrics
   ```bash
   python eval/run_eval.py --output eval/baseline.json
   ```

2. **Change strategy** (e.g., adjust engine weights, BM25 params, scoring formula)

3. **Run variant**: capture new metrics
   ```bash
   python eval/run_eval.py --output eval/variant_a.json
   ```

4. **Compare**: diff the aggregate metrics
   ```bash
   python -c "
   import json
   b = json.load(open('eval/baseline.json'))['aggregate']
   v = json.load(open('eval/variant_a.json'))['aggregate']
   for k in b:
       diff = v[k] - b[k]
       print(f'{k:25s} baseline={b[k]:.4f} variant={v[k]:.4f} delta={diff:+.4f}')
   "
   ```

## Current Baseline (30 queries, top-10)

| Category | P@10 | MRR | NDCG@10 |
|----------|------|-----|---------|
| tools | 0.73 | 1.00 | 0.98 |
| systems | 0.62 | 1.00 | 0.94 |
| programming | 0.58 | 0.60 | 0.84 |
| science | 0.27 | 0.70 | 0.68 |
| algorithms | 0.23 | 0.40 | 0.58 |
| ai | 0.23 | 0.27 | 0.44 |
| **overall** | **0.44** | **0.66** | **0.74** |

### Optimization Targets

1. **AI & algorithms** — lowest precision; need better academic engines (arxiv, semantic scholar, google scholar)
2. **Engine coverage** — avg 1.5 engines/query; many engines suspended (403/429); proxy pool needed
3. **Recall** — 0.46; increase max_results or add more engines per category
