//! Configurable engine that executes searches based on YAML config.
//!
//! Supports both HTML (CSS selectors) and JSON API engines.

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;
use scraper::{Html, Selector};

use crate::engine::config::{EngineConfig, EngineType};
use crate::engine::trait_def::SearchEngine;
use crate::models::error::{EngineResult, SearchError};
use crate::models::query::SearchQuery;
use crate::models::result::RawSearchResult;

/// A search engine driven by declarative configuration.
pub struct ConfigurableEngine {
    config: EngineConfig,
    client: Client,
}

impl ConfigurableEngine {
    pub fn new(config: EngineConfig) -> Self {
        let mut headers = HeaderMap::new();
        for (key, value) in &config.headers {
            if let (Ok(k), Ok(v)) = (
                reqwest::header::HeaderName::from_bytes(key.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                headers.insert(k, v);
            }
        }

        let client = Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .default_headers(headers)
            .build()
            .expect("failed to build HTTP client");

        Self { config, client }
    }

    /// Parse HTML response using CSS selectors.
    fn parse_html(&self, body: &str) -> EngineResult<Vec<RawSearchResult>> {
        let selectors = self.config.html.as_ref().ok_or_else(|| {
            SearchError::Parse("html selectors not configured".to_string())
        })?;

        let doc = Html::parse_document(body);
        let results_sel = Selector::parse(&selectors.results)
            .map_err(|e| SearchError::Parse(e.to_string()))?;
        let url_sel = Selector::parse(&selectors.url)
            .map_err(|e| SearchError::Parse(e.to_string()))?;
        let title_sel = Selector::parse(&selectors.title)
            .map_err(|e| SearchError::Parse(e.to_string()))?;
        let content_sel = selectors
            .content
            .as_ref()
            .map(|s| Selector::parse(s))
            .transpose()
            .map_err(|e| SearchError::Parse(e.to_string()))?;

        let mut results = Vec::new();
        for (idx, item) in doc.select(&results_sel).enumerate() {
            let title = item
                .select(&title_sel)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            let url = item
                .select(&url_sel)
                .next()
                .and_then(|e| e.value().attr("href"))
                .unwrap_or("")
                .to_string();

            let snippet = content_sel
                .as_ref()
                .and_then(|sel| item.select(sel).next())
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            if title.is_empty() || url.is_empty() {
                continue;
            }

            results.push(RawSearchResult {
                title,
                url,
                snippet,
                published_date: None,
                position: (idx + 1) as u32,
            });
        }

        Ok(results)
    }

    /// Parse JSON response using field mappings.
    fn parse_json(&self, body: &str) -> EngineResult<Vec<RawSearchResult>> {
        let queries = self.config.json.as_ref().ok_or_else(|| {
            SearchError::Parse("json queries not configured".to_string())
        })?;

        let data: serde_json::Value =
            serde_json::from_str(body).map_err(|e| SearchError::Parse(e.to_string()))?;

        let results_array = get_json_path(&data, &queries.results)
            .and_then(|v| v.as_array())
            .ok_or_else(|| SearchError::Parse("results array not found".to_string()))?;

        let mut results = Vec::new();
        for (idx, item) in results_array.iter().enumerate() {
            let title = get_json_path(item, &queries.title)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let url = get_json_path(item, &queries.url)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let url = if url.starts_with("http") {
                url
            } else {
                format!("{}{}", queries.url_prefix, url)
            };

            let snippet = queries
                .content
                .as_ref()
                .and_then(|q| get_json_path(item, q))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if title.is_empty() || url.is_empty() {
                continue;
            }

            results.push(RawSearchResult {
                title,
                url,
                snippet,
                published_date: None,
                position: (idx + 1) as u32,
            });
        }

        Ok(results)
    }
}

#[async_trait]
impl SearchEngine for ConfigurableEngine {
    fn name(&self) -> &'static str {
        // Leak the string to get a 'static reference.
        // This is safe because engine names are set once at startup.
        Box::leak(self.config.name.clone().into_boxed_str())
    }

    fn categories(&self) -> &[&'static str] {
        // Same approach: leak the category strings.
        let cats: Vec<&'static str> = self
            .config
            .categories
            .iter()
            .map(|s| Box::leak(s.clone().into_boxed_str()) as &'static str)
            .collect();
        Box::leak(cats.into_boxed_slice())
    }

    fn timeout(&self) -> u64 {
        self.config.timeout
    }

    async fn search(&self, query: &SearchQuery) -> EngineResult<Vec<RawSearchResult>> {
        let url = self.config.build_url(
            &query.query,
            query.page,
            &query.language,
            &query.time_range,
            query.safe_search,
        );

        let mut request = match self.config.method.as_str() {
            "POST" => self.client.post(&url),
            _ => self.client.get(&url),
        };

        for (key, value) in &self.config.cookies {
            request = request.header("Cookie", format!("{}={}", key, value));
        }

        if let Some(ref body) = self.config.request_body {
            let body = body
                .replace("{query}", &query.query)
                .replace("{pageno}", &(self.config.first_page_num + query.page).to_string());
            request = request.body(body);
        }

        let resp = request.send().await?;

        if !resp.status().is_success() {
            return Err(SearchError::Request(format!(
                "HTTP {}",
                resp.status()
            )));
        }

        let body = resp.text().await?;

        match self.config.engine_type {
            EngineType::Html => self.parse_html(&body),
            EngineType::Json => self.parse_json(&body),
        }
    }
}

/// Get a value from a JSON object using a slash-separated path.
/// Example: "data/items" -> data["items"]
fn get_json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let mut current = value;
    for part in parts {
        current = current.get(part)?;
    }
    Some(current)
}
