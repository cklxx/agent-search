//! Search engine trait and registry.

pub mod trait_def;
pub mod engines;

pub use trait_def::*;

use std::collections::HashMap;
use std::sync::Arc;

/// A reference-counted engine handle.
pub type EngineRef = Arc<dyn SearchEngine>;

/// Registry of available search engines.
#[derive(Default)]
pub struct EngineRegistry {
    engines: HashMap<String, EngineRef>,
}

impl EngineRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an engine.
    pub fn register(&mut self, engine: EngineRef) {
        self.engines.insert(engine.name().to_string(), engine);
    }

    /// Get an engine by name.
    pub fn get(&self, name: &str) -> Option<EngineRef> {
        self.engines.get(name).cloned()
    }

    /// Get all registered engines.
    pub fn all(&self) -> Vec<EngineRef> {
        self.engines.values().cloned().collect()
    }

    /// Get engines filtered by category.
    pub fn by_category(&self, category: &str) -> Vec<EngineRef> {
        self.engines
            .values()
            .filter(|e| e.categories().contains(&category))
            .cloned()
            .collect()
    }

    /// Get engine names.
    pub fn names(&self) -> Vec<String> {
        self.engines.keys().cloned().collect()
    }
}
