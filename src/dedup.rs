//! URL normalization for deduplication.

use url::Url;

/// Lowercases host, strips tracking params (utm_*, fbclid, ...),
/// removes trailing slash and fragment.
/// Also unwraps web.archive.org snapshots to their original URLs.
pub fn normalize_url(raw: &str) -> String {
    // Unwrap Wayback Machine snapshots:
    // https://web.archive.org/web/20220312110024/https://github.com/...
    // -> https://github.com/...
    if let Some(rest) = raw.strip_prefix("https://web.archive.org/web/") {
        if let Some(idx) = rest.find('/') {
            let after_ts = &rest[idx + 1..];
            if after_ts.starts_with("http://") || after_ts.starts_with("https://") {
                return normalize_url(after_ts);
            }
        }
    }

    // Unwrap archive.org github repo snapshots:
    // https://archive.org/details/github.com-user-repo_-_2022-03-12_11-00-24
    // -> https://github.com/user/repo
    if let Some(rest) = raw.strip_prefix("https://archive.org/details/github.com-") {
        // Format: github.com-<user>-<repo>_-_<date> or github.com-<user>-<repo>
        let parts: Vec<&str> = rest.splitn(3, '-').collect();
        if parts.len() >= 2 {
            let user = parts[0];
            let repo = parts[1];
            if !user.is_empty() && !repo.is_empty() {
                return format!("https://github.com/{}/{}", user, repo);
            }
        }
    }

    // Unwrap other web archive services to their original URLs.
    // archive.today / archive.ph: https://archive.today/20220101000000/https://example.com
    for prefix in &[
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
