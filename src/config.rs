//! Configuration loading.

use serde::Deserialize;
use std::path::Path;

/// Server configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_host")]
    pub host: String,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,

    #[serde(default = "default_cache_size")]
    pub cache_size: u64,

    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_secs: u64,

    /// SearXNG upstream instance URL.
    #[serde(default = "default_searxng_url")]
    pub searxng_url: String,
}

fn default_searxng_url() -> String {
    "http://127.0.0.1:8888".to_string()
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_request_timeout() -> u64 {
    10
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
            request_timeout: default_request_timeout(),
            cache_size: default_cache_size(),
            cache_ttl_secs: default_cache_ttl(),
            searxng_url: default_searxng_url(),
        }
    }
}

impl Config {
    /// Load config from a TOML file. Falls back to defaults if the file doesn't exist.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }
}
