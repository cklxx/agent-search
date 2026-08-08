//! Pluggable ranking strategies for A/B comparison.
//!
//! Each strategy implements [`RankingStrategy`] and can be selected at runtime.
//! The `/search/ab` endpoint runs two strategies side-by-side for comparison.

use crate::models::query::SearchQuery;
use crate::models::result::{RawSearchResult, SearchResult};

/// A ranking strategy that scores search results.
///
/// Implementations define how raw results from engines are converted into
/// scored, ranked results. Different strategies can be compared via A/B testing.
pub trait RankingStrategy: Send + Sync {
    /// Strategy name (used for A/B comparison output).
    fn name(&self) -> &str;

    /// Score a single raw result. Higher = more relevant.
    fn score(&self, raw: &RawSearchResult, query: &SearchQuery, engine_weight: f32) -> f32;

    /// Convert a raw result to a scored search result using this strategy.
    fn rank(&self, raw: RawSearchResult, query: &SearchQuery, engine_weight: f32) -> SearchResult {
        let score = self.score(&raw, query, engine_weight);
        SearchResult {
            title: raw.title,
            url: raw.url,
            snippet: raw.snippet,
            content: None,
            published_date: raw.published_date,
            score,
            engine: String::new(),
            engines: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Strategy: BM25 (Tantivy) + position + engine weight
// ---------------------------------------------------------------------------

/// BM25 scoring via an in-memory Tantivy index, combined with position and
/// engine weight. This is the default strategy.
pub struct Bm25Strategy;

impl RankingStrategy for Bm25Strategy {
    fn name(&self) -> &str {
        "bm25"
    }

    fn score(&self, raw: &RawSearchResult, query: &SearchQuery, engine_weight: f32) -> f32 {
        let bm25 = bm25_score(raw, query).unwrap_or(0.0);
        let position_weight = 1.0 / raw.position as f32;
        let score = bm25 * position_weight * engine_weight;
        1.0 / (1.0 + (-score).exp())
    }
}

fn bm25_score(raw: &RawSearchResult, query: &SearchQuery) -> tantivy::Result<f32> {
    use tantivy::collector::TopDocs;
    use tantivy::query::QueryParser;
    use tantivy::schema::{Schema, TEXT};
    use tantivy::{doc, Index, IndexWriter};

    let mut schema_builder = Schema::builder();
    let title_field = schema_builder.add_text_field("title", TEXT);
    let url_field = schema_builder.add_text_field("url", TEXT);
    let snippet_field = schema_builder.add_text_field("snippet", TEXT);
    let schema = schema_builder.build();

    let index = Index::create_in_ram(schema);
    let mut index_writer: IndexWriter = index.writer(50_000_000)?;
    index_writer.add_document(doc!(
        title_field => raw.title.clone(),
        url_field => raw.url.clone(),
        snippet_field => raw.snippet.clone(),
    ))?;
    index_writer.commit()?;

    let reader = index.reader()?;
    let searcher = reader.searcher();

    let query_parser =
        QueryParser::for_index(&index, vec![title_field, url_field, snippet_field]);
    let parsed_query = query_parser.parse_query(&query.query)?;

    let top_docs = searcher.search(&parsed_query, &TopDocs::with_limit(1))?;
    Ok(top_docs.first().map(|(score, _)| *score).unwrap_or(0.0))
}

// ---------------------------------------------------------------------------
// Strategy: TF-IDF (term frequency × inverse document frequency)
// ---------------------------------------------------------------------------

/// Simple TF-IDF scoring. IDF is approximated from term length since we lack
/// corpus statistics.
pub struct TfIdfStrategy;

impl RankingStrategy for TfIdfStrategy {
    fn name(&self) -> &str {
        "tfidf"
    }

    fn score(&self, raw: &RawSearchResult, query: &SearchQuery, engine_weight: f32) -> f32 {
        let query_terms: Vec<&str> = query.query.split_whitespace().collect();
        if query_terms.is_empty() {
            return 0.0;
        }

        let text = format!("{} {} {}", raw.title, raw.url, raw.snippet).to_lowercase();
        let doc_len = text.split_whitespace().count().max(1) as f32;

        let mut tfidf = 0.0;
        for term in &query_terms {
            let term_lower = term.to_lowercase();
            let tf = text.matches(&term_lower).count() as f32 / doc_len;
            // Approximate IDF: longer terms are rarer
            let idf = 1.0 + (term.len() as f32).ln();
            tfidf += tf * idf;
        }

        let position_weight = 1.0 / raw.position as f32;
        let score = tfidf * position_weight * engine_weight * 100.0;
        1.0 / (1.0 + (-score).exp())
    }
}

// ---------------------------------------------------------------------------
// Strategy: Position-only (rank purely by engine result position)
// ---------------------------------------------------------------------------

/// Scores results only by their position in the engine's result list.
/// Useful as a baseline to measure how much relevance scoring adds.
pub struct PositionOnlyStrategy;

impl RankingStrategy for PositionOnlyStrategy {
    fn name(&self) -> &str {
        "position_only"
    }

    fn score(&self, raw: &RawSearchResult, _query: &SearchQuery, engine_weight: f32) -> f32 {
        let position_weight = 1.0 / raw.position as f32;
        position_weight * engine_weight
    }
}

// ---------------------------------------------------------------------------
// Strategy: Engine-weight-only (rank purely by engine trust)
// ---------------------------------------------------------------------------

/// Scores results only by the weight of the engine that returned them.
/// Useful for testing engine weight configuration.
pub struct EngineWeightStrategy;

impl RankingStrategy for EngineWeightStrategy {
    fn name(&self) -> &str {
        "engine_weight"
    }

    fn score(&self, _raw: &RawSearchResult, _query: &SearchQuery, engine_weight: f32) -> f32 {
        engine_weight
    }
}

// ---------------------------------------------------------------------------
// Strategy: BM25 with title boost (BM25 + extra weight for title matches)
// ---------------------------------------------------------------------------

/// BM25 scoring with an explicit boost when query terms appear in the title.
pub struct Bm25TitleBoostStrategy;

impl RankingStrategy for Bm25TitleBoostStrategy {
    fn name(&self) -> &str {
        "bm25_title_boost"
    }

    fn score(&self, raw: &RawSearchResult, query: &SearchQuery, engine_weight: f32) -> f32 {
        let bm25 = bm25_score(raw, query).unwrap_or(0.0);

        let query_terms: Vec<&str> = query.query.split_whitespace().collect();
        let title_lower = raw.title.to_lowercase();
        let title_match = query_terms
            .iter()
            .filter(|t| title_lower.contains(&t.to_lowercase()))
            .count() as f32
            / query_terms.len() as f32;

        let position_weight = 1.0 / raw.position as f32;
        let score = (bm25 + title_match * 2.0) * position_weight * engine_weight;
        1.0 / (1.0 + (-score).exp())
    }
}

// ---------------------------------------------------------------------------
// Strategy registry
// ---------------------------------------------------------------------------

/// Get a ranking strategy by name.
pub fn get_strategy(name: &str) -> Box<dyn RankingStrategy> {
    match name {
        "bm25" => Box::new(Bm25Strategy),
        "tfidf" => Box::new(TfIdfStrategy),
        "position_only" => Box::new(PositionOnlyStrategy),
        "engine_weight" => Box::new(EngineWeightStrategy),
        "bm25_title_boost" => Box::new(Bm25TitleBoostStrategy),
        _ => Box::new(Bm25Strategy), // default
    }
}

/// List all available strategy names.
pub fn strategy_names() -> Vec<&'static str> {
    vec![
        "bm25",
        "tfidf",
        "position_only",
        "engine_weight",
        "bm25_title_boost",
    ]
}
