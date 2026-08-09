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
        ("stackoverflow.com", 1.5),
        ("serverfault.com", 1.4),
        ("superuser.com", 1.4),
        ("github.com", 1.4),
        ("gitlab.com", 1.3),
        ("developer.mozilla.org", 1.4),
        ("docs.rs", 1.4),
        ("doc.rust-lang.org", 1.4),
        ("rust-lang.org", 1.4),
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
        ("wikipedia.org", 1.3),
        // LLM / AI
        ("huggingface.co", 1.4),
        ("openai.com", 1.3),
        ("anthropic.com", 1.3),
        // Low-quality mirrors / snapshots — downrank
        ("archive.org", 0.5),
        ("web.archive.org", 0.5),
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
        .filter(|(d, _)| domain == **d || domain.ends_with(&format!(".{}", d)))
        .map(|(_, v)| *v)
        .fold(1.0_f32, f32::max);

    // If not in the authority table, check spam/TLD heuristics.
    if authority == 1.0 {
        // Spam domain blacklist.
        if SPAM_DOMAINS.iter().any(|d| domain == *d || domain.ends_with(&format!(".{}", d))) {
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

/// BM25 (simplified: TF, no IDF) × position × engine weight × domain authority. Default.
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
        let score = bm25 * position_weight * engine_weight * authority;
        normalize(score)
    }
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

    // Deduplicate query terms.
    let query_terms: Vec<&str> = {
        let mut seen = std::collections::HashSet::new();
        query
            .query
            .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
            .filter(|t| !t.is_empty())
            .filter(|t| seen.insert(t.to_lowercase()))
            .collect()
    };
    if query_terms.is_empty() {
        return 0.0;
    }

    let mut score = 0.0;
    let mut matched_terms = 0;
    for term in &query_terms {
        let term_lower = term.to_lowercase();
        // Weighted term frequency across fields (BM25F).
        let tf_title = title_tokens.iter().filter(|t| **t == term_lower).count() as f32;
        let tf_url = url_tokens.iter().filter(|t| **t == term_lower).count() as f32;
        let tf_snippet = snippet_tokens.iter().filter(|t| **t == term_lower).count() as f32;
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
    text.to_lowercase()
        .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|t| !t.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Normalize a non-negative score to [0, 1) while preserving order.
/// Unlike sigmoid, 0 maps to 0.
fn normalize(score: f32) -> f32 {
    score / (1.0 + score)
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

        let query_terms: Vec<&str> = {
            let mut seen = std::collections::HashSet::new();
            query
                .query
                .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
                .filter(|t| !t.is_empty())
                .filter(|t| seen.insert(t.to_lowercase()))
                .collect()
        };
        if query_terms.is_empty() {
            return 0.0;
        }

        let mut tfidf = 0.0;
        for term in &query_terms {
            let term_lower = term.to_lowercase();
            let tf = tokens.iter().filter(|t| *t == &term_lower).count() as f32 / doc_len;
            let idf = 1.0 + (term.len() as f32).ln();
            tfidf += tf * idf;
        }

        let position_weight = 1.0 / (raw.position as f32 + 1.0).log2();
        let authority = domain_authority(&raw.url);
        let score = tfidf * position_weight * engine_weight * authority * 100.0;
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
        let score = pos_score * engine_weight * authority;
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

        let query_terms: Vec<&str> = {
            let mut seen = std::collections::HashSet::new();
            query
                .query
                .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
                .filter(|t| !t.is_empty())
                .filter(|t| seen.insert(t.to_lowercase()))
                .collect()
        };
        let title_tokens = tokenize(&raw.title);
        let title_match = query_terms
            .iter()
            .filter(|t| title_tokens.iter().any(|tok| tok == &t.to_lowercase()))
            .count() as f32
            / query_terms.len() as f32;

        let position_weight = 1.0 / (raw.position as f32 + 1.0).log2();
        let authority = domain_authority(&raw.url);
        let score = (bm25 + title_match * 2.0) * position_weight * engine_weight * authority;
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
        let score = pos_score * authority;
        normalize(score)
    }
}

/// Cross-encoder reranking with jina-reranker-v2-base-multilingual.
/// Uses a pool of reranker instances so concurrent requests don't serialize
/// on a single Mutex. Each instance independently rebuilds to reclaim onnx
/// runtime memory.
pub struct BgeRerankerStrategy {
    pool: Vec<Mutex<TextRerank>>,
    call_counts: Vec<AtomicU64>,
    next: AtomicUsize,
}

impl BgeRerankerStrategy {
    pub fn new() -> anyhow::Result<Self> {
        // Pool size = min(CPU cores, 8). Each instance loads the full model
        // (~300MB), so cap to avoid excessive memory.
        let pool_size = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(8);
        let mut pool = Vec::with_capacity(pool_size);
        let mut call_counts = Vec::with_capacity(pool_size);
        for _ in 0..pool_size {
            pool.push(Mutex::new(Self::build_reranker()?));
            call_counts.push(AtomicU64::new(0));
        }
        Ok(Self {
            pool,
            call_counts,
            next: AtomicUsize::new(0),
        })
    }

    fn build_reranker() -> anyhow::Result<TextRerank> {
        let mut options = RerankInitOptions::default();
        options.model_name = RerankerModel::JINARerankerV2BaseMultiligual;
        options.cache_dir = std::path::PathBuf::from("models");
        // One thread per instance: with N pool instances we get N-way
        // parallelism without oversubscribing the CPU.
        options.intra_threads = Some(1);
        // Cap sequence length to keep rerank latency and memory bounded.
        // 1024 chars ≈ 256–512 tokens; 512 tokens gives the cross-encoder
        // enough context for technical content without blowing up memory.
        options.max_length = 512;
        TextRerank::try_new(options)
    }
}

impl RankingStrategy for BgeRerankerStrategy {
    fn name(&self) -> &str {
        "bge_reranker"
    }

    fn score(&self, raw: &RawSearchResult, query: &SearchQuery, engine_weight: f32, engines: &[String]) -> f32 {
        let items = [(raw.clone(), engines.to_vec(), engine_weight)];
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

        let bm25_scores: Vec<f32> = items
            .iter()
            .map(|(raw, _, _)| bm25_score(raw, query))
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
                let doc = format!("{} {} {}", raw.title, raw.url, raw.snippet);
                doc.chars().take(1024).collect()
            })
            .collect();

        let doc_refs: Vec<&str> = documents.iter().map(|s| s.as_str()).collect();

        // Round-robin pick a reranker instance from the pool.
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.pool.len();
        let count = self.call_counts[idx].fetch_add(1, Ordering::Relaxed);
        let should_rebuild = count > 0 && count.is_multiple_of(REBUILD_EVERY);

        let mut scores = vec![0.0; items.len()];

        {
            let mut reranker = match self.pool[idx].lock() {
                Ok(r) => r,
                Err(_) => return scores,
            };

            if let Ok(results) = reranker.rerank(query.query.as_str(), doc_refs, false, None) {
                for (rank, r) in results.iter().enumerate() {
                    let orig_idx = top_indices[rank];
                    let (raw, _, _) = &items[orig_idx];
                    let coverage = query_coverage(&documents[rank], query);
                    let authority = domain_authority(&raw.url);
                    let raw_score = authority * authority * r.score * coverage;
                    scores[orig_idx] = raw_score.clamp(0.0, 1.0);
                }
            }

            if should_rebuild {
                if let Ok(new_reranker) = Self::build_reranker() {
                    *reranker = new_reranker;
                } else {
                    tracing::error!("reranker rebuild failed");
                }
            }
        }

        scores
    }
}

/// Fraction of unique query terms that appear in the document.
fn query_coverage(document: &str, query: &SearchQuery) -> f32 {
    let query_terms: std::collections::HashSet<String> = query
        .query
        .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();
    if query_terms.is_empty() {
        return 1.0;
    }
    let doc_lower = document.to_lowercase();
    let matched = query_terms
        .iter()
        .filter(|t| doc_lower.contains(t.as_str()))
        .count() as f32;
    matched / query_terms.len() as f32
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
