//! Bing search engine (HTML scraping).

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;
use scraper::{Html, Selector};

use crate::engine::engines::{build_pool_clients, pick_client};
use crate::engine::trait_def::SearchEngine;
use crate::models::error::{EngineResult, SearchError};
use crate::models::query::SearchQuery;
use crate::models::result::RawSearchResult;
use crate::proxy::ProxyManager;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub struct Bing {
    pool_clients: Vec<Client>,
    default_client: Client,
    proxy_manager: Option<Arc<ProxyManager>>,
}

impl Bing {
    pub fn new(proxy_manager: Option<Arc<ProxyManager>>) -> Self {
        let headers = HeaderMap::new();
        let (pool_clients, default_client, proxy_manager) =
            build_pool_clients(USER_AGENT, &headers, proxy_manager);
        Self {
            pool_clients,
            default_client,
            proxy_manager,
        }
    }

    fn client(&self) -> &Client {
        pick_client(&self.pool_clients, &self.default_client, &self.proxy_manager)
    }
}

#[async_trait]
impl SearchEngine for Bing {
    fn name(&self) -> &'static str {
        "bing"
    }

    fn categories(&self) -> &[&'static str] {
        &["general"]
    }

    async fn search(&self, query: &SearchQuery) -> EngineResult<Vec<RawSearchResult>> {
        let start = query.page * 10 + 1;
        let url = if query.page == 0 {
            format!("https://www.bing.com/search?q={}", urlencoding::encode(&query.query))
        } else {
            format!(
                "https://www.bing.com/search?q={}&first={}",
                urlencoding::encode(&query.query),
                start
            )
        };

        let resp = self.client().get(&url).send().await?;
        let body = resp.text().await?;
        let doc = Html::parse_document(&body);

        let result_selector = Selector::parse(".b_algo").map_err(|e| SearchError::Parse(e.to_string()))?;
        let title_selector = Selector::parse("h2 a").map_err(|e| SearchError::Parse(e.to_string()))?;
        let snippet_selector = Selector::parse(".b_caption p").map_err(|e| SearchError::Parse(e.to_string()))?;

        let mut results = Vec::new();
        for (idx, result) in doc.select(&result_selector).enumerate() {
            let title_el = result.select(&title_selector).next();
            let snippet_el = result.select(&snippet_selector).next();

            let title = title_el
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let url = title_el
                .and_then(|e| e.value().attr("href"))
                .unwrap_or_default()
                .to_string();
            let snippet = snippet_el
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
