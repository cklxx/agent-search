//! Built-in search engine implementations.

pub mod bing;
pub mod brave;
pub mod duckduckgo;
pub mod searxng;

use std::sync::Arc;

use super::{ConfigurableEngine, EngineRegistry};
use crate::engine::config::load_engines_from_yaml;

/// Register all built-in engines.
pub fn register_builtin(registry: &mut EngineRegistry, searxng_url: &str) {
    // SearXNG as the primary upstream (gives access to 200+ engines)
    registry.register(Arc::new(searxng::Searxng::new(searxng_url)));

    // Native engines (fallback / direct)
    registry.register(Arc::new(duckduckgo::DuckDuckGo::new()));
    registry.register(Arc::new(bing::Bing::new()));
    registry.register(Arc::new(brave::Brave::new()));
}

/// Register engines from YAML configuration files.
pub fn register_from_config(registry: &mut EngineRegistry, config_path: &std::path::Path) {
    match load_engines_from_yaml(config_path) {
        Ok(configs) => {
            for config in configs {
                let engine = ConfigurableEngine::new(config);
                registry.register(Arc::new(engine));
            }
        }
        Err(e) => {
            tracing::warn!("failed to load engine config from {:?}: {}", config_path, e);
        }
    }
}

/// Create an engine registry with all built-in engines.
pub fn builtin_registry(searxng_url: &str) -> EngineRegistry {
    let mut registry = EngineRegistry::new();
    register_builtin(&mut registry, searxng_url);

    // Load configurable engines from YAML
    let config_path = std::path::Path::new("engines.yaml");
    if config_path.exists() {
        register_from_config(&mut registry, config_path);
    }

    registry
}
