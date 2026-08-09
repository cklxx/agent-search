//! Local Tantivy index.
//!
//! Two search modes:
//! - `search_cached`: exact query-string match (cache for repeated queries).
//! - `search_fulltext`: BM25 over title + content (self-built index from crawled pages).

use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::{QueryParser, TermQuery};
use tantivy::schema::{Field, Schema, STORED, STRING, TEXT, Value};
use tantivy::{doc, Index, IndexWriter, Term, TantivyError};

use crate::models::result::SearchResult;

const DEFAULT_SEARCH_LIMIT: usize = 20;

pub struct LocalIndex {
    index: Index,
    query_field: Field,
    title_field: Field,
    url_field: Field,
    snippet_field: Field,
    content_field: Field,
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
            content_field: fields.content,
            score_field: fields.score,
            engine_field: fields.engine,
        }
    }

    /// Cache results alongside the query that produced them.
    pub fn cache_results(&self, query: &str, results: &[SearchResult]) -> Result<(), TantivyError> {
        let mut index_writer: IndexWriter = self.index.writer(50_000_000)?;

        for result in results {
            let engine = result.engines.first().cloned().unwrap_or_default();
            index_writer.add_document(doc!(
                self.query_field => query.to_string(),
                self.title_field => result.title.clone(),
                self.url_field => result.url.clone(),
                self.snippet_field => result.snippet.clone(),
                self.content_field => result.snippet.clone(),
                self.score_field => result.score as f64,
                self.engine_field => engine,
            ))?;
        }

        index_writer.commit()?;
        Ok(())
    }

    /// Index a crawled page (title + full content) for full-text search.
    pub fn index_page(&self, url: &str, title: &str, content: &str) -> Result<(), TantivyError> {
        let mut index_writer: IndexWriter = self.index.writer(50_000_000)?;
        index_writer.add_document(doc!(
            self.query_field => String::new(),
            self.title_field => title.to_string(),
            self.url_field => url.to_string(),
            self.snippet_field => content.chars().take(300).collect::<String>(),
            self.content_field => content.to_string(),
            self.score_field => 0.0,
            self.engine_field => "local".to_string(),
        ))?;
        index_writer.commit()?;
        Ok(())
    }

    /// Returns cached results for the exact query. The query field is STRING
    /// (untokenized), so a TermQuery does exact string matching.
    pub fn search_cached(&self, query: &str) -> Option<Vec<SearchResult>> {
        let reader = self.index.reader().ok()?;
        let searcher = reader.searcher();

        let term = Term::from_field_text(self.query_field, query);
        let term_query = TermQuery::new(term, tantivy::schema::IndexRecordOption::Basic);
        let top_docs = searcher
            .search(&term_query, &TopDocs::with_limit(DEFAULT_SEARCH_LIMIT))
            .ok()?;

        if top_docs.is_empty() {
            return None;
        }

        Some(self.collect_results(&searcher, top_docs))
    }

    /// Full-text BM25 search over title + content fields.
    pub fn search_fulltext(&self, query: &str, limit: usize) -> Option<Vec<SearchResult>> {
        let reader = self.index.reader().ok()?;
        let searcher = reader.searcher();

        let query_parser = QueryParser::for_index(
            &self.index,
            vec![self.title_field, self.content_field],
        );
        let parsed_query = query_parser.parse_query(query).ok()?;

        let top_docs = searcher
            .search(&parsed_query, &TopDocs::with_limit(limit))
            .ok()?;

        if top_docs.is_empty() {
            return None;
        }

        Some(self.collect_results(&searcher, top_docs))
    }

    fn collect_results(
        &self,
        searcher: &tantivy::Searcher,
        top_docs: Vec<(f32, tantivy::DocAddress)>,
    ) -> Vec<SearchResult> {
        let mut results = Vec::with_capacity(top_docs.len());
        for (score, doc_address) in top_docs {
            let retrieved_doc = match searcher.doc(doc_address) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let title = field_str(&retrieved_doc, self.title_field);
            let url = field_str(&retrieved_doc, self.url_field);
            let snippet = field_str(&retrieved_doc, self.snippet_field);
            let engine = field_str(&retrieved_doc, self.engine_field);

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
    content: Field,
    score: Field,
    engine: Field,
}

fn build_schema() -> (Schema, IndexFields) {
    let mut schema_builder = Schema::builder();
    // STRING (not tokenized) so exact query matching works.
    let query = schema_builder.add_text_field("query", STRING | STORED);
    let title = schema_builder.add_text_field("title", TEXT | STORED);
    let url = schema_builder.add_text_field("url", TEXT | STORED);
    let snippet = schema_builder.add_text_field("snippet", TEXT | STORED);
    let content = schema_builder.add_text_field("content", TEXT | STORED);
    let score = schema_builder.add_f64_field("score", STORED);
    let engine = schema_builder.add_text_field("engine", TEXT | STORED);
    let schema = schema_builder.build();
    (schema, IndexFields { query, title, url, snippet, content, score, engine })
}

fn field_str(doc: &tantivy::schema::TantivyDocument, field: Field) -> String {
    doc.get_first(field).and_then(|v| v.as_str()).unwrap_or("").to_string()
}
