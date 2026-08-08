//! Search query model.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,

    #[serde(default = "default_max_results")]
    pub max_results: usize,

    /// "day", "week", "month", "year", or null.
    #[serde(default)]
    pub time_range: Option<String>,

    /// e.g. "en", "zh".
    #[serde(default)]
    pub language: Option<String>,

    /// 0-indexed.
    #[serde(default)]
    pub page: u32,

    /// 0 = off, 1 = moderate, 2 = strict.
    #[serde(default)]
    pub safe_search: u8,
}

fn default_max_results() -> usize {
    10
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            max_results: default_max_results(),
            time_range: None,
            language: None,
            page: 0,
            safe_search: 0,
        }
    }
}
