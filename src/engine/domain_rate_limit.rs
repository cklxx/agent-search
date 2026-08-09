//! Per-domain concurrency limiting.
//!
//! Multiple engines may target the same upstream domain (e.g. several
//! Stack Exchange sites all hit `api.stackexchange.com`). Without a
//! per-domain cap, those engines fire simultaneously and trigger 429
//! rate limits. `DomainRateLimiter` assigns each domain its own
//! semaphore so requests to the same domain are serialized.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Default max concurrent requests per domain.
pub const DEFAULT_MAX_PER_DOMAIN: usize = 2;

pub struct DomainRateLimiter {
    semaphores: Mutex<HashMap<String, Arc<Semaphore>>>,
    max_per_domain: usize,
}

impl Default for DomainRateLimiter {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_PER_DOMAIN)
    }
}

impl DomainRateLimiter {
    pub fn new(max_per_domain: usize) -> Self {
        Self {
            semaphores: Mutex::new(HashMap::new()),
            max_per_domain,
        }
    }

    /// Acquire a permit for the given domain. Blocks until one is available.
    /// The permit is released when dropped.
    pub async fn acquire(&self, domain: &str) -> OwnedSemaphorePermit {
        let sem = {
            let mut map = self.semaphores.lock().unwrap_or_else(|e| e.into_inner());
            map.entry(domain.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(self.max_per_domain)))
                .clone()
        };
        // Semaphore only errors when closed; we never close these semaphores.
        sem.acquire_owned().await.expect("domain semaphore closed")
    }
}
