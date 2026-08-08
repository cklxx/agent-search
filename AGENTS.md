# Agent Search — Agent Contract

Assisting **ckl**. Project caveats and hard gates only — generic Rust / HTTP /
search knowledge is intentionally absent, and so is anything you can read off
the file tree. Match the surrounding code's idiom, naming, and comment density.

**Load on demand, not upfront:**

| When | Read |
|------|------|
| Engine config format + examples | `src/engine/config.rs`, `engines.yaml` |
| Ranking strategies | `src/ranking.rs` |
| Aggregation / dedup / scoring flow | `src/aggregator.rs` |
| Suspension backoff rules | `src/engine/suspension.rs` |
| Eval queries & metrics | `eval/queries.yaml`, `eval/run_eval.py` |

`AGENTS.md` is canonical; `CLAUDE.md` is a symlink to it.

---

## Project shape

`agent-search` is a high-performance search engine for AI agents. It fans out
to multiple upstream engines (SearXNG + native HTML/JSON engines), deduplicates
by normalized URL, scores with a pluggable ranking strategy (BM25 default),
and returns structured results. A local Tantivy index caches results for
repeat queries.

Request flow (`POST /search`):
1. `QueryCache` (moka) — exact key `query:page:max_results`, TTL from config.
2. `LocalIndex` (Tantivy) — BM25 similarity over past cached results.
3. `aggregator::aggregate` — `fetch_raw_results` (select engines by category,
   fan out to non-suspended engines, dedup by normalized URL) then
   `score_results` (score with the active `RankingStrategy`, sort descending,
   truncate to `max_results`).
4. Write back to both `LocalIndex` and `QueryCache`.

Key seams:
- **`SearchEngine` trait** (`src/engine/trait_def.rs`) — one impl per engine.
  All engines are `ConfigurableEngine` instances loaded from `./engines.yaml`.
  No built-in Rust engine impls — YAML is the single source of truth.
- **`RankingStrategy` trait** (`src/ranking.rs`) — one impl per scoring
  strategy. A/B compare via `POST /search/ab`. Adding a strategy: impl the
  trait, add a match arm in `get_strategy`, add the name to `strategy_names`.
- **`EngineSuspensionManager`** — exponential backoff per error type.

Non-obvious ownership:
- **All engines are configured in `engines.yaml`** (no Rust engine impls).
  YAML is the single source of truth — add/modify engines there.
- **Category routing.** `select_engines` always includes `general` engines,
  plus engines from categories inferred from query keywords (`infer_categories`
  in `src/models/query.rs`). A query with "rust" or "python" also hits `it`
  engines; "arxiv" or "paper" hits `science` engines. The `category` field
  in `SearchQuery` overrides inference.
- **Dedup key = `normalize_url`** (`src/dedup.rs`): lowercases host, strips
  tracking params (`utm_*`, `fbclid`, `gclid`, `_ga`, `ref`, …), removes
  trailing slash and fragment. When multiple engines return the same URL,
  the scoring weight is the **max** across those engines.
- **Suspension is per-engine-name, not per-proxy.** A 403 suspends the whole
  engine for 180s even if another proxy in the pool would work.
- **A/B fetches once, scores twice.** `/search/ab` calls `fetch_raw_results`
  once, then `score_results` with each strategy — no upstream variance.
- **BM25 is the default strategy** — simplified in-process TF score (no IDF,
  no Tantivy). The Tantivy `LocalIndex` is only for the query-result cache.
- **Strategy is global** (from `config.toml`). `/search` ignores any
  `strategy` field in the body; only `/search/ab` takes `strategy_a`/
  `strategy_b` and returns 400 on unknown names.
- **Proxy precedence:** engine `proxy` field (wrapped as a single-element
  pool) > global `proxy_urls` pool > none.

---

