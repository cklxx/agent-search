//! Local Tantivy index.
//!
//! Two search modes:
//! - `search_cached`: exact query-string match (cache for repeated queries).
//! - `search_fulltext`: BM25 over title + content (self-built index from crawled pages).

use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::{QueryParser, TermQuery};
use tantivy::schema::{Field, Schema, STORED, STRING, TEXT, TextOptions, TextFieldIndexing, IndexRecordOption, Value};
use tantivy::tokenizer::{TextAnalyzer, Token, TokenStream, Tokenizer};
use tantivy::{doc, Index, IndexWriter, Term, TantivyError};

use crate::dedup::{DedupService, normalize_url};
use crate::models::result::SearchResult;

const DEFAULT_SEARCH_LIMIT: usize = 20;

/// True if `c` is a CJK ideograph or punctuation that needs n-gram tokenization.
fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{4e00}'..='\u{9fff}'      // CJK Unified Ideographs
        | '\u{3400}'..='\u{4dbf}'    // CJK Extension A
        | '\u{20000}'..='\u{2a6df}'  // CJK Extension B
        | '\u{2a700}'..='\u{2b73f}'  // CJK Extension C/D
        | '\u{3000}'..='\u{303f}'    // CJK Symbols and Punctuation
        | '\u{ff00}'..='\u{ffef}'    // Halfwidth and Fullwidth Forms
        | '\u{3040}'..='\u{30ff}'    // Hiragana + Katakana
        | '\u{ac00}'..='\u{d7af}'    // Hangul Syllables
    )
}

/// Mixed tokenizer: ASCII words are kept whole; CJK runs are split into
/// 2-character grams. This avoids the 2-gram explosion that makes English
/// queries match almost any document (common bigrams like "re", "he", "on").
#[derive(Clone)]
struct MixedCjkTokenizer;

impl Tokenizer for MixedCjkTokenizer {
    type TokenStream<'a> = MixedCjkTokenStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        MixedCjkTokenStream {
            text,
            tokens: Vec::new(),
            index: 0,
            built: false,
        }
    }
}

struct MixedCjkTokenStream<'a> {
    text: &'a str,
    tokens: Vec<Token>,
    index: usize,
    built: bool,
}

impl<'a> MixedCjkTokenStream<'a> {
    fn build(&mut self) {
        let bytes = self.text.as_bytes();
        let mut position = 0;
        let mut char_iter = self.text.char_indices().peekable();

        while let Some(&(byte_start, c)) = char_iter.peek() {
            if c.is_ascii_alphanumeric() {
                // ASCII word: consume consecutive alphanumeric chars.
                let start = byte_start;
                while let Some(&(_, ch)) = char_iter.peek() {
                    if ch.is_ascii_alphanumeric() {
                        char_iter.next();
                    } else {
                        break;
                    }
                }
                let end = match char_iter.peek() {
                    Some(&(b, _)) => b,
                    None => bytes.len(),
                };
                let word = &self.text[start..end];
                self.tokens.push(Token {
                    offset_from: start,
                    offset_to: end,
                    position,
                    text: word.to_lowercase(),
                    position_length: 1,
                });
                position += 1;
            } else if is_cjk(c) {
                // CJK run: collect consecutive CJK chars, emit 2-grams.
                let mut run: Vec<(usize, char)> = Vec::new();
                while let Some(&(_, ch)) = char_iter.peek() {
                    if is_cjk(ch) {
                        run.push(char_iter.next().unwrap());
                    } else {
                        break;
                    }
                }
                if run.len() == 1 {
                    let (idx, ch) = run[0];
                    let end = idx + ch.len_utf8();
                    self.tokens.push(Token {
                        offset_from: idx,
                        offset_to: end,
                        position,
                        text: ch.to_string(),
                        position_length: 1,
                    });
                    position += 1;
                } else {
                    for w in run.windows(2) {
                        let (start_idx, _) = w[0];
                        let (end_idx, end_ch) = w[1];
                        let end = end_idx + end_ch.len_utf8();
                        let gram: String = w.iter().map(|(_, c)| c).collect();
                        self.tokens.push(Token {
                            offset_from: start_idx,
                            offset_to: end,
                            position,
                            text: gram,
                            position_length: 1,
                        });
                        position += 1;
                    }
                }
            } else {
                // Whitespace / punctuation: skip.
                char_iter.next();
            }
        }
        self.built = true;
    }
}

