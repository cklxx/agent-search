//! Web page content fetcher for agents.
//!
//! Fetches full page content and extracts main text,
//! which is more useful for LLMs than just snippets.

use reqwest::Client;
use scraper::{Html, Selector};

/// Fetched page content.
#[derive(Debug, Clone)]
pub struct FetchedContent {
    pub url: String,
    pub title: String,
    pub content: String,
    pub fetched_at: chrono::DateTime<chrono::Utc>,
}

/// Fetch a URL and extract the main text content.
pub async fn fetch_content(url: &str) -> Result<FetchedContent, reqwest::Error> {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()?;

    let resp = client.get(url).send().await?;
    let body = resp.text().await?;

    let doc = Html::parse_document(&body);

    // Extract title
    let title_selector = Selector::parse("title").unwrap();
    let title = doc
        .select(&title_selector)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .unwrap_or_default();

    // Extract main content
    let content = extract_main_text(&doc);

    Ok(FetchedContent {
        url: url.to_string(),
        title,
        content,
        fetched_at: chrono::Utc::now(),
    })
}

/// Extract the main text content from an HTML document.
///
/// Strategy:
/// 1. Remove script, style, nav, header, footer elements
/// 2. Find the element with the most text content
/// 3. Extract text from that element
fn extract_main_text(doc: &Html) -> String {
    // Try common content selectors first
    let content_selectors = [
        "article",
        "main",
        "[role='main']",
        ".content",
        ".post-content",
        ".article-content",
        ".entry-content",
        "#content",
        ".markdown-body",
    ];

    for selector in &content_selectors {
        if let Ok(sel) = Selector::parse(selector) {
            if let Some(el) = doc.select(&sel).next() {
                let text = el.text().collect::<String>();
                if text.split_whitespace().count() > 50 {
                    return clean_text(&text);
                }
            }
        }
    }

    // Fallback: find the paragraph with the most text
    if let Ok(p_selector) = Selector::parse("p") {
        let paragraphs: Vec<String> = doc
            .select(&p_selector)
            .map(|p| p.text().collect::<String>().trim().to_string())
            .filter(|t| t.split_whitespace().count() > 10)
            .collect();

        if !paragraphs.is_empty() {
            return paragraphs.join("\n\n");
        }
    }

    // Last resort: extract all text
    let body_selector = Selector::parse("body").unwrap();
    if let Some(body) = doc.select(&body_selector).next() {
        let text = body.text().collect::<String>();
        return clean_text(&text);
    }

    String::new()
}

/// Clean up extracted text: remove excessive whitespace.
fn clean_text(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_text() {
        let text = "  hello  \n\n  world  \n  ";
        assert_eq!(clean_text(text), "hello\nworld");
    }
}
