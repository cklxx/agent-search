//! Built-in search engines.

use std::sync::Arc;

use reqwest::header::HeaderMap;
use reqwest::Client;

use super::{ConfigurableEngine, EngineRegistry};
use crate::engine::config::load_engines_from_yaml;
use crate::proxy::ProxyManager;

/// Returns `(pool_clients, default_client, proxy_manager)`.
/// `pool_clients` is aligned with `ProxyManager::urls`; empty if no proxies.
/// Proxies that fail to build a client are skipped (logged as warnings).
pub(crate) fn build_pool_clients(
    user_agent: &str,
    headers: &HeaderMap,
    proxy_manager: Option<Arc<ProxyManager>>,
) -> (Vec<Client>, Client, Option<Arc<ProxyManager>>) {
    let build = |proxy: Option<&str>| -> Option<Client> {
        let mut builder = Client::builder()
            .user_agent(user_agent)
            .default_headers(headers.clone());
        if let Some(url) = proxy {
            match reqwest::Proxy::all(url) {
                Ok(p) => builder = builder.proxy(p),
                Err(e) => {
                    tracing::warn!("invalid proxy URL {}: {}", url, e);
                    return None;
                }
            }
        }
        match builder.build() {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("failed to build HTTP client: {}", e);
                None
            }
        }
    };

    if let Some(pm) = proxy_manager {
        if !pm.is_empty() {
            let pool: Vec<Client> = pm
                .urls()
                .iter()
                .filter_map(|u| build(Some(u)))
                .collect();
            let default = build(None).expect("default client build must succeed");
            return (pool, default, Some(pm));
        }
    }

    let default = build(None).expect("default client build must succeed");
    (Vec::new(), default, None)
}

pub(crate) fn pick_client<'a>(
    pool_clients: &'a [Client],
    default_client: &'a Client,
    proxy_manager: &Option<Arc<ProxyManager>>,
) -> &'a Client {
    if let Some(pm) = proxy_manager {
        if let Some(idx) = pm.next_index() {
            return &pool_clients[idx];
        }
    }
    default_client
}

/// All engines come from `engines.yaml`. No built-in Rust impls.
pub fn register_from_config(
    registry: &mut EngineRegistry,
    config_path: &std::path::Path,
    proxy_manager: Option<Arc<ProxyManager>>,
    stackexchange_api_key: Option<&str>,
) {
    match load_engines_from_yaml(config_path) {
        Ok(mut configs) => {
            let key = stackexchange_api_key.unwrap_or("");
            for config in &mut configs {
                if key.is_empty() {
                    // No API key: remove the key parameter entirely to avoid
                    // sending `key=` which Stack Exchange rejects.
                    config.search_url = config.search_url.replace("&key={api_key}", "");
                    config.search_url = config.search_url.replace("key={api_key}&", "");
                    config.search_url = config.search_url.replace("?key={api_key}", "");
                } else {
                    config.search_url = config.search_url.replace("{api_key}", key);
                }
            }
            for config in configs {
                let engine = ConfigurableEngine::new(config, proxy_manager.clone());
                registry.register(Arc::new(engine));
            }
        }
        Err(e) => {
            tracing::warn!("failed to load engine config from {:?}: {}", config_path, e);
        }
    }
}

pub fn builtin_registry(
    proxy_manager: Option<Arc<ProxyManager>>,
    stackexchange_api_key: Option<&str>,
) -> EngineRegistry {
    let mut registry = EngineRegistry::new();

    let config_path = std::path::Path::new("engines.yaml");
    if config_path.exists() {
        register_from_config(&mut registry, config_path, proxy_manager, stackexchange_api_key);
    }

    registry
}
