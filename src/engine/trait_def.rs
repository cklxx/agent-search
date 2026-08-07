//! Search engine trait definition.

use async_trait::async_trait;

use crate::models::error::EngineResult;
use crate::models::query::SearchQuery;
use crate::models::result::RawSearchResult;

/// Trait that all search engines must implement.
#[async_trait]
pub trait SearchEngine: Send + Sync {
    /// Engine name (unique identifier).
    fn name(&self) -> &'static str;

    /// Categories this engine belongs to (e.g. "general", "news").
    fn categories(&self) -> &[&'static str];

    /// Request timeout in seconds.
    fn timeout(&self) -> u64 {
        10
    }

    /// Whether the engine supports the given query.
    fn supports(&self, _query: &SearchQuery) -> bool {
        true
    }

    /// Execute a search and return raw results.
    async fn search(&self, query: &SearchQuery) -> EngineResult<Vec<RawSearchResult>>;

    /// Health check. Returns true if the engine is operational.
    async fn health_check(&self) -> bool {
        true
    }
}
