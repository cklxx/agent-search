//! In-memory query cache (moka).

use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;

use crate::models::query::SearchQuery;
use crate::models::result::SearchResponse;

#[derive(Clone)]
pub struct QueryCache {
    cache: Cache<String, Arc<SearchResponse>>,
}

impl QueryCache {
    pub fn new(max_capacity: u64, ttl: Duration) -> Self {
        let cache = Cache::builder()
            .max_capacity(max_capacity)
            .time_to_live(ttl)
            .build();
        Self { cache }
    }

    pub async fn get(&self, key: &str) -> Option<Arc<SearchResponse>> {
        self.cache.get(key).await
    }

    pub async fn insert(&self, key: String, response: Arc<SearchResponse>) {
        self.cache.insert(key, response).await;
    }
}

/// Build a cache key from all query fields that affect results.
pub fn cache_key(query: &SearchQuery) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}",
        query.query,
        query.page,
        query.max_results,
        query.language.as_deref().unwrap_or(""),
        query.time_range.as_deref().unwrap_or(""),
        query.safe_search,
        query.category.as_deref().unwrap_or(""),
    )
}
