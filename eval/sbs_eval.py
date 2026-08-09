#!/usr/bin/env python3
"""Side-by-side (SBS) evaluation: compare our search results against a baseline.

Baseline can be:
- Google (via SearXNG JSON API)
- Our own multi-engine aggregation (for strategy A/B comparison)

Metrics:
- overlap@k: fraction of our top-k results that appear in baseline top-k
- ndcg@k: ranking quality using baseline position as relevance
- reciprocal_rank: how far down the first baseline result appears in ours
"""

import argparse
import json
import sys
import time
from typing import Any

import requests


def search_ours(base_url: str, query: str, max_results: int = 10) -> list[dict]:
    """Query our search API."""
    resp = requests.post(
        f"{base_url}/search",
        json={"query": query, "max_results": max_results},
        timeout=30,
    )
    resp.raise_for_status()
    return resp.json().get("results", [])


def search_baseline_searxng(searxng_url: str, query: str, max_results: int = 10, engine: str = "bing") -> list[dict]:
    """Query SearXNG (aggregates Google, Bing, DuckDuckGo, etc.)."""
    params = {"q": query, "format": "json", "pageno": 1}
    if engine:
        params["engines"] = engine
    resp = requests.get(f"{searxng_url}/search", params=params, timeout=30)
    resp.raise_for_status()
    data = resp.json()
    results = data.get("results", [])
    return [{"title": r.get("title", ""), "url": r.get("url", "")} for r in results[:max_results]]


def normalize_url(url: str) -> str:
    """Normalize URL for comparison: lowercase host, strip trailing slash, fragment."""
    url = url.strip().lower()
    if "#" in url:
        url = url[: url.index("#")]
    url = url.rstrip("/")
    return url


def overlap_at_k(ours: list[dict], baseline: list[dict], k: int) -> float:
    """Fraction of our top-k URLs that appear in baseline top-k."""
    if not ours or not baseline:
        return 0.0
    ours_urls = {normalize_url(r["url"]) for r in ours[:k]}
    baseline_urls = {normalize_url(r["url"]) for r in baseline[:k]}
    if not ours_urls:
        return 0.0
    return len(ours_urls & baseline_urls) / len(ours_urls)


def ndcg_at_k(ours: list[dict], baseline: list[dict], k: int) -> float:
    """NDCG@k using baseline position as relevance (1 / (pos+1))."""
    baseline_relevance: dict[str, float] = {}
    for i, r in enumerate(baseline[:k]):
        baseline_relevance[normalize_url(r["url"])] = 1.0 / (i + 1)

    dcg = 0.0
    for i, r in enumerate(ours[:k]):
        rel = baseline_relevance.get(normalize_url(r["url"]), 0.0)
        dcg += rel / (i + 1)  # log2(i+2) not needed since relevance already decays

    # Ideal DCG: baseline results in order.
    idcg = 0.0
    for i in range(min(k, len(baseline))):
        rel = 1.0 / (i + 1)
        idcg += rel / (i + 1)

    if idcg == 0:
        return 0.0
    return dcg / idcg


def reciprocal_rank(ours: list[dict], baseline: list[dict]) -> float:
    """Reciprocal rank of the first baseline URL in our results."""
    baseline_urls = [normalize_url(r["url"]) for r in baseline]
    for i, r in enumerate(ours):
        if normalize_url(r["url"]) in baseline_urls:
            return 1.0 / (i + 1)
    return 0.0


def evaluate_query(
    query: str,
    ours: list[dict],
    baseline: list[dict],
    k: int = 10,
) -> dict[str, Any]:
    return {
        "query": query,
        "overlap@10": overlap_at_k(ours, baseline, k),
        "ndcg@10": ndcg_at_k(ours, baseline, k),
        "mrr": reciprocal_rank(ours, baseline),
        "ours_count": len(ours),
        "baseline_count": len(baseline),
    }


def main():
    parser = argparse.ArgumentParser(description="SBS search evaluation")
    parser.add_argument("--base-url", default="http://127.0.0.1:18789")
    parser.add_argument("--searxng-url", default="http://127.0.0.1:39217")
    parser.add_argument("--queries", nargs="+", default=None,
                        help="Queries to evaluate. If omitted, reads from stdin (one per line).")
    parser.add_argument("--k", type=int, default=10)
    parser.add_argument("--baseline-engine", default="bing",
                        help="SearXNG engine to use as baseline (e.g., bing, google, duckduckgo)")
    parser.add_argument("--baseline", choices=["searxng", "ours_bm25"], default="searxng",
                        help="Baseline source. 'ours_bm25' compares bge_reranker against bm25.")
    args = parser.parse_args()

    if args.queries:
        queries = args.queries
    else:
        queries = [line.strip() for line in sys.stdin if line.strip()]

    if not queries:
        print("No queries provided.", file=sys.stderr)
        sys.exit(1)

    results = []
    for q in queries:
        print(f"[query] {q}", file=sys.stderr)
        try:
            ours = search_ours(args.base_url, q, args.k)
            if args.baseline == "searxng":
                baseline = search_baseline_searxng(args.searxng_url, q, args.k, args.baseline_engine)
            else:
                # Compare bge_reranker (default) against bm25 via /search/ab.
                resp = requests.post(
                    f"{args.base_url}/search/ab",
                    json={"query": q, "strategy_a": "bm25", "strategy_b": "bge_reranker", "max_results": args.k},
                    timeout=60,
                )
                resp.raise_for_status()
                data = resp.json()
                baseline = data.get("results_a", [])
                ours = data.get("results_b", [])

            metrics = evaluate_query(q, ours, baseline, args.k)
            results.append(metrics)
            print(f"  overlap@10={metrics['overlap@10']:.2f}  ndcg@10={metrics['ndcg@10']:.3f}  mrr={metrics['mrr']:.3f}", file=sys.stderr)
        except Exception as e:
            print(f"  error: {e}", file=sys.stderr)
            results.append({"query": q, "error": str(e)})
        time.sleep(0.5)  # be polite to upstream

    # Aggregate.
    valid = [r for r in results if "error" not in r]
    if not valid:
        print(json.dumps(results, indent=2, ensure_ascii=False))
        return

    avg_overlap = sum(r["overlap@10"] for r in valid) / len(valid)
    avg_ndcg = sum(r["ndcg@10"] for r in valid) / len(valid)
    avg_mrr = sum(r["mrr"] for r in valid) / len(valid)

    print("\n" + "=" * 60)
    print(f"{'METRIC':<20} {'VALUE':>10}")
    print("-" * 60)
    print(f"{'overlap@10':<20} {avg_overlap:>10.3f}")
    print(f"{'ndcg@10':<20} {avg_ndcg:>10.3f}")
    print(f"{'mrr':<20} {avg_mrr:>10.3f}")
    print(f"{'queries':<20} {len(valid):>10}")
    print("=" * 60)

    print(json.dumps({"average": {"overlap@10": avg_overlap, "ndcg@10": avg_ndcg, "mrr": avg_mrr}, "per_query": results}, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
