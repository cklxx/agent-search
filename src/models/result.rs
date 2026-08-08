//! Search result models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_date: Option<DateTime<Utc>>,

    /// Relevance score (0.0 - 1.0).
    pub score: f32,

    /// All engines that returned this result (after dedup merge).
    pub engines: Vec<String>,
}

/// Raw result as returned by an engine (before scoring/merging).
#[derive(Debug, Clone)]
pub struct RawSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub published_date: Option<DateTime<Utc>>,
    /// 1-indexed position in the engine's result list.
    pub position: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineErrorInfo {
    pub engine: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub errors: Vec<EngineErrorInfo>,
}
