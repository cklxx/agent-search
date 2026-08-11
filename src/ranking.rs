//! Pluggable ranking strategies for A/B comparison.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use fastembed::{RerankInitOptions, RerankerModel, TextRerank};

use crate::models::query::SearchQuery;
use crate::models::result::RawSearchResult;

/// Domain authority boost for trusted sources in LLM/systems/academic search.
/// Higher = more authoritative for technical queries.
fn domain_authority(url: &str) -> f32 {
    let domain = url
        .split("://")
        .nth(1)
        .and_then(|s| s.split('/').next())
        .unwrap_or("")
        .to_lowercase();

    // Static authority table. (domain, boost). Suffix match: "wikipedia.org"
    // also covers "en.wikipedia.org".
    static AUTHORITIES: &[(&str, f32)] = &[
        // Academic
        ("arxiv.org", 1.5),
        ("scholar.google.com", 1.5),
        ("semanticscholar.org", 1.5),
        ("pubmed.ncbi.nlm.nih.gov", 1.4),
        ("nature.com", 1.4),
        ("science.org", 1.4),
        // Programming / systems
        ("stackoverflow.com", 1.4),
        ("serverfault.com", 1.3),
        ("superuser.com", 1.3),
        ("github.com", 0.7),
        ("gitlab.com", 0.6),
        ("developer.mozilla.org", 1.3),
        ("docs.rs", 1.3),
        ("doc.rust-lang.org", 1.3),
        ("rust-lang.org", 1.3),
        ("python.org", 1.3),
        ("go.dev", 1.3),
        ("golang.org", 1.3),
        ("kubernetes.io", 1.4),
        ("docker.com", 1.3),
        ("nginx.org", 1.3),
        ("man7.org", 1.4),
        ("kernel.org", 1.4),
        ("llvm.org", 1.4),
        ("redis.io", 1.3),
        ("postgresql.org", 1.3),
        ("mysql.com", 1.3),
        ("learn.microsoft.com", 1.3),
        // General knowledge
        ("wikipedia.org", 1.0),
        // LLM / AI
        ("huggingface.co", 1.4),
        ("openai.com", 1.3),
        ("anthropic.com", 1.3),
        // Low-quality mirrors / snapshots — downrank
        ("archive.org", 0.01),
        ("web.archive.org", 0.01),
    ];

    // Known content-farm / spam domains — hard downrank.
    static SPAM_DOMAINS: &[&str] = &[
        "medium.com",   // user-generated, often republished
        "dev.to",       // user-generated
        "hackernoon.com",
        "towardsdatascience.com",
        "javascript.plainenglish.io",
        "levelup.gitconnected.com",
    ];

    let authority = AUTHORITIES
        .iter()
        .filter(|(d, _)| domain_matches(&domain, d))
        .map(|(_, v)| *v)
        .fold(0.0_f32, f32::max);

    // No match in the authority table: default to 1.0 (neutral).
    let authority = if authority == 0.0 { 1.0 } else { authority };

    // If not in the authority table, check spam/TLD heuristics.
    if authority == 1.0 {
        // Spam domain blacklist.
        if SPAM_DOMAINS.iter().any(|d| domain_matches(&domain, d)) {
            return 0.5;
        }
        // Spam-heavy TLDs get a penalty unless whitelisted above.
        let tld = domain.rsplit('.').next().unwrap_or("");
        if matches!(tld, "xyz" | "top" | "click" | "info" | "site" | "online" | "icu") {
            return 0.5;
        }
    }

    authority
}

/// True if `domain` equals `candidate` or is a subdomain of it.
fn domain_matches(domain: &str, candidate: &str) -> bool {
    domain == candidate || domain.strip_suffix(candidate).is_some_and(|s| s.ends_with('.'))
}

pub trait RankingStrategy: Send + Sync {
    fn name(&self) -> &str;
    fn score(&self, raw: &RawSearchResult, query: &SearchQuery, engine_weight: f32, engines: &[String]) -> f32;

