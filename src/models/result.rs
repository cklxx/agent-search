//! Search result models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single search result returned to the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Result title.
    pub title: String,

    /// Result URL.
    pub url: String,

    /// Short snippet/description.
    pub snippet: String,

    /// Full page content (only when include_raw_content is true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    /// Publication date if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_date: Option<DateTime<Utc>>,

    /// Relevance score (0.0 - 1.0).
    pub score: f32,

    /// Name of the engine that returned this result.
    pub engine: String,

    /// All engines that returned this result (after dedup merge).
    pub engines: Vec<String>,
}

/// Raw search result as returned by an engine (before scoring/merging).
#[derive(Debug, Clone)]
pub struct RawSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub published_date: Option<DateTime<Utc>>,
    /// Position in the engine's result list (1-indexed).
    pub position: u32,
}

/// Information about an engine that failed during a search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineErrorInfo {
    pub engine: String,
    pub error: String,
}

/// The complete search response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub errors: Vec<EngineErrorInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
}
