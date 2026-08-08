//! Built-in search engines.

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

/// All engines come from `engines.yaml`. No built-in Rust impls.
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
    proxy_manager: Option<Arc<ProxyManager>>,
) -> EngineRegistry {
    let mut registry = EngineRegistry::new();

    let config_path = std::path::Path::new("engines.yaml");
    if config_path.exists() {
        register_from_config(&mut registry, config_path, proxy_manager);
    }

    registry
}