    /// Score a batch of results. Default implementation calls `score` per
    /// result. Strategies with batch-capable models (e.g. cross-encoders)
    /// should override this for efficiency.
    fn score_batch(
        &self,
        items: &[(RawSearchResult, Vec<String>, f32)],
        query: &SearchQuery,
    ) -> Vec<f32> {
        items
            .iter()
            .map(|(raw, engines, weight)| self.score(raw, query, *weight, engines))
            .collect()
    }
}

/// BM25 (simplified: TF, no IDF) × position × engine weight × domain authority × freshness. Default.
pub struct Bm25Strategy;

impl RankingStrategy for Bm25Strategy {
    fn name(&self) -> &str {
        "bm25"
    }

    fn score(&self, raw: &RawSearchResult, query: &SearchQuery, engine_weight: f32, _engines: &[String]) -> f32 {
        let bm25 = bm25_score(raw, query);
        // Gentler position decay: 1/log2(pos+1) so cross-engine position
        // differences don't overwhelm relevance and authority signals.
        let position_weight = 1.0 / (raw.position as f32 + 1.0).log2();
        let authority = domain_authority(&raw.url);
        let freshness = freshness_weight(raw.published_date);
        let score = bm25 * position_weight * engine_weight * authority * freshness;
        normalize(score)
    }
}

/// Exponential time decay: newer content ranks higher.
/// Half-life of 180 days: content 6 months old scores ~0.5, 1 year old ~0.25.
fn freshness_weight(published: Option<chrono::DateTime<chrono::Utc>>) -> f32 {
    let Some(pub_date) = published else {
        // No date: neutral weight.
        return 1.0;
    };
    let age_days = (chrono::Utc::now() - pub_date).num_days().max(0) as f32;
    let half_life = 180.0;
    (0.5_f32).powf(age_days / half_life)
}

fn bm25_score(raw: &RawSearchResult, query: &SearchQuery) -> f32 {
    let k1 = 1.2;

    // Tokenize each field separately for BM25F field weighting.
    let title_tokens = tokenize(&raw.title);
    let url_tokens = tokenize(&raw.url);
    let snippet_tokens = tokenize(&raw.snippet);

    // BM25F field weights: title > url > snippet.
    const TITLE_W: f32 = 3.0;
    const URL_W: f32 = 1.5;
    const SNIPPET_W: f32 = 1.0;

    // Deduplicate query terms. Use the same tokenizer as document fields so
    // Chinese 2-grams match between query and document.
    let query_terms: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        tokenize(&query.query)
            .into_iter()
            .filter(|t| seen.insert(t.clone()))
            .collect()
    };
    if query_terms.is_empty() {
        return 0.0;
    }

    let mut score = 0.0;
    let mut matched_terms = 0;
    for term in &query_terms {
        // Weighted term frequency across fields (BM25F).
        let tf_title = title_tokens.iter().filter(|t| *t == term).count() as f32;
        let tf_url = url_tokens.iter().filter(|t| *t == term).count() as f32;
        let tf_snippet = snippet_tokens.iter().filter(|t| *t == term).count() as f32;
        let tf = tf_title * TITLE_W + tf_url * URL_W + tf_snippet * SNIPPET_W;

        if tf > 0.0 {
            matched_terms += 1;
        }
        score += tf * (k1 + 1.0) / (tf + k1);
    }

    // Query coverage: boost results that match more query terms.
    let coverage = matched_terms as f32 / query_terms.len() as f32;
    score * (0.5 + 0.5 * coverage)
}

fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let lower = text.to_lowercase();

    // Split on whitespace and ASCII punctuation, then handle CJK runs with
    // character 2-grams. ASCII words are kept as-is (after lowercasing).
    for segment in lower.split(|c: char| c.is_whitespace() || c.is_ascii_punctuation()) {
        if segment.is_empty() {
            continue;
        }
        if segment.is_ascii() {
            tokens.push(segment.to_string());
        } else {
            // CJK or mixed: emit 2-character grams so Chinese queries match.
            let chars: Vec<char> = segment.chars().collect();
            if chars.len() == 1 {
                tokens.push(chars[0].to_string());
            } else {
                for w in chars.windows(2) {
                    tokens.push(format!("{}{}", w[0], w[1]));
                }
            }
        }
    }
    tokens
}

/// Extract the most query-relevant window from a long text.
///
/// Splits the text into overlapping windows of `target_len` characters and
/// returns the one with the highest query term overlap. This gives the
/// cross-encoder the most relevant context instead of the first N characters
/// (which are often navigation/boilerplate).
fn extract_relevant_snippet(text: &str, query: &str, target_len: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= target_len {
        return text.to_string();
    }

    let query_terms: std::collections::HashSet<String> = tokenize(query).into_iter().collect();
    if query_terms.is_empty() {
        return chars[..target_len].iter().collect();
    }

    // Step by half the target length for overlapping windows.
    let step = target_len / 2;
    let mut best_start = 0;
    let mut best_score = 0usize;

    let mut start = 0;
    while start < chars.len() {
        let end = (start + target_len).min(chars.len());
        let window: String = chars[start..end].iter().collect();
        let window_tokens = tokenize(&window);
        let score = window_tokens
            .iter()
            .filter(|t| query_terms.contains(*t))
            .count();
        if score > best_score {
            best_score = score;
            best_start = start;
        }
        if end >= chars.len() {
            break;
        }
        start += step;
    }

    let end = (best_start + target_len).min(chars.len());
    chars[best_start..end].iter().collect()
}

/// Normalize a non-negative score to [0, 1) while preserving order.
/// Unlike sigmoid, 0 maps to 0.
fn normalize(score: f32) -> f32 {
    score / (1.0 + score)
}

/// Returns true if the query contains a technical acronym (a run of 2+
/// uppercase ASCII letters, e.g. "RAG", "GPTQ", "HTTP/2", "KV-cache").
/// The cross-encoder often fails to interpret these correctly, so BM25
/// exact-match gets more weight for such queries.
fn has_technical_acronym(query: &str) -> bool {
    query.split_whitespace().any(|w| {
        let mut upper_run = 0;
        for c in w.chars() {
            if c.is_ascii_uppercase() {
                upper_run += 1;
                if upper_run >= 2 {
                    return true;
                }
            } else {
                upper_run = 0;
            }
        }
        false
    })
}

/// TF-IDF. IDF approximated from term length (no corpus stats).
pub struct TfIdfStrategy;

impl RankingStrategy for TfIdfStrategy {
    fn name(&self) -> &str {
        "tfidf"
    }

    fn score(&self, raw: &RawSearchResult, query: &SearchQuery, engine_weight: f32, _engines: &[String]) -> f32 {
        let tokens = tokenize(&format!("{} {} {}", raw.title, raw.url, raw.snippet));
        let doc_len = tokens.len().max(1) as f32;

        let query_terms: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            tokenize(&query.query)
                .into_iter()
                .filter(|t| seen.insert(t.clone()))
                .collect()
        };
        if query_terms.is_empty() {
            return 0.0;
        }

        let mut tfidf = 0.0;
        for term in &query_terms {
            let tf = tokens.iter().filter(|t| t == &term).count() as f32 / doc_len;
            let idf = 1.0 + (term.len() as f32).ln();
            tfidf += tf * idf;
        }

        let position_weight = 1.0 / (raw.position as f32 + 1.0).log2();
        let authority = domain_authority(&raw.url);
        let freshness = freshness_weight(raw.published_date);
        let score = tfidf * position_weight * engine_weight * authority * freshness * 100.0;
        normalize(score)
    }
}

/// Rank by engine result position only. Baseline for measuring relevance gain.
pub struct PositionOnlyStrategy;

