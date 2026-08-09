//! Local Tantivy index for caching search results.

use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, STORED, TEXT, Value};
use tantivy::{doc, Index, IndexWriter, TantivyError};

use crate::models::result::SearchResult;

const DEFAULT_SEARCH_LIMIT: usize = 20;

pub struct LocalIndex {
    index: Index,
    query_field: Field,
    title_field: Field,
    url_field: Field,
    snippet_field: Field,
    score_field: Field,
    engine_field: Field,
}

impl LocalIndex {
    pub fn new_in_ram() -> Result<Self, TantivyError> {
        let (schema, fields) = build_schema();
        let index = Index::create_in_ram(schema);
        Ok(Self::from_parts(index, fields))
    }

    pub fn open_or_create(dir: &Path) -> Result<Self, TantivyError> {
        let (schema, fields) = build_schema();
        let index = if dir.exists() {
            Index::open_in_dir(dir)?
        } else {
            std::fs::create_dir_all(dir)
                .map_err(|e| TantivyError::InvalidArgument(format!("failed to create index dir: {}", e)))?;
            Index::create_in_dir(dir, schema)?
        };
        Ok(Self::from_parts(index, fields))
    }

    fn from_parts(index: Index, fields: IndexFields) -> Self {
        Self {
            index,
            query_field: fields.query,
            title_field: fields.title,
            url_field: fields.url,
            snippet_field: fields.snippet,
            score_field: fields.score,
            engine_field: fields.engine,
        }
    }

    /// Index results alongside the query that produced them.
    pub fn cache_results(&self, query: &str, results: &[SearchResult]) -> Result<(), TantivyError> {
        let mut index_writer: IndexWriter = self.index.writer(50_000_000)?;

        for result in results {
            let engine = result.engines.first().cloned().unwrap_or_default();
            index_writer.add_document(doc!(
                self.query_field => query.to_string(),
                self.title_field => result.title.clone(),
                self.url_field => result.url.clone(),
                self.snippet_field => result.snippet.clone(),
                self.score_field => result.score as f64,
                self.engine_field => engine,
            ))?;
        }

        index_writer.commit()?;
        Ok(())
    }

    /// BM25 search across the query field for exact cached queries,
    /// falling back to title/snippet/url full-text search for similar queries.
    /// Returns cached results for queries textually similar to `query`.
    pub fn search_cached(&self, query: &str) -> Option<Vec<SearchResult>> {
        let reader = self.index.reader().ok()?;
        let searcher = reader.searcher();

        // First, try exact match on the query field (cached results).
        let query_parser = QueryParser::for_index(&self.index, vec![self.query_field]);
        if let Ok(parsed_query) = query_parser.parse_query(query) {
            if let Ok(top_docs) = searcher.search(&parsed_query, &TopDocs::with_limit(DEFAULT_SEARCH_LIMIT)) {
                if !top_docs.is_empty() {
                    return Some(self.collect_results(&searcher, top_docs));
                }
            }
        }

        // Fallback: full-text search across title, snippet, and url fields.
        // This acts as a local search index when upstream engines are unavailable.
        let text_parser = QueryParser::for_index(
            &self.index,
            vec![self.title_field, self.snippet_field, self.url_field],
        );
        if let Ok(parsed_query) = text_parser.parse_query(query) {
            if let Ok(top_docs) = searcher.search(&parsed_query, &TopDocs::with_limit(DEFAULT_SEARCH_LIMIT)) {
                if !top_docs.is_empty() {
                    return Some(self.collect_results(&searcher, top_docs));
                }
            }
        }

        None
    }

    fn collect_results(
        &self,
        searcher: &tantivy::Searcher,
        top_docs: Vec<(f32, tantivy::DocAddress)>,
    ) -> Vec<SearchResult> {
        let mut results = Vec::with_capacity(top_docs.len());
        for (_score, doc_address) in top_docs {
            let retrieved_doc = match searcher.doc(doc_address) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let title = field_str(&retrieved_doc, self.title_field);
            let url = field_str(&retrieved_doc, self.url_field);
            let snippet = field_str(&retrieved_doc, self.snippet_field);
            let engine = field_str(&retrieved_doc, self.engine_field);
            let score = retrieved_doc
                .get_first(self.score_field)
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as f32;

            results.push(SearchResult {
                title,
                url,
                snippet,
                published_date: None,
                score,
                engines: vec![engine],
            });
        }
        results
    }
}

struct IndexFields {
    query: Field,
    title: Field,
    url: Field,
    snippet: Field,
    score: Field,
    engine: Field,
}

fn build_schema() -> (Schema, IndexFields) {
    let mut schema_builder = Schema::builder();
    let query = schema_builder.add_text_field("query", TEXT | STORED);
    let title = schema_builder.add_text_field("title", TEXT | STORED);
    let url = schema_builder.add_text_field("url", TEXT | STORED);
    let snippet = schema_builder.add_text_field("snippet", TEXT | STORED);
    let score = schema_builder.add_f64_field("score", STORED);
    let engine = schema_builder.add_text_field("engine", TEXT | STORED);
    let schema = schema_builder.build();
    (schema, IndexFields { query, title, url, snippet, score, engine })
}

fn field_str(doc: &tantivy::schema::TantivyDocument, field: Field) -> String {
    doc.get_first(field).and_then(|v| v.as_str()).unwrap_or("").to_string()
}
