//! Pluggable ranking strategies for A/B comparison.

use std::sync::Mutex;

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
        // Programming / systems
        ("stackoverflow.com", 1.5),
        ("github.com", 1.4),
        ("developer.mozilla.org", 1.4),
        ("docs.rs", 1.4),
        ("doc.rust-lang.org", 1.4),
        ("python.org", 1.3),
        ("go.dev", 1.3),
        ("kubernetes.io", 1.4),
        ("docker.com", 1.3),
        ("nginx.org", 1.3),
        // General knowledge
        ("wikipedia.org", 1.3),
        // LLM / AI
        ("huggingface.co", 1.4),
        ("openai.com", 1.3),
    ];

    AUTHORITIES
        .iter()
        .filter(|(d, _)| domain == **d || domain.ends_with(&format!(".{}", d)))
        .map(|(_, v)| *v)
        .fold(1.0_f32, f32::max)
}

pub trait RankingStrategy: Send + Sync {
    fn name(&self) -> &str;
    fn score(&self, raw: &RawSearchResult, query: &SearchQuery, engine_weight: f32, engines: &[String]) -> f32;
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
    let text = format!("{} {} {}", raw.title, raw.url, raw.snippet).to_lowercase();
    // Tokenize on whitespace and punctuation boundaries.
    let tokens: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|t| !t.is_empty())
        .collect();
    let k1 = 1.2;

    let query_terms: Vec<&str> = query
        .query
        .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|t| !t.is_empty())
        .collect();
    if query_terms.is_empty() {
        return 0.0;
    }

    let mut score = 0.0;
    let mut matched_terms = 0;
    for term in &query_terms {
        let term_lower = term.to_lowercase();
        // Exact token match, not substring.
        let tf = tokens.iter().filter(|t| **t == term_lower).count() as f32;
        if tf > 0.0 {
            matched_terms += 1;
        }
        // BM25 term saturation. Raw tf, not tf/doc_len.
        score += tf * (k1 + 1.0) / (tf + k1);
    }

    // Query coverage: boost results that match more query terms.
    let coverage = matched_terms as f32 / query_terms.len() as f32;
    score * (0.5 + 0.5 * coverage)
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
        let text = format!("{} {} {}", raw.title, raw.url, raw.snippet).to_lowercase();
        let tokens: Vec<&str> = text
            .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
            .filter(|t| !t.is_empty())
            .collect();
        let doc_len = tokens.len().max(1) as f32;

        let query_terms: Vec<&str> = query
            .query
            .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
            .filter(|t| !t.is_empty())
            .collect();
        if query_terms.is_empty() {
            return 0.0;
        }

        let mut tfidf = 0.0;
        for term in &query_terms {
            let term_lower = term.to_lowercase();
            let tf = tokens.iter().filter(|t| **t == term_lower).count() as f32 / doc_len;
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
        1.0 / (1.0 + (-engine_weight).exp())
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

        let query_terms: Vec<&str> = query
            .query
            .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
            .filter(|t| !t.is_empty())
            .collect();
        let title_lower = raw.title.to_lowercase();
        let title_tokens: Vec<&str> = title_lower
            .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
            .filter(|t| !t.is_empty())
            .collect();
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

/// Cross-encoder reranking with bge-reranker-v2-m3 (2026 SOTA multilingual).
/// Computes query-document relevance via full cross-attention.
pub struct BgeRerankerStrategy {
    reranker: Mutex<TextRerank>,
}

impl BgeRerankerStrategy {
    pub fn new() -> anyhow::Result<Self> {
        let mut options = RerankInitOptions::default();
        options.model_name = RerankerModel::BGERerankerV2M3;
        let reranker = TextRerank::try_new(options)?;
        Ok(Self {
            reranker: Mutex::new(reranker),
        })
    }
}

impl RankingStrategy for BgeRerankerStrategy {
    fn name(&self) -> &str {
        "bge_reranker"
    }

    fn score(&self, raw: &RawSearchResult, query: &SearchQuery, _engine_weight: f32, _engines: &[String]) -> f32 {
        let document = format!("{} {}", raw.title, raw.snippet);
        let mut reranker = match self.reranker.lock() {
            Ok(r) => r,
            Err(_) => return 0.0,
        };
        match reranker.rerank(&query.query, vec![&document], false, None) {
            Ok(results) => results.first().map(|r| r.score).unwrap_or(0.0),
            Err(_) => 0.0,
        }
    }
}

pub fn get_strategy(name: &str) -> Option<Box<dyn RankingStrategy>> {
    match name {
        "bm25" => Some(Box::new(Bm25Strategy)),
        "tfidf" => Some(Box::new(TfIdfStrategy)),
        "position_only" => Some(Box::new(PositionOnlyStrategy)),
        "engine_weight" => Some(Box::new(EngineWeightStrategy)),
        "bm25_title_boost" => Some(Box::new(Bm25TitleBoostStrategy)),
        "searxng_only" => Some(Box::new(SearxngOnlyStrategy)),
        "bge_reranker" => match BgeRerankerStrategy::new() {
            Ok(s) => Some(Box::new(s)),
            Err(e) => {
                eprintln!("Failed to load bge-reranker-v2-m3: {}", e);
                None
            }
        },
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
