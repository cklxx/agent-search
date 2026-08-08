//! Built-in search engines.

pub mod bing;
pub mod brave;
pub mod duckduckgo;
pub mod searxng;

use std::sync::Arc;

use reqwest::header::HeaderMap;
use reqwest::Client;

use super::{ConfigurableEngine, EngineRegistry};
use crate::engine::config::load_engines_from_yaml;
use crate::proxy::ProxyManager;

/// Returns `(pool_clients, default_client, proxy_manager)`.
/// `pool_clients` is aligned with `ProxyManager::urls`; empty if no proxies.
pub(crate) fn build_pool_clients(
    user_agent: &str,
    headers: &HeaderMap,
    proxy_manager: Option<Arc<ProxyManager>>,
) -> (Vec<Client>, Client, Option<Arc<ProxyManager>>) {
    let build = |proxy: Option<&str>| -> Client {
        let mut builder = Client::builder()
            .user_agent(user_agent)
            .default_headers(headers.clone());
        if let Some(url) = proxy {
            if let Ok(p) = reqwest::Proxy::all(url) {
                builder = builder.proxy(p);
            }
        }
        builder.build().expect("failed to build HTTP client")
    };

    if let Some(pm) = proxy_manager {
        if !pm.is_empty() {
            let pool = pm.urls().iter().map(|u| build(Some(u))).collect();
            let default = build(None);
            return (pool, default, Some(pm));
        }
    }

    let default = build(None);
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

pub fn register_builtin(
    registry: &mut EngineRegistry,
    searxng_url: &str,
    proxy_manager: Option<Arc<ProxyManager>>,
) {
    registry.register(Arc::new(searxng::Searxng::new(searxng_url, proxy_manager.clone())));
    registry.register(Arc::new(duckduckgo::DuckDuckGo::new(proxy_manager.clone())));
    registry.register(Arc::new(bing::Bing::new(proxy_manager.clone())));
    registry.register(Arc::new(brave::Brave::new(proxy_manager)));
}

pub fn register_from_config(
    registry: &mut EngineRegistry,
    config_path: &std::path::Path,
    proxy_manager: Option<Arc<ProxyManager>>,
) {
    match load_engines_from_yaml(config_path) {
        Ok(configs) => {
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
    searxng_url: &str,
    proxy_manager: Option<Arc<ProxyManager>>,
) -> EngineRegistry {
    let mut registry = EngineRegistry::new();
    register_builtin(&mut registry, searxng_url, proxy_manager.clone());

    let config_path = std::path::Path::new("engines.yaml");
    if config_path.exists() {
        register_from_config(&mut registry, config_path, proxy_manager);
    }

    registry
}
