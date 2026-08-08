#!/usr/bin/env python3
"""AB evaluation harness for agent-search.

Runs a set of technical/academic queries against the search API and
computes relevance metrics based on expected keywords and domains.

Usage:
    python eval/run_eval.py [--base-url http://127.0.0.1:8080] [--top-k 10]
"""

import argparse
import json
import time
from pathlib import Path
from typing import Any

import requests
import yaml


def load_queries(path: str) -> list[dict[str, Any]]:
    with open(path) as f:
        data = yaml.safe_load(f)
    return data["queries"]


def search(base_url: str, query: str, max_results: int) -> dict[str, Any]:
    """Run a search and return the parsed response + latency."""
    url = f"{base_url}/search"
    payload = {"query": query, "max_results": max_results}
    start = time.time()
    resp = requests.post(url, json=payload, timeout=30)
    latency = time.time() - start
    resp.raise_for_status()
    data = resp.json()
    data["_latency"] = latency
    return data


def is_relevant(result: dict[str, Any], expected_keywords: list[str], expected_domains: list[str]) -> bool:
    """Check if a result is relevant based on expected keywords/domains."""
    text = f"{result.get('title', '')} {result.get('snippet', '')} {result.get('url', '')}".lower()
    url = result.get("url", "").lower()

    # Domain match
    for domain in expected_domains:
        if domain.lower() in url:
            return True

    # Keyword match (at least 2 keywords)
    matched = sum(1 for kw in expected_keywords if kw.lower() in text)
    return matched >= 2


def relevance_score(result: dict[str, Any], expected_keywords: list[str], expected_domains: list[str]) -> float:
    """Graded relevance: 0 (irrelevant) to 1 (highly relevant)."""
    text = f"{result.get('title', '')} {result.get('snippet', '')} {result.get('url', '')}".lower()
    url = result.get("url", "").lower()

    score = 0.0

    # Domain match gives 0.5
    for domain in expected_domains:
        if domain.lower() in url:
            score += 0.5
            break

    # Keyword match: each matched keyword adds (0.5 / len(keywords))
    if expected_keywords:
        matched = sum(1 for kw in expected_keywords if kw.lower() in text)
        score += 0.5 * (matched / len(expected_keywords))

    return min(score, 1.0)


def dcg(relevances: list[float], k: int) -> float:
    """Discounted Cumulative Gain at k."""
    return sum(rel / (i + 1) for i, rel in enumerate(relevances[:k]))


def ndcg_at_k(relevances: list[float], k: int) -> float:
    """Normalized DCG at k."""
    actual = dcg(relevances, k)
    ideal = dcg(sorted(relevances, reverse=True), k)
    return actual / ideal if ideal > 0 else 0.0


def evaluate_query(query_item: dict[str, Any], base_url: str, top_k: int) -> dict[str, Any]:
    """Evaluate a single query and return metrics."""
    query = query_item["query"]
    expected_keywords = query_item.get("expected_keywords", [])
    expected_domains = query_item.get("expected_domains", [])

    try:
        data = search(base_url, query, top_k)
    except Exception as e:
        return {
            "query": query,
            "category": query_item.get("category", "unknown"),
            "error": str(e),
            "latency": 0,
            "num_results": 0,
            "precision@k": 0,
            "recall": 0,
            "mrr": 0,
            "ndcg@k": 0,
            "engine_coverage": 0,
        }

    results = data.get("results", [])
    latency = data.get("_latency", 0)

    # Compute relevance for each result
    relevances = [relevance_score(r, expected_keywords, expected_domains) for r in results]
    relevant_flags = [is_relevant(r, expected_keywords, expected_domains) for r in results]

    # Precision@K
    precision = sum(relevant_flags[:top_k]) / min(len(results), top_k) if results else 0

    # Recall (assuming all relevant results are in the top-K; approximate)
    total_relevant = sum(relevant_flags)
    recall = total_relevant / max(len(expected_keywords), 1)  # rough upper bound
    recall = min(recall, 1.0)

    # MRR
    mrr = 0.0
    for i, rel in enumerate(relevant_flags):
        if rel:
            mrr = 1.0 / (i + 1)
            break

    # NDCG@K
    ndcg = ndcg_at_k(relevances, top_k)

    # Engine coverage
    engines = set()
    for r in results:
        for e in r.get("engines", []):
            engines.add(e)
    engine_coverage = len(engines)

    return {
        "query": query,
        "category": query_item.get("category", "unknown"),
        "latency": latency,
        "num_results": len(results),
        "precision@k": precision,
        "recall": recall,
        "mrr": mrr,
        "ndcg@k": ndcg,
        "engine_coverage": engine_coverage,
        "errors": len(data.get("errors", [])),
    }


