//! Relevance scoring for search results.

use crate::models::query::SearchQuery;
use crate::models::result::{RawSearchResult, SearchResult};

/// Calculate a relevance score for a search result.
///
/// Combines:
/// - BM25-like term frequency score on (title + url + snippet)
/// - Position weight (1 / position)
/// - Engine weight
pub fn calculate_score(raw: &RawSearchResult, query: &SearchQuery, engine_weight: f32) -> f32 {
    let query_terms: Vec<&str> = query.query.split_whitespace().collect();
    if query_terms.is_empty() {
        return 0.0;
    }

    let text = format!("{} {} {}", raw.title, raw.url, raw.snippet).to_lowercase();

    // Simple TF-based score: sum of term frequencies normalized by text length
    let mut tf_score = 0.0;
    for term in &query_terms {
        let term_lower = term.to_lowercase();
        let freq = text.matches(&term_lower).count() as f32;
        tf_score += freq;
    }
    tf_score /= text.len().max(1) as f32;
    tf_score *= 1000.0; // scale up

    // Position weight: higher position (lower number) = higher score
    let position_weight = 1.0 / raw.position as f32;

    // Title bonus: if query terms appear in title, boost
    let title_lower = raw.title.to_lowercase();
    let title_match = query_terms
        .iter()
        .filter(|t| title_lower.contains(&t.to_lowercase()))
        .count() as f32
        / query_terms.len() as f32;

    let score = (tf_score + title_match * 0.5) * position_weight * engine_weight;

    // Normalize to 0.0 - 1.0 range using sigmoid
    1.0 / (1.0 + (-score).exp())
}

/// Convert a raw result to a scored search result.
pub fn score_result(raw: RawSearchResult, query: &SearchQuery, engine_weight: f32) -> SearchResult {
    let score = calculate_score(&raw, query, engine_weight);
    SearchResult {
        title: raw.title,
        url: raw.url,
        snippet: raw.snippet,
        content: None,
        published_date: raw.published_date,
        score,
        engine: String::new(), // set by aggregator
        engines: Vec::new(),   // set by aggregator
    }
}
