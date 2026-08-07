//! SearXNG upstream engine.
//!
//! Uses a SearXNG instance as the upstream search provider.
//! This gives us access to all 200+ engines that SearXNG supports
//! while we handle aggregation, dedup, scoring, and the agent API.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;

use crate::engine::engines::{build_pool_clients, pick_client};
use crate::engine::trait_def::SearchEngine;
use crate::models::error::{EngineResult, SearchError};
use crate::models::query::SearchQuery;
use crate::models::result::RawSearchResult;
use crate::proxy::ProxyManager;

const USER_AGENT: &str = "agent-search/0.1";

/// SearXNG upstream search engine.
pub struct Searxng {
    pool_clients: Vec<Client>,
    default_client: Client,
    proxy_manager: Option<Arc<ProxyManager>>,
    base_url: String,
}

impl Searxng {
    pub fn new(base_url: impl Into<String>, proxy_manager: Option<Arc<ProxyManager>>) -> Self {
        let headers = HeaderMap::new();
        let (pool_clients, default_client, proxy_manager) =
            build_pool_clients(USER_AGENT, &headers, proxy_manager);
        Self {
            pool_clients,
            default_client,
            proxy_manager,
            base_url: base_url.into(),
        }
    }

    fn client(&self) -> &Client {
        pick_client(&self.pool_clients, &self.default_client, &self.proxy_manager)
    }
}

#[async_trait]
impl SearchEngine for Searxng {
    fn name(&self) -> &'static str {
        "searxng"
    }

    fn categories(&self) -> &[&'static str] {
        &["general"]
    }

    fn timeout(&self) -> u64 {
        15
    }

    async fn search(&self, query: &SearchQuery) -> EngineResult<Vec<RawSearchResult>> {
        let mut params = vec![
            ("q", query.query.clone()),
            ("format", "json".to_string()),
            ("pageno", (query.page + 1).to_string()),
            // Use a curated set of reliable general engines
            ("engines", "bing,google,wikipedia,duckduckgo".to_string()),
        ];

        if let Some(ref lang) = query.language {
            params.push(("language", lang.clone()));
        }
        if let Some(ref time_range) = query.time_range {
            params.push(("time_range", time_range.clone()));
        }
        if query.safe_search > 0 {
            params.push(("safesearch", query.safe_search.to_string()));
        }

        let url = format!("{}/search", self.base_url);
        let resp = self.client().get(&url).query(&params).send().await?;

        if !resp.status().is_success() {
            return Err(SearchError::Request(format!(
                "SearXNG returned status {}",
                resp.status()
            )));
        }

        let body: serde_json::Value = resp.json().await?;

        let results_arr = body
            .get("results")
            .and_then(|r| r.as_array())
            .ok_or_else(|| SearchError::Parse("missing results array".to_string()))?;

        let mut results = Vec::new();
        for (idx, item) in results_arr.iter().enumerate() {
            let title = item
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            let url = item
                .get("url")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string();
            let snippet = item
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();

            if title.is_empty() || url.is_empty() {
                continue;
            }

            let published_date = item
                .get("publishedDate")
                .and_then(|d| d.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));

            results.push(RawSearchResult {
                title,
                url,
                snippet,
                published_date,
                position: (idx + 1) as u32,
            });
        }

        Ok(results)
    }
}