impl<'a> TokenStream for MixedCjkTokenStream<'a> {
    fn advance(&mut self) -> bool {
        if !self.built {
            self.build();
        }
        if self.index < self.tokens.len() {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn token(&self) -> &Token {
        &self.tokens[self.index - 1]
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.tokens[self.index - 1]
    }
}

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
        // Mixed tokenizer: ASCII words kept whole, CJK runs split into 2-grams.
        let tokenizer = TextAnalyzer::from(MixedCjkTokenizer);
        index.tokenizers().register("cjk_ngram", tokenizer);

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
    /// Skips URLs already indexed (tracked by the shared DedupService).
    pub fn index_page(&self, url: &str, title: &str, content: &str, dedup: &DedupService) -> Result<(), TantivyError> {
        if !dedup.insert(url) {
            return Ok(());
        }

        let mut index_writer: IndexWriter = self.index.writer(50_000_000)?;
        index_writer.add_document(doc!(
            self.query_field => String::new(),
            self.title_field => title.to_string(),
            self.url_field => url.to_string(),
            self.snippet_field => content.chars().take(1000).collect::<String>(),
            self.content_field => content.to_string(),
            self.score_field => 0.0,
            self.engine_field => "local".to_string(),
        ))?;
        index_writer.commit()?;
        Ok(())
    }

    /// Bulk-index multiple pages in a single commit. Much faster than calling
    /// `index_page` in a loop (one commit per call).
    pub fn bulk_index(
        &self,
        pages: &[(String, String, String)],
        dedup: &DedupService,
    ) -> Result<usize, TantivyError> {
        let mut index_writer: IndexWriter = self.index.writer(50_000_000)?;
        let mut count = 0;
        for (url, title, content) in pages {
            if !dedup.insert(url) {
                continue;
            }
            index_writer.add_document(doc!(
                self.query_field => String::new(),
                self.title_field => title.clone(),
                self.url_field => url.clone(),
                self.snippet_field => content.chars().take(1000).collect::<String>(),
                self.content_field => content.clone(),
                self.score_field => 0.0,
                self.engine_field => "local".to_string(),
            ))?;
            count += 1;
        }
        index_writer.commit()?;
        Ok(count)
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

        Some(self.collect_results(&searcher, top_docs, true))
    }

    /// Full-text BM25 search over title + content fields.
    ///
    /// Short queries (≤3 terms) use AND semantics for precision; longer
    /// queries use OR semantics for recall. The cross-encoder reranks
    /// the top results, so recall matters more for long queries.
    ///
    /// Query expansion: each English word is stemmed and the stem is
    /// appended to the query, so "borrowing" matches "borrow" in the index.
    pub fn search_fulltext(&self, query: &str, limit: usize) -> Option<Vec<SearchResult>> {
        let reader = self.index.reader().ok()?;
        let searcher = reader.searcher();

        let mut query_parser = QueryParser::for_index(
            &self.index,
            vec![self.title_field, self.content_field],
        );

        // Expand query with stemmed forms for better recall.
        let expanded = expand_query(query);

        // Short queries: require all terms to match (AND).
        // Long queries: any term can match (OR) for broader recall.
        if count_query_terms(&expanded) <= 3 {
            query_parser.set_conjunction_by_default();
        }

        let parsed_query = query_parser.parse_query(&expanded).ok()?;

        let top_docs = searcher
            .search(&parsed_query, &TopDocs::with_limit(limit))
            .ok()?;

        if top_docs.is_empty() {
            return None;
        }

        Some(self.collect_results(&searcher, top_docs, false))
    }

    fn collect_results(
        &self,
        searcher: &tantivy::Searcher,
        top_docs: Vec<(f32, tantivy::DocAddress)>,
        use_stored_score: bool,
    ) -> Vec<SearchResult> {
        let mut seen = std::collections::HashSet::new();
        let mut results = Vec::with_capacity(top_docs.len());
        for (bm25_score, doc_address) in top_docs {
            let retrieved_doc = match searcher.doc(doc_address) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let title = field_str(&retrieved_doc, self.title_field);
            let url = field_str(&retrieved_doc, self.url_field);
            // Use the first 1000 chars of content as the snippet. For crawled
            // pages this is full page text; for cached results it's the engine
            // snippet. Longer snippets improve keyword-overlap relevance
            // scoring without changing the stored index.
            let content = field_str(&retrieved_doc, self.content_field);
            let snippet: String = content.chars().take(1000).collect();
            let engine = field_str(&retrieved_doc, self.engine_field);

            // Dedup by normalized URL: the same page may be indexed under
            // multiple queries or with different URL encodings.
            let normalized = normalize_url(&url);
            if !seen.insert(normalized) {
                continue;
            }

            // For cached exact-query results, use the stored ranking score.
            // For full-text results, use the BM25 score from Tantivy.
            let score = if use_stored_score {
                retrieved_doc
                    .get_first(self.score_field)
                    .and_then(|v| v.as_f64())
                    .unwrap_or(bm25_score as f64) as f32
            } else {
                bm25_score
            };

            results.push(SearchResult {
                title,
                url,
                snippet,
                published_date: None,
                score,
                engines: vec![engine],
                weight: 1.0,
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

    // Title and content use a 2-gram tokenizer for CJK support.
    let cjk_indexing = TextFieldIndexing::default()
        .set_tokenizer("cjk_ngram")
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let cjk_options = TextOptions::default()
        .set_indexing_options(cjk_indexing)
        .set_stored();

    let title = schema_builder.add_text_field("title", cjk_options.clone());
    let url = schema_builder.add_text_field("url", TEXT | STORED);
    let snippet = schema_builder.add_text_field("snippet", cjk_options.clone());
    let content = schema_builder.add_text_field("content", cjk_options);
    let score = schema_builder.add_f64_field("score", STORED);
    let engine = schema_builder.add_text_field("engine", TEXT | STORED);
    let schema = schema_builder.build();
    (schema, IndexFields { query, title, url, snippet, content, score, engine })
}

fn field_str(doc: &tantivy::schema::TantivyDocument, field: Field) -> String {
    doc.get_first(field).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// Count the number of searchable terms in a query string.
///
/// ASCII alphanumeric runs count as one term each; CJK characters count
/// as one term each. This matches the MixedCjkTokenizer behavior.
fn count_query_terms(query: &str) -> usize {
    let mut count = 0;
    let mut in_ascii_word = false;
    for c in query.chars() {
        if c.is_ascii_alphanumeric() {
            if !in_ascii_word {
                count += 1;
                in_ascii_word = true;
            }
        } else if is_cjk(c) {
            count += 1;
            in_ascii_word = false;
        } else {
            in_ascii_word = false;
        }
    }
    count
}

/// Expand a query by appending stemmed forms of English words.
///
/// For each ASCII word, a simplified stem is computed and appended.
/// This improves recall: "borrowing" also matches "borrow" in the index.
/// CJK characters are passed through unchanged.
fn expand_query(query: &str) -> String {
    let mut result = String::with_capacity(query.len() * 2);
    let mut current_word = String::new();

    for c in query.chars() {
        if c.is_ascii_alphanumeric() {
            current_word.push(c);
        } else {
            if !current_word.is_empty() {
                result.push_str(&current_word);
                if let Some(stem) = stem_word(&current_word) {
                    if stem != current_word.to_lowercase() {
                        result.push(' ');
                        result.push_str(&stem);
                    }
                }
                current_word.clear();
            }
            result.push(c);
        }
    }

    if !current_word.is_empty() {
        result.push_str(&current_word);
        if let Some(stem) = stem_word(&current_word) {
            if stem != current_word.to_lowercase() {
                result.push(' ');
                result.push_str(&stem);
            }
        }
    }

    result
}

/// Simplified English stemmer. Returns the stemmed form if it differs
/// from the lowercase original. Handles common suffixes: -ing, -ed,
/// -es, -s, -ies, -er, -est, -ly.
fn stem_word(word: &str) -> Option<String> {
    let lower = word.to_lowercase();
    if lower.len() <= 3 {
        return None;
    }

    let stem = if let Some(s) = lower.strip_suffix("ies") {
        if s.len() > 1 {
            format!("{s}y")
        } else {
            return None;
        }
    } else if let Some(s) = lower.strip_suffix("es") {
        if s.len() > 1 {
            s.to_string()
        } else {
            return None;
        }
    } else if let Some(s) = lower.strip_suffix("ing") {
        if s.len() > 1 {
            // running → run (drop double consonant), making → make
            if s.ends_with(|c: char| !"aeiou".contains(c))
                && s.len() > 1
                && s.chars().last() == s.chars().nth_back(1)
            {
                s[..s.len() - 1].to_string()
            } else {
                s.to_string()
            }
        } else {
            return None;
        }
    } else if let Some(s) = lower.strip_suffix("ed") {
        if let Some(stem) = s.strip_suffix('i') {
            format!("{stem}y")
        } else {
            s.to_string()
        }
    } else if let Some(s) = lower.strip_suffix("er") {
        s.to_string()
    } else if let Some(s) = lower.strip_suffix("est") {
        s.to_string()
    } else if let Some(s) = lower.strip_suffix("ly") {
        s.to_string()
    } else if let Some(s) = lower.strip_suffix('s') {
        if !lower.ends_with("ss") && s.len() > 1 {
            s.to_string()
        } else {
            return None;
        }
    } else {
        return None;
    };

    if stem.len() >= 2 && stem != lower {
        Some(stem)
    } else {
        None
    }
}