impl RankingStrategy for PositionOnlyStrategy {
    fn name(&self) -> &str {
        "position_only"
    }

    fn score(&self, raw: &RawSearchResult, _query: &SearchQuery, engine_weight: f32, _engines: &[String]) -> f32 {
        let pos_score = 1.0 / (raw.position as f32 + 1.0).log2();
        let authority = domain_authority(&raw.url);
        let freshness = freshness_weight(raw.published_date);
        let score = pos_score * engine_weight * authority * freshness;
        normalize(score)
    }
}

/// Rank by engine weight only. For testing weight configuration.
pub struct EngineWeightStrategy;

impl RankingStrategy for EngineWeightStrategy {
    fn name(&self) -> &str {
        "engine_weight"
    }

    fn score(&self, _raw: &RawSearchResult, _query: &SearchQuery, engine_weight: f32, _engines: &[String]) -> f32 {
        normalize(engine_weight)
    }
}

/// BM25 + boost when query terms appear in the title.
pub struct Bm25TitleBoostStrategy;

impl RankingStrategy for Bm25TitleBoostStrategy {
    fn name(&self) -> &str {
        "bm25_title_boost"
    }

    fn score(&self, raw: &RawSearchResult, query: &SearchQuery, engine_weight: f32, _engines: &[String]) -> f32 {
        let bm25 = bm25_score(raw, query);

        let query_terms: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            tokenize(&query.query)
                .into_iter()
                .filter(|t| seen.insert(t.clone()))
                .collect()
        };
        let title_tokens = tokenize(&raw.title);
        let title_match = query_terms
            .iter()
            .filter(|t| title_tokens.iter().any(|tok| tok == *t))
            .count() as f32
            / query_terms.len().max(1) as f32;

        let position_weight = 1.0 / (raw.position as f32 + 1.0).log2();
        let authority = domain_authority(&raw.url);
        let freshness = freshness_weight(raw.published_date);
        let score = (bm25 + title_match * 2.0) * position_weight * engine_weight * authority * freshness;
        normalize(score)
    }
}

/// SearXNG-only baseline: rank results returned by SearXNG by their native
/// position. Non-SearXNG results score 0. Used to measure the value of
/// multi-engine aggregation + re-ranking over a single upstream.
pub struct SearxngOnlyStrategy;

impl RankingStrategy for SearxngOnlyStrategy {
    fn name(&self) -> &str {
        "searxng_only"
    }

    fn score(&self, raw: &RawSearchResult, _query: &SearchQuery, _engine_weight: f32, engines: &[String]) -> f32 {
        if !engines.iter().any(|e| e == "searxng") {
            return 0.0;
        }
        let pos_score = 1.0 / (raw.position as f32 + 1.0).log2();
        let authority = domain_authority(&raw.url);
        let freshness = freshness_weight(raw.published_date);
        let score = pos_score * authority * freshness;
        normalize(score)
    }
}

/// Cross-encoder reranking with jina-reranker-v2-base-multilingual.
/// Uses a pool of reranker instances so concurrent requests don't serialize
/// on a single Mutex. Each instance independently rebuilds to reclaim onnx
/// runtime memory.
pub struct BgeRerankerStrategy {
    pool: Vec<Arc<Mutex<TextRerank>>>,
    call_counts: Vec<AtomicU64>,
    next: AtomicUsize,
}

impl BgeRerankerStrategy {
    const INTRA_THREADS: usize = 2;

