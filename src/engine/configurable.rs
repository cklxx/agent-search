//! Configurable engine driven by YAML config (html or json).

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;
use scraper::{Html, Selector};

use crate::engine::engines::{build_pool_clients, pick_client};
use crate::engine::config::{EngineConfig, EngineType};
use crate::engine::trait_def::SearchEngine;
use crate::models::error::{EngineResult, SearchError};
use crate::models::query::SearchQuery;
use crate::models::result::RawSearchResult;
use crate::proxy::ProxyManager;

const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub struct ConfigurableEngine {
    config: EngineConfig,
    pool_clients: Vec<Client>,
    default_client: Client,
    proxy_manager: Option<Arc<ProxyManager>>,
}

impl ConfigurableEngine {
    /// Proxy precedence: engine `proxy` > global pool > none.
    pub fn new(config: EngineConfig, proxy_manager: Option<Arc<ProxyManager>>) -> Self {
        let headers = build_headers(&config);

        // Engine-specific proxy: wrap as a single-element pool.
        let proxy_manager = match &config.proxy {
            Some(url) => Some(Arc::new(ProxyManager::new(vec![url.clone()]))),
            None => proxy_manager,
        };

        let (pool_clients, default_client, proxy_manager) =
            build_pool_clients(DEFAULT_USER_AGENT, &headers, proxy_manager);
        Self { config, pool_clients, default_client, proxy_manager }
    }

    fn client(&self) -> &Client {
        pick_client(&self.pool_clients, &self.default_client, &self.proxy_manager)
    }

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

    fn parse_json(&self, body: &str) -> EngineResult<Vec<RawSearchResult>> {
        let queries = self.config.json.as_ref().ok_or_else(|| {
            SearchError::Parse("json queries not configured".to_string())
        })?;

        let data: serde_json::Value =
            serde_json::from_str(body).map_err(|e| SearchError::Parse(e.to_string()))?;

        // Detect API error responses (e.g. Stack Exchange throttle_violation).
        // These return HTTP 200 with an error body, not a 4xx status, so we
        // check for common error fields and treat them as rate-limit errors
        // to trigger the 180s suspension instead of noisy parse-error retries.
        if data.get("error").is_some()
            || data.get("error_message").is_some()
            || data.get("error_id").is_some()
        {
            return Err(SearchError::HttpStatus(429));
        }

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

    /// Parse Atom/RSS XML using the same CSS selector config as HTML engines.
    /// URL is extracted from element text (e.g. <id>) or href attribute.
    fn parse_xml(&self, body: &str) -> EngineResult<Vec<RawSearchResult>> {
        let selectors = self.config.html.as_ref().ok_or_else(|| {
            SearchError::Parse("xml selectors not configured".to_string())
        })?;

        // Strip XML declaration and namespaces — html5ever (used by scraper)
        // is an HTML parser and doesn't handle XML prologs or namespaces.
        let cleaned = body
            .trim_start_matches(|c: char| c != '<')
            .replace("xmlns=", "xmlns_stripped=")
            .replace("xmlns:", "xmlns_stripped_");
        let doc = Html::parse_document(&cleaned);
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

            // XML: URL may be in href attribute or element text (e.g. <id>).
            let url = match item.select(&url_sel).next() {
                Some(el) => el
                    .value()
                    .attr("href")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| el.text().collect::<String>().trim().to_string()),
                None => String::new(),
            };

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
}
fn build_headers(config: &EngineConfig) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (key, value) in &config.headers {
        if let (Ok(k), Ok(v)) = (
            reqwest::header::HeaderName::from_bytes(key.as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            headers.insert(k, v);
        }
    }
    headers
}

#[async_trait]
impl SearchEngine for ConfigurableEngine {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn categories(&self) -> &[String] {
        &self.config.categories
    }

    fn timeout(&self) -> u64 {
        self.config.timeout
    }

    fn weight(&self) -> f32 {
        self.config.weight
    }

    async fn search(&self, query: &SearchQuery) -> EngineResult<Vec<RawSearchResult>> {
        let url = self.config.build_url(
            &query.query,
            query.page,
            &query.language,
            &query.time_range,
            query.safe_search,
        );

        let client = self.client();

        let mut request = match self.config.method.as_str() {
            "POST" => client.post(&url),
            _ => client.get(&url),
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
            return Err(SearchError::HttpStatus(resp.status().as_u16()));
        }

        let body = resp.text().await?;

        match self.config.engine_type {
            EngineType::Html => self.parse_html(&body),
            EngineType::Json => self.parse_json(&body),
            EngineType::Xml => self.parse_xml(&body),
        }
    }
}

/// Get a value from a JSON object using a slash-separated path.
/// Thin wrapper around `serde_json::Value::pointer` (RFC 6901).
/// Numeric segments index arrays; string segments index object keys.
/// Example: "data/items/0/title" -> data["items"][0]["title"]
fn get_json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return Some(value);
    }
    value.pointer(&format!("/{}", path))
}
