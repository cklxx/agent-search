//! Declarative engine configuration (YAML).
//!
//! Two engine types: `html` (CSS selectors) and `json` (field mapping).
//! Adding an engine only requires a YAML entry — no Rust code.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Engine configuration loaded from YAML.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EngineConfig {
    /// Engine name (unique identifier).
    pub name: String,

    /// Engine type: "html" or "json".
    #[serde(rename = "type")]
    pub engine_type: EngineType,

    /// Categories this engine belongs to.
    #[serde(default = "default_categories")]
    pub categories: Vec<String>,

    /// Search URL template.
    /// Placeholders: {query}, {pageno}, {lang}, {time_range}, {safe_search}
    pub search_url: String,

    /// HTTP method: "GET" or "POST".
    #[serde(default = "default_method")]
    pub method: String,

    /// Request body template (for POST).
    #[serde(default)]
    pub request_body: Option<String>,

    /// Additional headers.
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// Additional cookies.
    #[serde(default)]
    pub cookies: HashMap<String, String>,

    /// Whether the engine supports paging.
    #[serde(default)]
    pub paging: bool,

    /// Page size (for offset-based pagination).
    #[serde(default = "default_page_size")]
    pub page_size: u32,

    /// First page number (0 or 1).
    #[serde(default = "default_first_page_num")]
    pub first_page_num: u32,

    /// Whether to send page number on the first page.
    #[serde(default = "default_true")]
    pub send_page_num_on_first_page: bool,

    /// Language to use when "all" is selected.
    #[serde(default = "default_lang_all")]
    pub lang_all: String,

    /// Whether the engine supports time range.
    #[serde(default)]
    pub time_range_support: bool,

    /// Time range URL parameter template.
    #[serde(default = "default_time_range_url")]
    pub time_range_url: String,

    /// Time range value mapping.
    #[serde(default)]
    pub time_range_map: HashMap<String, String>,

    /// Whether the engine supports safe search.
    #[serde(default)]
    pub safesearch: bool,

    /// Safe search value mapping.
    #[serde(default)]
    pub safe_search_map: HashMap<u8, String>,

    /// Request timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout: u64,

    /// Engine weight for scoring (higher = more trusted).
    #[serde(default = "default_weight")]
    pub weight: f32,

    /// Optional proxy URL for this engine (overrides global proxy pool).
    #[serde(default)]
    pub proxy: Option<String>,

    /// HTML-specific selectors (only for type "html").
    #[serde(default)]
    pub html: Option<HtmlSelectors>,

    /// JSON-specific queries (only for type "json").
    #[serde(default)]
    pub json: Option<JsonQueries>,
}

fn default_categories() -> Vec<String> {
    vec!["general".to_string()]
}

fn default_method() -> String {
    "GET".to_string()
}

fn default_page_size() -> u32 {
    1
}

fn default_first_page_num() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

fn default_lang_all() -> String {
    "en".to_string()
}

fn default_time_range_url() -> String {
    String::new()
}

fn default_timeout() -> u64 {
    5
}

fn default_weight() -> f32 {
    1.0
}

/// Engine type.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EngineType {
    Html,
    Json,
    Xml,
}

/// CSS selectors for HTML engines.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HtmlSelectors {
    /// CSS selector for the list of result items.
    pub results: String,

    /// CSS selector for the result URL (href attribute).
    pub url: String,

    /// CSS selector for the result title.
    pub title: String,

    /// CSS selector for the result description/snippet.
    #[serde(default)]
    pub content: Option<String>,

    /// CSS selector for the result thumbnail.
    #[serde(default)]
    pub thumbnail: Option<String>,
}

/// JSON field queries for JSON engines.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JsonQueries {
    /// JSON path to the results array (slash-separated keys).
    pub results: String,

    /// JSON path to the result URL.
    pub url: String,

    /// JSON path to the result title.
    pub title: String,

    /// JSON path to the result description.
    #[serde(default)]
    pub content: Option<String>,

    /// Prefix to prepend to the URL.
    #[serde(default)]
    pub url_prefix: String,
}

impl EngineConfig {
    /// Build the search URL from the template and query parameters.
    pub fn build_url(&self, query: &str, page: u32, language: &Option<String>, time_range: &Option<String>, safe_search: u8) -> String {
        let pageno = if !self.paging || (page == 0 && !self.send_page_num_on_first_page) {
            self.first_page_num
        } else {
            self.first_page_num + page
        };

        let lang = language.as_deref().unwrap_or(&self.lang_all);
        let lang = if lang == "all" { &self.lang_all } else { lang };

        let time_range_str = if self.time_range_support {
            time_range
                .as_ref()
                .and_then(|tr| self.time_range_map.get(tr))
                .map(|val| self.time_range_url.replace("{time_range_val}", val))
                .unwrap_or_default()
        } else {
            String::new()
        };

        let safe_search_str = if self.safesearch {
            self.safe_search_map.get(&safe_search).cloned().unwrap_or_default()
        } else {
            String::new()
        };

        self.search_url
            .replace("{query}", &urlencoding::encode(query))
            .replace("{pageno}", &pageno.to_string())
            .replace("{lang}", lang)
            .replace("{time_range}", &time_range_str)
            .replace("{safe_search}", &safe_search_str)
    }
}

/// Load engine configurations from a YAML file.
pub fn load_engines_from_yaml(path: &std::path::Path) -> Result<Vec<EngineConfig>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let configs: Vec<EngineConfig> = serde_yaml::from_str(&content).map_err(|e| e.to_string())?;
    Ok(configs)
}