    pub fn new() -> anyhow::Result<Self> {
        // Pool size and intra_threads are tuned so total onnx threads ≈ CPU
        // cores: pool_size * intra_threads ≈ cores. On machines with many
        // cores (e.g. 64), we use a larger pool to parallelize across
        // concurrent requests. Each instance uses ~1.4GB memory.
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let intra_threads = Self::INTRA_THREADS;
        // Cap pool size at 8 to bound memory (~11GB for 8 instances).
        let pool_size = (cores / intra_threads).clamp(1, 8);
        let mut pool = Vec::with_capacity(pool_size);
        let mut call_counts = Vec::with_capacity(pool_size);
        for _ in 0..pool_size {
            pool.push(Arc::new(Mutex::new(Self::build_reranker(intra_threads)?)));
            call_counts.push(AtomicU64::new(0));
        }
        Ok(Self {
            pool,
            call_counts,
            next: AtomicUsize::new(0),
        })
    }

    fn build_reranker(intra_threads: usize) -> anyhow::Result<TextRerank> {
        let mut options = RerankInitOptions::default();
        options.model_name = RerankerModel::JINARerankerV2BaseMultiligual;
        options.cache_dir = std::path::PathBuf::from("models");
        options.intra_threads = Some(intra_threads);
        // 512 tokens matches the model's max position embeddings and gives
        // the cross-encoder fuller context from titles + snippets.
        options.max_length = 512;
        TextRerank::try_new(options)
    }
}

impl RankingStrategy for BgeRerankerStrategy {
    fn name(&self) -> &str {
        "bge_reranker"
    }

    fn score(&self, raw: &RawSearchResult, query: &SearchQuery, engine_weight: f32, _engines: &[String]) -> f32 {
        let items = [(raw.clone(), Vec::new(), engine_weight)];
        self.score_batch(&items, query).pop().unwrap_or(0.0)
    }

