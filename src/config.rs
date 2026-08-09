//! Server configuration (TOML).

use serde::Deserialize;
use std::path::Path;

use crate::proxy::ProxyManager;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_host")]
    pub host: String,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_cache_size")]
    pub cache_size: u64,

    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_secs: u64,

    #[serde(default)]
    pub proxy_urls: Vec<String>,

    #[serde(default = "default_strategy")]
    pub strategy: String,

    /// Overall request timeout in seconds.
    #[serde(default = "default_request_timeout")]
    pub request_timeout_secs: u64,

    /// Queries to preheat on startup.
    #[serde(default)]
    pub warmup_queries: Vec<String>,

    /// Upstream search API base URL. Defaults to $UPSTREAM_SEARCH_URL.
    #[serde(default = "default_upstream_search_url")]
    pub upstream_search_url: Option<String>,

    /// API key for the upstream search API. Defaults to $UPSTREAM_API_KEY.
    #[serde(default = "default_upstream_api_key")]
    pub upstream_api_key: Option<String>,

    /// Enable the built-in MCP server.
    #[serde(default = "default_true")]
    pub mcp_enabled: bool,

    /// HTTP path for the MCP server endpoint.
    #[serde(default = "default_mcp_path")]
    pub mcp_path: String,

    /// Stack Exchange API key. Raises rate limit from 300 to 10,000 req/day.
    #[serde(default)]
    pub stackexchange_api_key: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_mcp_path() -> String {
    "/mcp".to_string()
}

fn default_upstream_search_url() -> Option<String> {
    std::env::var("UPSTREAM_SEARCH_URL").ok()
}

fn default_upstream_api_key() -> Option<String> {
    std::env::var("UPSTREAM_API_KEY").ok()
}

fn default_request_timeout() -> u64 {
    15
}

fn default_strategy() -> String {
    "bm25".to_string()
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    18789
}

fn default_cache_size() -> u64 {
    1000
}

fn default_cache_ttl() -> u64 {
    30
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            cache_size: default_cache_size(),
            cache_ttl_secs: default_cache_ttl(),
            proxy_urls: Vec::new(),
            strategy: default_strategy(),
            request_timeout_secs: default_request_timeout(),
            warmup_queries: Vec::new(),
            upstream_search_url: default_upstream_search_url(),
            upstream_api_key: default_upstream_api_key(),
            mcp_enabled: default_true(),
            mcp_path: default_mcp_path(),
            stackexchange_api_key: None,
        }
    }
}

impl Config {
    /// Falls back to defaults if the file doesn't exist.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn proxy_manager(&self) -> ProxyManager {
        ProxyManager::new(self.proxy_urls.clone())
    }
}