## HTTP API

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/health` | Liveness check |
| GET | `/engines` | Registered engine names |
| GET | `/strategies` | Available ranking strategy names |
| POST | `/search` | Search (body: `SearchQuery`) |
| POST | `/search/ab` | A/B two strategies on one query |
| POST | `/search/stream` | SSE stream of results |
| GET | `/content?url=` | Fetch and extract page main text |

`SearchQuery` fields: `query`, `max_results` (default 10), `page` (0-indexed),
`language`, `time_range` (`day`/`week`/`month`/`year`), `safe_search` (0/1/2),
`category` (optional, overrides keyword-based category inference).

---

## Hard gates

**Every ranking-strategy change runs the eval harness.**
`python eval/run_eval.py --strategy-a <baseline> --strategy-b <new>`.
No strategy change ships without a before/after metrics table.

Primary metrics (ship decision): **NDCG@10**, **MRR**.
Secondary: precision@10.
Diagnostic (report, don't weight): recall (approximate — keyword-ratio
based), engine_coverage, avg_overlap.

Ship only if NDCG and MRR both improve or are neutral with no regression.
Treat deltas < 0.02 as noise; check per-category breakdown. The relevance
scorer uses domain + keyword matching, so don't over-optimize for it.

In scope: `src/ranking.rs`, `src/aggregator.rs`, `src/dedup.rs`, engine
weights in `engines.yaml`. Exempt: docs / agent files.

**No `Box::leak` for `'static`.** If a trait needs `'static str` but the value
is dynamic, change the trait signature to `&str` — don't leak.

**No string-matching on error messages for control flow.** HTTP status codes
are structured data — use `SearchError::HttpStatus(u16)`, match on the variant
in `suspension_duration`. Never parse status out of `Request(String)`.

**All `RankingStrategy::score` returns `[0.0, 1.0]`.** All strategies apply
a sigmoid to the raw score. If you add a new strategy, apply sigmoid or clamp.

**Unknown strategy names error.** `get_strategy` returns `Option`; `/search/ab`
returns 400 on unknown names. Never silently fall back to BM25 in request
paths (startup config may fall back with a warning).

**No half-states.** Finish a refactor unit or revert it; never leave parallel
old+new paths in the tree. Example: don't add a `RankingStrategy` impl
without also adding it to `get_strategy` and `strategy_names`.

**Approach-first for >3 files or architectural decisions** — list the files
you'll touch and the one-line change per file, then execute. "Architectural" =
changing a trait signature, the aggregator flow, engine registration, or the
cache layering. Wait for the user ONLY when a real tradeoff exists.

---

## Working rules

**Phases** (non-trivial tasks): Explore → Plan (>5 files or irreversible →
stop and flag) → Implement → Verify (`cargo check`, `cargo test`,
`cargo clippy -- -D warnings`, eval if ranking changed) → Reflect.
Trivial → Implement + Verify.

**Tests: minimal and unit-level.** Default is no new test. Add one only when
the change carries logic that can silently break (dedup, scoring, suspension),
and then the smallest gate that fails when it breaks. Existing tests are unit
tests on pure functions (`dedup.rs`, `proxy.rs`, `suspension.rs`). The eval
harness serves as the integration test for ranking.

**Delegation.** Independent tasks go out in parallel. Two failed subagent
attempts → hand-write the diff.

**Git.** Commit directly to `main`. Small tranches, each self-contained,
simplify pass first. Ranking changes commit only after eval passes.

**Code layout.** Comment only when the reason is not visible from the code
itself — a cross-module decision, an external-API quirk, or a workaround.
If the line below explains itself, omit the comment.
Example: `/// Proxy precedence: engine proxy > global pool > none.`
explains a non-local decision; `// build headers` does not.

**Memory.** When the user corrects an approach, record it as a one-line note
in the project memory file. No generic summaries — only corrections that
change future behavior.

---

## Writing (reports & experiment notes)

1. **Standard terms.** Use the established term; do not coin a new one.
2. **No metaphors.** Name the referent directly.
3. **Neutral headers.** Table headers are neutral nouns.
4. **No "X, not Y" sentences.** State the fact directly.
5. **Unknown cause.** Write "cause unknown"; do not append unverified
   explanations.
6. **No colloquialisms.** Formal, precise wording only.

---

## Build & run

```bash
cargo check                    # first gate — must compile
cargo build --release
cargo test
cargo clippy -- -D warnings

# Start the server (reads config.toml, engines.yaml)
cargo run --release

# Run the eval harness (server must be running)
python eval/run_eval.py --strategy-a bm25 --strategy-b tfidf
```