    fn score_batch(
        &self,
        items: &[(RawSearchResult, Vec<String>, f32)],
        query: &SearchQuery,
    ) -> Vec<f32> {
        if items.is_empty() {
            return Vec::new();
        }

        const TOP_N: usize = 50;
        const REBUILD_EVERY: u64 = 50;

        // Coarse rank with BM25 term frequency + position to select the top-N
        // candidates for cross-encoder reranking. Authority, freshness, and
        // engine weight are intentionally excluded here: they modulate the
        // final score, not which candidates get reranked (the cross-encoder
        // decides relevance).
        let bm25_scores: Vec<f32> = items
            .iter()
            .map(|(raw, _engines, _weight)| {
                let bm25 = bm25_score(raw, query);
                let position_weight = 1.0 / (raw.position as f32 + 1.0).log2();
                bm25 * position_weight
            })
            .collect();

        let mut indices: Vec<usize> = (0..items.len()).collect();
        indices.sort_by(|&a, &b| {
            bm25_scores[b]
                .partial_cmp(&bm25_scores[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let top_indices: Vec<usize> = indices.into_iter().take(TOP_N).collect();

        let documents: Vec<String> = top_indices
            .iter()
            .map(|&i| {
                let (raw, _, _) = &items[i];
                // Query-aware snippet: find the window of content with the
                // most query term matches. The first 1000 chars are often
                // navigation/boilerplate, which gives the cross-encoder
                // little signal to judge relevance.
                let snippet = extract_relevant_snippet(&raw.snippet, &query.query, 1000);
                let mut doc = format!("{} {}", raw.title, snippet);
                // Truncate to 2048 chars. max_length=512 tokens (~1500 chars for
                // multilingual text), so this gives the cross-encoder fuller
                // context without exceeding the model's token limit.
                if let Some((idx, _)) = doc.char_indices().nth(2048) {
                    doc.truncate(idx);
                }
                doc
            })
            .collect();

        let doc_refs: Vec<&str> = documents.iter().map(|s| s.as_str()).collect();

        // Round-robin pick a reranker instance from the pool.
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.pool.len();
        let count = self.call_counts[idx].fetch_add(1, Ordering::Relaxed);
        let should_rebuild = count > 0 && count.is_multiple_of(REBUILD_EVERY);

        // Query-dependent blend weight. The cross-encoder
        // (jina-reranker-v2-base-multilingual) underperforms on queries with
        // technical acronyms (RAG, GPTQ, CRISPR, ...) where exact keyword
        // matching is more reliable. For those queries, lean more on BM25.
        let (bm25_weight, ce_weight) = if has_technical_acronym(&query.query) {
            (0.55, 0.45)
        } else {
            (0.45, 0.55)
        };

        let mut scores = vec![0.0; items.len()];

        {
            let mut reranker = match self.pool[idx].lock() {
                Ok(r) => r,
                Err(_) => return scores,
            };

            if let Ok(results) = reranker.rerank(query.query.as_str(), doc_refs, false, None) {
                for r in results.iter() {
                    // `rerank` returns results sorted by score descending.
                    // `r.index` is the position in the input documents slice.
                    let orig_idx = top_indices[r.index];
                    let (raw, _, weight) = &items[orig_idx];
                    let authority = domain_authority(&raw.url);
                    let freshness = freshness_weight(raw.published_date);
                    // Cross-encoder returns raw logits (can be negative).
                    // Sigmoid maps to (0, 1).
                    let relevance = 1.0 / (1.0 + (-r.score).exp());
                    // BM25 as a keyword-matching floor. normalize() maps to [0,1).
                    let bm25_norm = normalize(bm25_scores[orig_idx]);
                    let blended = bm25_weight * bm25_norm + ce_weight * relevance;
                    // Authority, engine weight, and freshness as multipliers,
                    // matching the bm25 strategy's treatment. Clamp each to
                    // avoid overwhelming the cross-encoder relevance signal.
                    let authority_factor = authority.clamp(0.3, 1.3);
                    let weight_factor = weight.clamp(0.7, 1.3);
                    let raw_score = blended * authority_factor * weight_factor * freshness;
                    scores[orig_idx] = raw_score.clamp(0.0, 1.0);
                }
            } else {
                tracing::error!("rerank failed for query: {}", query.query);
            }
        }

        // Rebuild outside the lock: model loading takes hundreds of ms, and
        // we don't want to block other requests round-robined onto this slot.
        if should_rebuild {
            // intra_threads must match the original build; use 2 to keep
            // in sync with BgeRerankerStrategy::new.
            if let Ok(new_reranker) = Self::build_reranker(2) {
                if let Ok(mut reranker) = self.pool[idx].lock() {
                    *reranker = new_reranker;
                }
            } else {
                tracing::error!("reranker rebuild failed");
            }
        }

        scores
    }
}

/// Cached bge_reranker instance. Loading the model pool is expensive
/// (~2.4GB for 8 instances), so we build it once and reuse it across
/// /search/ab requests.
static BGE_RERANKER: OnceLock<Option<Arc<dyn RankingStrategy>>> = OnceLock::new();

pub fn get_strategy(name: &str) -> Option<Arc<dyn RankingStrategy>> {
    match name {
        "bm25" => Some(Arc::new(Bm25Strategy)),
        "tfidf" => Some(Arc::new(TfIdfStrategy)),
        "position_only" => Some(Arc::new(PositionOnlyStrategy)),
        "engine_weight" => Some(Arc::new(EngineWeightStrategy)),
        "bm25_title_boost" => Some(Arc::new(Bm25TitleBoostStrategy)),
        "searxng_only" => Some(Arc::new(SearxngOnlyStrategy)),
        "bge_reranker" => {
            let cached = BGE_RERANKER.get_or_init(|| {
                match BgeRerankerStrategy::new() {
                    Ok(s) => Some(Arc::new(s)),
                    Err(e) => {
                        eprintln!("Failed to load jina-reranker-v2: {}", e);
                        None
                    }
                }
            });
            cached.clone()
        }
        _ => None,
    }
}

pub fn strategy_names() -> Vec<&'static str> {
    vec![
        "bm25",
        "tfidf",
        "position_only",
        "engine_weight",
        "bm25_title_boost",
        "searxng_only",
        "bge_reranker",
    ]
}
