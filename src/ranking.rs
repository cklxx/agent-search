//! Relevance scoring for search results.

use crate::models::query::SearchQuery;
use crate::models::result::{RawSearchResult, SearchResult};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Schema, TEXT};
use tantivy::{doc, Index, IndexWriter};

/// Calculate a relevance score for a search result.
///
/// Combines:
/// - BM25 score on (title + url + snippet) via an in-memory Tantivy index
/// - Position weight (1 / position)
/// - Engine weight
pub fn calculate_score(raw: &RawSearchResult, query: &SearchQuery, engine_weight: f32) -> f32 {
    let bm25 = bm25_score(raw, query).unwrap_or(0.0);

    // Position weight: higher position (lower number) = higher score
    let position_weight = 1.0 / raw.position as f32;

    let score = bm25 * position_weight * engine_weight;

    // Normalize to 0.0 - 1.0 range using sigmoid
    1.0 / (1.0 + (-score).exp())
}

/// Compute the BM25 score for a single result against the query.
///
/// Builds an in-memory Tantivy index containing the single document
/// (title, url, snippet) and runs BM25 retrieval. Tantivy uses BM25
/// as its default similarity.
fn bm25_score(raw: &RawSearchResult, query: &SearchQuery) -> tantivy::Result<f32> {
    // Build schema with three searchable text fields.
    // We do not mark them STORED because we only need scoring, not retrieval.
    let mut schema_builder = Schema::builder();
    let title_field = schema_builder.add_text_field("title", TEXT);
    let url_field = schema_builder.add_text_field("url", TEXT);
    let snippet_field = schema_builder.add_text_field("snippet", TEXT);
    let schema = schema_builder.build();

    // Create an in-memory index
    let index = Index::create_in_ram(schema);

    // Index the single document
    let mut index_writer: IndexWriter = index.writer(50_000_000)?;
    index_writer.add_document(doc!(
        title_field => raw.title.clone(),
        url_field => raw.url.clone(),
        snippet_field => raw.snippet.clone(),
    ))?;
    index_writer.commit()?;

    // Acquire a searcher over the committed segment
    let reader = index.reader()?;
    let searcher = reader.searcher();

    // Parse the query against all three fields (title, url, snippet).
    // Terms are combined with OR (Should) semantics by default.
    let query_parser = QueryParser::for_index(&index, vec![title_field, url_field, snippet_field]);
    let parsed_query = query_parser.parse_query(&query.query)?;

    // Retrieve the top document — there is only one in the index.
    let top_docs = searcher.search(&parsed_query, &TopDocs::with_limit(1))?;

    Ok(top_docs.first().map(|(score, _)| *score).unwrap_or(0.0))
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
