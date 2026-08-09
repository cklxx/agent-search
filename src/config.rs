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
    8080
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
