//! Search query model.

use serde::{Deserialize, Serialize};

/// A search query submitted by an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    /// The search query string.
    pub query: String,

    /// Maximum number of results to return.
    #[serde(default = "default_max_results")]
    pub max_results: usize,

    /// Search depth: "basic" or "advanced".
    #[serde(default = "default_depth")]
    pub search_depth: String,

    /// Whether to include an AI-generated answer summary.
    #[serde(default)]
    pub include_answer: bool,

    /// Whether to include raw page content in results.
    #[serde(default)]
    pub include_raw_content: bool,

    /// Time range filter: "day", "week", "month", "year", or null.
    #[serde(default)]
    pub time_range: Option<String>,

    /// Domain allowlist. If empty, all domains are allowed.
    #[serde(default)]
    pub domains: Vec<String>,

    /// Language filter (e.g. "en", "zh").
    #[serde(default)]
    pub language: Option<String>,

    /// Page number (0-indexed).
    #[serde(default)]
    pub page: u32,

    /// Safe search level: 0 = off, 1 = moderate, 2 = strict.
    #[serde(default)]
    pub safe_search: u8,
}

fn default_max_results() -> usize {
    10
}

fn default_depth() -> String {
    "basic".to_string()
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            max_results: default_max_results(),
            search_depth: default_depth(),
            include_answer: false,
            include_raw_content: false,
            time_range: None,
            domains: Vec::new(),
            language: None,
            page: 0,
            safe_search: 0,
        }
    }
}
