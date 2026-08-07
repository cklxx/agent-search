//! Local full-text index for caching popular search queries.
//!
//! Uses Tantivy with BM25 scoring to build an in-memory/disk index of
//! search results. Repeated or similar queries are served locally
//! without hitting upstream search engines.

use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, STORED, TEXT, Value};
use tantivy::{doc, Index, IndexWriter, TantivyError};

use crate::models::result::SearchResult;

/// Default number of results returned by [`LocalIndex::search_cached`].
const DEFAULT_SEARCH_LIMIT: usize = 20;

/// Local search index backed by Tantivy (BM25 scoring).
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
    /// Create a new in-memory index.
    pub fn new_in_ram() -> Result<Self, TantivyError> {
        let (schema, fields) = build_schema();
        let index = Index::create_in_ram(schema);
        Ok(Self::from_parts(index, fields))
    }

    /// Open or create a persistent index at the given directory.
    pub fn open_or_create(dir: &Path) -> Result<Self, TantivyError> {
        let (schema, fields) = build_schema();
        let index = if dir.exists() {
            Index::open_in_dir(dir)?
        } else {
            std::fs::create_dir_all(dir).map_err(|e| {
                TantivyError::InvalidArgument(format!("failed to create index dir: {}", e))
            })?;
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

    /// Cache a batch of search results for the given query.
    ///
    /// Each result is indexed alongside the originating query string so
    /// that subsequent similar queries can retrieve them via BM25 search.
    pub fn cache_results(&self, query: &str, results: &[SearchResult]) -> Result<(), TantivyError> {
        let mut index_writer: IndexWriter = self.index.writer(50_000_000)?;

        for result in results {
            index_writer.add_document(doc!(
                self.query_field => query.to_string(),
                self.title_field => result.title.clone(),
                self.url_field => result.url.clone(),
                self.snippet_field => result.snippet.clone(),
                self.score_field => result.score as f64,
                self.engine_field => result.engine.clone(),
            ))?;
        }

        index_writer.commit()?;
        Ok(())
    }

    /// Search the cached index for results matching `query`.
    ///
    /// Uses BM25 scoring across the `query`, `title`, `snippet`, and `url`
    /// fields. Returns `None` if the search fails or no results match.
    pub fn search_cached(&self, query: &str) -> Option<Vec<SearchResult>> {
        let reader = self.index.reader().ok()?;
        let searcher = reader.searcher();

        let query_parser = QueryParser::for_index(
            &self.index,
            vec![
                self.query_field,
                self.title_field,
                self.snippet_field,
                self.url_field,
            ],
        );
        let parsed_query = query_parser.parse_query(query).ok()?;

        let top_docs = searcher
            .search(&parsed_query, &TopDocs::with_limit(DEFAULT_SEARCH_LIMIT))
            .ok()?;

        let mut results = Vec::with_capacity(top_docs.len());
        for (_score, doc_address) in top_docs {
            let retrieved_doc = searcher.doc(doc_address).ok()?;
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
                content: None,
                published_date: None,
                score,
                engine: engine.clone(),
                engines: vec![engine],
            });
        }

        if results.is_empty() {
            None
        } else {
            Some(results)
        }
    }
}

/// Schema field handles returned from [`build_schema`].
struct IndexFields {
    query: Field,
    title: Field,
    url: Field,
    snippet: Field,
    score: Field,
    engine: Field,
}

/// Build the index schema and return it together with the field handles.
fn build_schema() -> (Schema, IndexFields) {
    let mut schema_builder = Schema::builder();
    let query = schema_builder.add_text_field("query", TEXT | STORED);
    let title = schema_builder.add_text_field("title", TEXT | STORED);
    let url = schema_builder.add_text_field("url", TEXT | STORED);
    let snippet = schema_builder.add_text_field("snippet", TEXT | STORED);
    let score = schema_builder.add_f64_field("score", STORED);
    let engine = schema_builder.add_text_field("engine", TEXT | STORED);
    let schema = schema_builder.build();
    (
        schema,
        IndexFields {
            query,
            title,
            url,
            snippet,
            score,
            engine,
        },
    )
}

/// Extract the first stored string value of `field` from `doc`, defaulting to "".
fn field_str(doc: &tantivy::schema::TantivyDocument, field: Field) -> String {
    doc.get_first(field)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}
