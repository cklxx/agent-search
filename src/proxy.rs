//! Proxy pool manager with round-robin rotation.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Manages a pool of proxy URLs and hands them out in round-robin order.
///
/// Cloning is cheap: the URL list is shared behind an `Arc`, and the
/// round-robin counter is atomic so all clones share the same position.
#[derive(Clone)]
pub struct ProxyManager {
    urls: std::sync::Arc<Vec<String>>,
    index: std::sync::Arc<AtomicUsize>,
}

impl ProxyManager {
    /// Create a new proxy manager from a list of proxy URLs.
    ///
    /// Empty URLs are filtered out.
    pub fn new(urls: Vec<String>) -> Self {
        let urls: Vec<String> = urls.into_iter().filter(|u| !u.trim().is_empty()).collect();
        Self {
            urls: std::sync::Arc::new(urls),
            index: std::sync::Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Returns true if the proxy pool is empty.
    pub fn is_empty(&self) -> bool {
        self.urls.is_empty()
    }

    /// Number of proxies in the pool.
    pub fn len(&self) -> usize {
        self.urls.len()
    }

    /// The list of proxy URLs.
    pub fn urls(&self) -> &[String] {
        &self.urls
    }

    /// Return the next proxy URL in round-robin order.
    ///
    /// Returns `None` if the pool is empty.
    pub fn next(&self) -> Option<&str> {
        if self.urls.is_empty() {
            return None;
        }
        let idx = self.index.fetch_add(1, Ordering::Relaxed) % self.urls.len();
        Some(self.urls[idx].as_str())
    }

    /// Return the index of the next proxy in round-robin order.
    ///
    /// Useful for indexing into a pre-built client array aligned with [`urls`].
    /// Returns `None` if the pool is empty.
    pub fn next_index(&self) -> Option<usize> {
        if self.urls.is_empty() {
            return None;
        }
        Some(self.index.fetch_add(1, Ordering::Relaxed) % self.urls.len())
    }
}

impl Default for ProxyManager {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pool_returns_none() {
        let pm = ProxyManager::new(vec![]);
        assert!(pm.is_empty());
        assert_eq!(pm.next(), None);
        assert_eq!(pm.next_index(), None);
    }

    #[test]
    fn round_robin_rotates() {
        let pm = ProxyManager::new(vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(pm.len(), 3);
        assert_eq!(pm.next(), Some("a"));
        assert_eq!(pm.next(), Some("b"));
        assert_eq!(pm.next(), Some("c"));
        assert_eq!(pm.next(), Some("a")); // wraps around
    }

    #[test]
    fn single_proxy_always_returns_same() {
        let pm = ProxyManager::new(vec!["only".into()]);
        assert_eq!(pm.next(), Some("only"));
        assert_eq!(pm.next(), Some("only"));
    }

    #[test]
    fn empty_strings_filtered() {
        let pm = ProxyManager::new(vec!["a".into(), "".into(), "  ".into(), "b".into()]);
        assert_eq!(pm.len(), 2);
        assert_eq!(pm.next(), Some("a"));
        assert_eq!(pm.next(), Some("b"));
    }

    #[test]
    fn next_index_aligns_with_urls() {
        let pm = ProxyManager::new(vec!["a".into(), "b".into()]);
        let i0 = pm.next_index().unwrap();
        let i1 = pm.next_index().unwrap();
        let i2 = pm.next_index().unwrap();
        assert_eq!(pm.urls()[i0], "a");
        assert_eq!(pm.urls()[i1], "b");
        assert_eq!(pm.urls()[i2], "a");
    }
}
