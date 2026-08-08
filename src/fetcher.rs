//! Fetch web pages and extract main text for LLM consumption.

use reqwest::Client;
use scraper::{Html, Selector};

#[derive(Debug, Clone)]
pub struct FetchedContent {
    pub url: String,
    pub title: String,
    pub content: String,
    pub fetched_at: chrono::DateTime<chrono::Utc>,
}

pub async fn fetch_content(url: &str) -> Result<FetchedContent, reqwest::Error> {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()?;

    let resp = client.get(url).send().await?;
    let body = resp.text().await?;

    let doc = Html::parse_document(&body);

    let title_selector = Selector::parse("title").unwrap();
    let title = doc
        .select(&title_selector)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .unwrap_or_default();

    let content = extract_main_text(&doc);

    Ok(FetchedContent {
        url: url.to_string(),
        title,
        content,
        fetched_at: chrono::Utc::now(),
    })
}

/// Extract main text: try article/main selectors, then paragraphs, then body.
fn extract_main_text(doc: &Html) -> String {
    let content_selectors = [
        "article", "main", "[role='main']", ".content", ".post-content",
        ".article-content", ".entry-content", "#content", ".markdown-body",
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

    let body_selector = Selector::parse("body").unwrap();
    if let Some(body) = doc.select(&body_selector).next() {
        return clean_text(&body.text().collect::<String>());
    }

    String::new()
}

fn clean_text(text: &str) -> String {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
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
