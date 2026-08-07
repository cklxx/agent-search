//! Built-in search engine implementations.

pub mod bing;
pub mod brave;
pub mod duckduckgo;
pub mod searxng;

use std::sync::Arc;

use super::EngineRegistry;

/// Register all built-in engines.
pub fn register_builtin(registry: &mut EngineRegistry, searxng_url: &str) {
    // SearXNG as the primary upstream (gives access to 200+ engines)
    registry.register(Arc::new(searxng::Searxng::new(searxng_url)));

    // Native engines (fallback / direct)
    registry.register(Arc::new(duckduckgo::DuckDuckGo::new()));
    registry.register(Arc::new(bing::Bing::new()));
    registry.register(Arc::new(brave::Brave::new()));
}

/// Create an engine registry with all built-in engines.
pub fn builtin_registry(searxng_url: &str) -> EngineRegistry {
    let mut registry = EngineRegistry::new();
    register_builtin(&mut registry, searxng_url);
    registry
}