def main():
    parser = argparse.ArgumentParser(description="Evaluate agent-search relevance")
    parser.add_argument("--base-url", default="http://127.0.0.1:8080", help="Search API base URL")
    parser.add_argument("--top-k", type=int, default=10, help="Number of results to evaluate")
    parser.add_argument("--queries", default="eval/queries.yaml", help="Path to queries YAML")
    parser.add_argument("--output", default=None, help="Output JSON path (optional)")
    args = parser.parse_args()

    queries = load_queries(args.queries)
    print(f"Evaluating {len(queries)} queries (top-{args.top_k}) against {args.base_url}\n")

    results = []
    for i, q in enumerate(queries, 1):
        metrics = evaluate_query(q, args.base_url, args.top_k)
        results.append(metrics)
        status = "ERR" if metrics.get("error") else "OK"
        print(
            f"[{i:2d}/{len(queries)}] {status} {metrics['query'][:50]:50s} "
            f"P@K={metrics['precision@k']:.2f} MRR={metrics['mrr']:.2f} "
            f"NDCG={metrics['ndcg@k']:.2f} latency={metrics['latency']:.1f}s"
        )

    # Aggregate metrics
    valid = [r for r in results if not r.get("error")]
    if not valid:
        print("\nNo successful queries.")
        return

    avg = lambda key: sum(r[key] for r in valid) / len(valid)

    print("\n" + "=" * 70)
    print("AGGREGATE METRICS")
    print("=" * 70)
    print(f"  Queries evaluated:  {len(valid)} / {len(results)}")
    print(f"  Avg Precision@{args.top_k}:  {avg('precision@k'):.4f}")
    print(f"  Avg Recall:         {avg('recall'):.4f}")
    print(f"  Avg MRR:            {avg('mrr'):.4f}")
    print(f"  Avg NDCG@{args.top_k}:     {avg('ndcg@k'):.4f}")
    print(f"  Avg Engine Coverage:{avg('engine_coverage'):.1f}")
    print(f"  Avg Latency:        {avg('latency'):.2f}s")
    print(f"  Avg Results/Query:  {avg('num_results'):.1f}")

    # Per-category breakdown
    categories = {}
    for r in valid:
        cat = r["category"]
        categories.setdefault(cat, []).append(r)

    print("\nPER-CATEGORY BREAKDOWN")
    print("-" * 70)
    for cat, cat_results in sorted(categories.items()):
        p = sum(r["precision@k"] for r in cat_results) / len(cat_results)
        m = sum(r["mrr"] for r in cat_results) / len(cat_results)
        n = sum(r["ndcg@k"] for r in cat_results) / len(cat_results)
        print(f"  {cat:15s} P@K={p:.3f} MRR={m:.3f} NDCG={n:.3f} (n={len(cat_results)})")

    if args.output:
        with open(args.output, "w") as f:
            json.dump({"metrics": results, "aggregate": {
                "avg_precision": avg("precision@k"),
                "avg_recall": avg("recall"),
                "avg_mrr": avg("mrr"),
                "avg_ndcg": avg("ndcg@k"),
                "avg_engine_coverage": avg("engine_coverage"),
                "avg_latency": avg("latency"),
            }}, f, indent=2)
        print(f"\nResults written to {args.output}")


if __name__ == "__main__":
    main()
