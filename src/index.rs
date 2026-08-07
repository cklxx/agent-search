//! Local full-text index for caching search results.
//!
//! Uses Tantivy to build an in-memory/disk index of search results,
//! allowing repeated queries to be served locally without hitting
//! upstream search engines.

use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Schema, STORED, TEXT, Value};
use tantivy::{doc, Index, IndexWriter, TantivyError};

use crate::models::result::SearchResult;

/// Local search index backed by Tantivy.
pub struct LocalIndex {
    index: Index,
    title_field: tantivy::schema::Field,
    url_field: tantivy::schema::Field,
    snippet_field: tantivy::schema::Field,
    content_field: tantivy::schema::Field,
}

impl LocalIndex {
    /// Create a new in-memory index.
    pub fn new_in_ram() -> Result<Self, TantivyError> {
        let mut schema_builder = Schema::builder();
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let url_field = schema_builder.add_text_field("url", TEXT | STORED);
        let snippet_field = schema_builder.add_text_field("snippet", TEXT | STORED);
        let content_field = schema_builder.add_text_field("content", TEXT | STORED);
        let schema = schema_builder.build();

        let index = Index::create_in_ram(schema);

        Ok(Self {
            index,
            title_field,
            url_field,
            snippet_field,
            content_field,
        })
    }

    /// Open or create a persistent index at the given directory.
    pub fn open_or_create(dir: &Path) -> Result<Self, TantivyError> {
        let mut schema_builder = Schema::builder();
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let url_field = schema_builder.add_text_field("url", TEXT | STORED);
        let snippet_field = schema_builder.add_text_field("snippet", TEXT | STORED);
        let content_field = schema_builder.add_text_field("content", TEXT | STORED);
        let schema = schema_builder.build();

        let index = if dir.exists() {
            Index::open_in_dir(dir)?
        } else {
            std::fs::create_dir_all(dir).map_err(|e| {
                TantivyError::InvalidArgument(format!("failed to create index dir: {}", e))
            })?;
            Index::create_in_dir(dir, schema)?
        };

        Ok(Self {
            index,
            title_field,
            url_field,
            snippet_field,
            content_field,
        })
    }

    /// Add a batch of search results to the index.
    pub fn add_results(&self, results: &[SearchResult]) -> Result<(), TantivyError> {
        let mut index_writer: IndexWriter = self.index.writer(50_000_000)?;

        for result in results {
            index_writer.add_document(doc!(
                self.title_field => result.title.clone(),
                self.url_field => result.url.clone(),
                self.snippet_field => result.snippet.clone(),
                self.content_field => result.content.clone().unwrap_or_default(),
            ))?;
        }

        index_writer.commit()?;
        Ok(())
    }

    /// Search the local index and return matching results.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, TantivyError> {
        let reader = self.index.reader()?;
        let searcher = reader.searcher();

        let query_parser = QueryParser::for_index(
            &self.index,
            vec![
                self.title_field,
                self.url_field,
                self.snippet_field,
                self.content_field,
            ],
        );
        let parsed_query = query_parser.parse_query(query)?;

        let top_docs = searcher.search(&parsed_query, &TopDocs::with_limit(limit))?;

        let mut results = Vec::new();
        for (score, doc_address) in top_docs {
            let retrieved_doc: tantivy::schema::TantivyDocument = searcher.doc(doc_address)?;
            let title = retrieved_doc
                .get_first(self.title_field)
                .and_then(|v: &tantivy::schema::OwnedValue| v.as_str())
                .unwrap_or("")
                .to_string();
            let url = retrieved_doc
                .get_first(self.url_field)
                .and_then(|v: &tantivy::schema::OwnedValue| v.as_str())
                .unwrap_or("")
                .to_string();
            let snippet = retrieved_doc
                .get_first(self.snippet_field)
                .and_then(|v: &tantivy::schema::OwnedValue| v.as_str())
                .unwrap_or("")
                .to_string();
            let content = retrieved_doc
                .get_first(self.content_field)
                .and_then(|v: &tantivy::schema::OwnedValue| v.as_str())
                .map(|s: &str| s.to_string());

            results.push(SearchResult {
                title,
                url,
                snippet,
                content,
                published_date: None,
                score,
                engine: "local".to_string(),
                engines: vec!["local".to_string()],
            });
        }

        Ok(results)
    }
}
