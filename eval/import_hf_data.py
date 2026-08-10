#!/usr/bin/env python3
"""Import high-quality datasets from HuggingFace into the local search index.

Currently supports:
- Stack Overflow questions + accepted answers (technical Q&A)
- Wikipedia articles (general knowledge)

Usage:
    python eval/import_hf_data.py --dataset stackoverflow --limit 10000
    python eval/import_hf_data.py --dataset wikipedia --limit 5000
"""

import argparse
import json
import sys
import time

import requests


def index_page(base_url: str, url: str, title: str, content: str) -> bool:
    """Index a single page via the /index endpoint."""
    try:
        resp = requests.post(
            f"{base_url}/index",
            json={"url": url, "title": title, "content": content},
            timeout=10,
        )
        return resp.status_code == 200
    except Exception:
        return False


def import_stackoverflow(base_url: str, limit: int):
    """Import Stack Overflow Q&A from HuggingFace."""
    from datasets import load_dataset

    # Use the tiny Stack Overflow dataset (<1GB) for fast import.
    print(f"Loading tiny-stackoverflow dataset (limit={limit})...", file=sys.stderr)
    try:
        ds = load_dataset("nampdn-ai/tiny-stackoverflow", split="train", streaming=True)
    except Exception:
        # Fall back to the larger dataset if the small one is unavailable.
        print("tiny-stackoverflow not available, falling back to stack-exchange-preferences", file=sys.stderr)
        ds = load_dataset("HuggingFaceH4/stack-exchange-preferences", split="train", streaming=True)

    indexed = 0
    skipped = 0
    for i, item in enumerate(ds):
        if indexed >= limit:
            break

        # Try to extract question and answer from various dataset formats.
        question = item.get("question") or item.get("title") or item.get("Question") or ""
        answers = item.get("answers") or item.get("answer") or item.get("Answer") or []

        # Handle different answer formats.
        if isinstance(answers, str):
            answer_text = answers
        elif isinstance(answers, list) and answers:
            if isinstance(answers[0], dict):
                best = max(answers, key=lambda a: a.get("pm_score", a.get("score", 0)))
                answer_text = best.get("answer", best.get("text", ""))
            else:
                answer_text = str(answers[0])
        else:
            answer_text = ""

        if not question or not answer_text:
            skipped += 1
            continue

        content = f"Question: {question}\n\nAnswer: {answer_text}"
        title = question[:200]
        url = f"https://stackoverflow.com/q/{hash(question) % 100000000}"

        if index_page(base_url, url, title, content):
            indexed += 1
            if indexed % 100 == 0:
                print(f"  indexed {indexed}/{limit} (skipped {skipped})", file=sys.stderr)
        else:
            skipped += 1

        time.sleep(0.01)

    print(f"Stack Overflow: indexed={indexed}, skipped={skipped}", file=sys.stderr)


def import_wikipedia(base_url: str, limit: int):
    """Import Wikipedia articles from HuggingFace."""
    from datasets import load_dataset

    print(f"Loading Wikipedia dataset (limit={limit})...", file=sys.stderr)
    ds = load_dataset("wikimedia/wikipedia", "20231101.en", split="train", streaming=True)

    indexed = 0
    for i, item in enumerate(ds):
        if indexed >= limit:
            break

        title = item.get("title", "")
        text = item.get("text", "")
        url = item.get("url", f"https://en.wikipedia.org/wiki/{title.replace(' ', '_')}")

        if not title or not text:
            continue

        if index_page(base_url, url, title, text):
            indexed += 1
            if indexed % 100 == 0:
                print(f"  indexed {indexed}/{limit}", file=sys.stderr)

        time.sleep(0.01)

    print(f"Wikipedia: indexed={indexed}", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description="Import HF datasets into search index")
    parser.add_argument("--base-url", default="http://127.0.0.1:18789")
    parser.add_argument("--dataset", choices=["stackoverflow", "wikipedia"], required=True)
    parser.add_argument("--limit", type=int, default=10000)
    args = parser.parse_args()

    if args.dataset == "stackoverflow":
        import_stackoverflow(args.base_url, args.limit)
    elif args.dataset == "wikipedia":
        import_wikipedia(args.base_url, args.limit)


if __name__ == "__main__":
    main()
