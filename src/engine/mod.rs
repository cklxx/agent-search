//! Search engine trait and registry.

pub mod config;
pub mod configurable;
pub mod domain_rate_limit;
pub mod engines;
pub mod suspension;
pub mod trait_def;

pub use config::*;
pub use configurable::ConfigurableEngine;
pub use domain_rate_limit::{DomainRateLimiter, DEFAULT_MAX_PER_DOMAIN};
pub use suspension::EngineSuspensionManager;
pub use trait_def::*;

use std::collections::HashMap;
use std::sync::Arc;

pub type EngineRef = Arc<dyn SearchEngine>;

#[derive(Default)]
pub struct EngineRegistry {
    engines: HashMap<String, EngineRef>,
}

impl EngineRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, engine: EngineRef) {
        self.engines.insert(engine.name().to_string(), engine);
    }

    pub fn by_category(&self, category: &str) -> Vec<EngineRef> {
        self.engines
            .values()
            .filter(|e| e.categories().iter().any(|c| c == category))
            .cloned()
            .collect()
    }

    pub fn names(&self) -> Vec<String> {
        self.engines.keys().cloned().collect()
    }
}
