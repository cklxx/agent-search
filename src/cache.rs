//! In-memory query cache (moka).

use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;

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

pub fn cache_key(query: &str, page: u32, max_results: usize) -> String {
    format!("{}:{}:{}", query, page, max_results)
}
