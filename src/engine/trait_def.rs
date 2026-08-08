//! Search engine trait.

use async_trait::async_trait;

use crate::models::error::EngineResult;
use crate::models::query::SearchQuery;
use crate::models::result::RawSearchResult;

#[async_trait]
pub trait SearchEngine: Send + Sync {
    fn name(&self) -> &'static str;
    fn categories(&self) -> &[&'static str];

    fn timeout(&self) -> u64 {
        10
    }

    fn weight(&self) -> f32 {
        1.0
    }

    async fn search(&self, query: &SearchQuery) -> EngineResult<Vec<RawSearchResult>>;
}
