//! URL deduplication service.
//!
//! Centralizes all URL normalization and dedup logic so the aggregator,
//! index, and crawler share one source of truth.

use std::collections::HashSet;
use std::sync::Mutex;

use url::Url;

/// In-memory URL dedup service.
///
/// Tracks normalized URLs to avoid:
/// - Duplicate results from multiple engines returning the same page.
/// - Re-indexing pages already in the local index.
pub struct DedupService {
    seen: Mutex<HashSet<String>>,
}

impl Default for DedupService {
    fn default() -> Self {
        Self::new()
    }
}

impl DedupService {
    pub fn new() -> Self {
        Self {
            seen: Mutex::new(HashSet::new()),
        }
    }

    /// Normalize a URL for comparison: lowercase host, strip tracking params,
    /// remove trailing slash and fragment, unwrap archive snapshots.
    pub fn normalize(&self, raw: &str) -> String {
        normalize_url(raw)
    }

    /// Returns true if the normalized URL was not seen before, and marks it seen.
    /// Returns false if the URL was already seen.
    pub fn insert(&self, url: &str) -> bool {
        let normalized = normalize_url(url);
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(normalized)
    }

    /// Returns true if the normalized URL has been seen.
    pub fn contains(&self, url: &str) -> bool {
        let normalized = normalize_url(url);
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&normalized)
    }

    /// Forget all seen URLs.
    pub fn clear(&self) {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// Lowercases host, strips tracking params (utm_*, fbclid, ...),
/// removes trailing slash and fragment.
/// Also unwraps web.archive.org and archive.today snapshots to their original URLs.
pub fn normalize_url(raw: &str) -> String {
    // Unwrap archive.org github repo snapshots:
    // https://archive.org/details/github.com-user-repo_-_2022-03-12_11-00-24
    // -> https://github.com/user/repo
    if let Some(rest) = raw.strip_prefix("https://archive.org/details/github.com-") {
        let parts: Vec<&str> = rest.splitn(3, '-').collect();
        if parts.len() >= 2 {
            let user = parts[0];
            let repo = parts[1];
            if !user.is_empty() && !repo.is_empty() {
                return format!("https://github.com/{}/{}", user, repo);
            }
        }
    }

    // Unwrap web archive snapshots to their original URLs.
    for prefix in &[
        "https://web.archive.org/web/",
        "https://archive.today/",
        "https://archive.ph/",
        "https://archive.fo/",
        "https://archive.li/",
        "https://archive.md/",
        "https://archive.vn/",
    ] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            if let Some(idx) = rest.find('/') {
                let after_ts = &rest[idx + 1..];
                if after_ts.starts_with("http://") || after_ts.starts_with("https://") {
                    return normalize_url(after_ts);
                }
            }
        }
    }

    let parsed = match Url::parse(raw) {
        Ok(u) => u,
        Err(_) => return raw.to_string(),
    };

    let scheme = parsed.scheme().to_string();
    let host = parsed.host_str().unwrap_or("").to_lowercase();
    let mut path = parsed.path().to_string();

    // Wikipedia treats spaces and underscores equivalently in article paths.
    // Normalize spaces to underscores so "Rust (programming language)" and
    // "Rust_(programming_language)" resolve to the same key.
    if host.contains("wikipedia.org") {
        path = path.replace(' ', "_");
    }

    // Remove trailing slash (but keep root "/")
    if path.len() > 1 && path.ends_with('/') {
        path.pop();
    }

    // Filter out tracking parameters
    let tracking_params = [
        "utm_source", "utm_medium", "utm_campaign", "utm_term", "utm_content",
        "fbclid", "gclid", "msclkid", "dclid", "yclid",
        "_ga", "_gl", "mc_cid", "mc_eid",
        "ref", "ref_src", "ref_url",
    ];

    let query: Vec<String> = parsed
        .query_pairs()
        .filter(|(k, _)| !tracking_params.contains(&k.as_ref()))
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();

    let query_string = if query.is_empty() {
        String::new()
    } else {
        format!("?{}", query.join("&"))
    };

    format!("{}://{}{}{}", scheme, host, path, query_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_removes_utm() {
        let url = "https://example.com/page?utm_source=twitter&q=rust";
        assert_eq!(normalize_url(url), "https://example.com/page?q=rust");
    }

    #[test]
    fn test_normalize_lowercases_host() {
        let url = "HTTPS://Example.COM/Page";
        assert_eq!(normalize_url(url), "https://example.com/Page");
    }

    #[test]
    fn test_normalize_removes_trailing_slash() {
        let url = "https://example.com/page/";
        assert_eq!(normalize_url(url), "https://example.com/page");
    }

    #[test]
    fn test_normalize_keeps_root() {
        let url = "https://example.com/";
        assert_eq!(normalize_url(url), "https://example.com/");
    }

    #[test]
    fn test_dedup_service_insert_and_contains() {
        let svc = DedupService::new();
        assert!(svc.insert("https://example.com/page"));
        assert!(!svc.insert("https://example.com/page"));
        assert!(svc.contains("https://example.com/page"));
        assert!(!svc.contains("https://example.com/other"));
    }

    #[test]
    fn test_dedup_service_normalizes_before_check() {
        let svc = DedupService::new();
        assert!(svc.insert("https://Example.COM/page/"));
        // Same URL with different host case and trailing slash should be seen.
        assert!(!svc.insert("https://example.com/page"));
    }
}
