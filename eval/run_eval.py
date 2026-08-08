#!/usr/bin/env python3
"""AB evaluation harness for agent-search.

Runs a query set against two ranking strategies and compares their metrics.

Usage:
    # Compare bm25 vs tfidf
    python eval/run_eval.py --strategy-a bm25 --strategy-b tfidf

    # Single strategy evaluation
    python eval/run_eval.py --strategy bm25
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
        return yaml.safe_load(f)["queries"]


def search(base_url: str, query: str, max_results: int, strategy: str | None = None) -> dict[str, Any]:
    """Run a search with an optional strategy override."""
    url = f"{base_url}/search"
    payload = {"query": query, "max_results": max_results}
    if strategy:
        payload["strategy"] = strategy
    start = time.time()
    resp = requests.post(url, json=payload, timeout=30)
    latency = time.time() - start
    resp.raise_for_status()
    data = resp.json()
    data["_latency"] = latency
    return data


def search_ab(base_url: str, query: str, max_results: int, strategy_a: str, strategy_b: str) -> dict[str, Any]:
    """Run A/B comparison for a single query."""
    url = f"{base_url}/search/ab"
    payload = {
        "query": query,
        "strategy_a": strategy_a,
        "strategy_b": strategy_b,
        "max_results": max_results,
    }
    start = time.time()
    resp = requests.post(url, json=payload, timeout=60)
    latency = time.time() - start
    resp.raise_for_status()
    data = resp.json()
    data["_latency"] = latency
    return data


def relevance_score(result: dict, expected_keywords: list[str], expected_domains: list[str]) -> float:
    text = f"{result.get('title','')} {result.get('snippet','')} {result.get('url','')}".lower()
    url = result.get("url", "").lower()
    score = 0.0
    for domain in expected_domains:
        if domain.lower() in url:
            score += 0.5
            break
    if expected_keywords:
        matched = sum(1 for kw in expected_keywords if kw.lower() in text)
        score += 0.5 * (matched / len(expected_keywords))
    return min(score, 1.0)


def is_relevant(result: dict, expected_keywords: list[str], expected_domains: list[str]) -> bool:
    return relevance_score(result, expected_keywords, expected_domains) >= 0.5


def dcg(relevances: list[float], k: int) -> float:
    return sum(rel / (i + 1) for i, rel in enumerate(relevances[:k]))


def ndcg_at_k(relevances: list[float], k: int) -> float:
    actual = dcg(relevances, k)
    ideal = dcg(sorted(relevances, reverse=True), k)
    return actual / ideal if ideal > 0 else 0.0


def compute_metrics(results: list[dict], expected_keywords: list[str], expected_domains: list[str], top_k: int) -> dict:
    relevances = [relevance_score(r, expected_keywords, expected_domains) for r in results]
    relevant_flags = [is_relevant(r, expected_keywords, expected_domains) for r in results]

    precision = sum(relevant_flags[:top_k]) / min(len(results), top_k) if results else 0
    total_relevant = sum(relevant_flags)
    recall = min(total_relevant / max(len(expected_keywords), 1), 1.0)

    mrr = 0.0
    for i, rel in enumerate(relevant_flags):
        if rel:
            mrr = 1.0 / (i + 1)
            break

    ndcg = ndcg_at_k(relevances, top_k)

    engines = set()
    for r in results:
        for e in r.get("engines", []):
            engines.add(e)

    return {
        "precision": precision,
        "recall": recall,
        "mrr": mrr,
        "ndcg": ndcg,
        "engine_coverage": len(engines),
        "num_results": len(results),
    }


def evaluate_ab(queries: list[dict], base_url: str, top_k: int, strategy_a: str, strategy_b: str) -> dict:
    """Run A/B evaluation across all queries."""
    all_metrics_a = []
    all_metrics_b = []
    overlaps = []

    for i, q in enumerate(queries, 1):
        query = q["query"]
        kw = q.get("expected_keywords", [])
        domains = q.get("expected_domains", [])

        try:
            data = search_ab(base_url, query, top_k, strategy_a, strategy_b)
        except Exception as e:
            print(f"[{i:2d}/{len(queries)}] ERR {query[:40]:40s} {e}")
            continue

        results_a = data.get("results_a", [])
        results_b = data.get("results_b", [])
        overlap = data.get("overlap", 0)
        overlaps.append(overlap)

        ma = compute_metrics(results_a, kw, domains, top_k)
        mb = compute_metrics(results_b, kw, domains, top_k)
        all_metrics_a.append(ma)
        all_metrics_b.append(mb)

        print(
            f"[{i:2d}/{len(queries)}] {query[:40]:40s} "
            f"A:P={ma['precision']:.2f}/NDCG={ma['ndcg']:.2f} "
            f"B:P={mb['precision']:.2f}/NDCG={mb['ndcg']:.2f} "
            f"overlap={overlap}"
        )

    def avg(key, metrics):
        return sum(m[key] for m in metrics) / len(metrics) if metrics else 0

    return {
        "strategy_a": strategy_a,
        "strategy_b": strategy_b,
        "num_queries": len(all_metrics_a),
        "avg_overlap": sum(overlaps) / len(overlaps) if overlaps else 0,
        "a": {
            "precision": avg("precision", all_metrics_a),
            "recall": avg("recall", all_metrics_a),
            "mrr": avg("mrr", all_metrics_a),
            "ndcg": avg("ndcg", all_metrics_a),
            "engine_coverage": avg("engine_coverage", all_metrics_a),
        },
        "b": {
            "precision": avg("precision", all_metrics_b),
            "recall": avg("recall", all_metrics_b),
            "mrr": avg("mrr", all_metrics_b),
            "ndcg": avg("ndcg", all_metrics_b),
            "engine_coverage": avg("engine_coverage", all_metrics_b),
        },
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", default="http://127.0.0.1:8080")
    parser.add_argument("--top-k", type=int, default=10)
    parser.add_argument("--queries", default="eval/queries.yaml")
    parser.add_argument("--strategy-a", default="bm25")
    parser.add_argument("--strategy-b", default="tfidf")
    parser.add_argument("--output", default=None)
    args = parser.parse_args()

    queries = load_queries(args.queries)
    print(f"AB eval: {args.strategy_a} vs {args.strategy_b} ({len(queries)} queries, top-{args.top_k})\n")

    result = evaluate_ab(queries, args.base_url, args.top_k, args.strategy_a, args.strategy_b)

    print("\n" + "=" * 60)
    print(f"{'METRIC':20s} {'A ('+args.strategy_a+')':>12s} {'B ('+args.strategy_b+')':>12s} {'DELTA':>10s}")
    print("-" * 60)
    for metric in ["precision", "recall", "mrr", "ndcg", "engine_coverage"]:
        a_val = result["a"][metric]
        b_val = result["b"][metric]
        delta = b_val - a_val
        sign = "+" if delta >= 0 else ""
        print(f"{metric:20s} {a_val:12.4f} {b_val:12.4f} {sign}{delta:>9.4f}")
    print(f"{'avg_overlap':20s} {result['avg_overlap']:12.1f}")
    print("=" * 60)

    if args.output:
        with open(args.output, "w") as f:
            json.dump(result, f, indent=2)
        print(f"\nResults written to {args.output}")


if __name__ == "__main__":
    main()
