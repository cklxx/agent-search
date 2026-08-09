//! Web crawler: fetch pages and extract main content.

use reqwest::Client;
use scraper::{Html, Selector};

/// Fetches a page and returns (title, main_content).
pub async fn fetch_and_extract(
    client: &Client,
    url: &str,
) -> Result<(String, String), reqwest::Error> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(resp.error_for_status().unwrap_err());
    }
    let bytes = resp.bytes().await?;
    let body = String::from_utf8_lossy(&bytes).into_owned();
    Ok(extract_content(&body))
}

/// Extract title and main text content from HTML.
/// Strategy: <title> for title; <article> or the text-densest block for content.
pub fn extract_content(html: &str) -> (String, String) {
    let doc = Html::parse_document(html);

    let title = doc
        .select(&Selector::parse("title").unwrap())
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .unwrap_or_default();

    // Remove non-content elements before extracting text.
    let cleaned = remove_noise(html);
    let doc = Html::parse_document(&cleaned);

    // Prefer <article>; fall back to the densest <div>.
    let content = if let Some(article) = doc.select(&Selector::parse("article").unwrap()).next() {
        article.text().collect::<String>()
    } else {
        find_densest_block(&doc)
    };

    let content = normalize_whitespace(&content);
    (title, content)
}

/// Strip script, style, nav, header, footer, aside, form elements.
fn remove_noise(html: &str) -> String {
    let mut result = html.to_string();
    for tag in [
        "script", "style", "noscript", "nav", "header", "footer", "aside",
        "form", "iframe", "svg", "canvas",
    ] {
        let open = format!("<{}", tag);
        let close = format!("</{}>", tag);
        while let Some(start) = result.find(&open) {
            if let Some(end) = result[start..].find(&close) {
                let end = start + end + close.len();
                result.replace_range(start..end, " ");
            } else {
                break;
            }
        }
    }
    result
}

/// Find the block element with the highest text length.
fn find_densest_block(doc: &Html) -> String {
    let div_sel = Selector::parse("div, section, main").unwrap();
    let mut best = String::new();
    for el in doc.select(&div_sel) {
        let text: String = el.text().collect();
        if text.chars().count() > best.chars().count() {
            best = text;
        }
    }
    best
}

/// Collapse whitespace and trim.
fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_article() {
        let html = r#"
        <html><head><title>Test Page</title></head>
        <body>
            <nav>menu</nav>
            <article><h1>Hello</h1><p>This is the main content.</p></article>
            <footer>copyright</footer>
        </body></html>
        "#;
        let (title, content) = extract_content(html);
        assert_eq!(title, "Test Page");
        assert!(content.contains("main content"));
        assert!(!content.contains("menu"));
        assert!(!content.contains("copyright"));
    }

    #[test]
    fn test_extract_densest_div() {
        let html = r#"
        <html><head><title>Dense</title></head>
        <body>
            <div class="sidebar">short</div>
            <div class="content">This is a longer piece of text that should be selected as the main content block.</div>
        </body></html>
        "#;
        let (_title, content) = extract_content(html);
        assert!(content.contains("longer piece of text"));
    }
}
