//! URL normalization for deduplication.

use url::Url;

/// Lowercases host, strips tracking params (utm_*, fbclid, ...),
/// removes trailing slash and fragment.
pub fn normalize_url(raw: &str) -> String {
    let parsed = match Url::parse(raw) {
        Ok(u) => u,
        Err(_) => return raw.to_string(),
    };

    let scheme = parsed.scheme().to_string();
    let host = parsed.host_str().unwrap_or("").to_lowercase();
    let mut path = parsed.path().to_string();

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
}
