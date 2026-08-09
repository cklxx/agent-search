//! Web crawler: fetch pages and extract main content.

use ego_tree::NodeRef;
use reqwest::Client;
use scraper::{ElementRef, Html, Node, Selector};

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
///
/// Uses a readability-style scoring algorithm:
/// 1. Remove noise elements (script, style, nav, header, footer, ...).
/// 2. Score each candidate block (div, section, article, main, p) by text
///    length, punctuation count, link count, and link density.
/// 3. Pick the highest-scoring block and extract its text with paragraph
///    and code-block structure preserved.
pub fn extract_content(html: &str) -> (String, String) {
    let mut doc = Html::parse_document(html);

    let title = doc
        .select(&Selector::parse("title").unwrap())
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .unwrap_or_default();

    remove_noise(&mut doc);

    let content = find_best_block(&doc)
        .map(|el| extract_structured_text(&el))
        .unwrap_or_default();

    let content = normalize_final(&content);
    (title, content)
}

/// Detach noise elements from the DOM tree.
///
/// Uses tree mutation rather than string replacement so nested and
/// self-closing variants are handled correctly.
fn remove_noise(doc: &mut Html) {
    let noise_sel = Selector::parse(
        "script, style, noscript, nav, header, footer, aside, form, iframe, svg, canvas",
    )
    .unwrap();

    // Collect node IDs under an immutable borrow, then detach under a mutable one.
    let noise_ids: Vec<_> = doc.select(&noise_sel).map(|el| el.id()).collect();
    for id in noise_ids {
        if let Some(mut node) = doc.tree.get_mut(id) {
            node.detach();
        }
    }
}

/// Score a candidate block for content likelihood.
///
/// Positive signals: text length, punctuation count.
/// Negative signals: link count, link density (link text / total text).
fn score_block(el: &ElementRef) -> f64 {
    let text: String = el.text().collect();
    let text_len = text.chars().count();
    if text_len == 0 {
        return 0.0;
    }

    let punctuation = text
        .chars()
        .filter(|c| matches!(c, '.' | ',' | '!' | '?' | ';' | ':'))
        .count() as f64;

    let link_sel = Selector::parse("a").unwrap();
    let mut link_text_len = 0usize;
    let mut link_count = 0usize;
    for link in el.select(&link_sel) {
        link_text_len += link.text().collect::<String>().chars().count();
        link_count += 1;
    }

    let link_density = link_text_len as f64 / text_len as f64;
    let text_len = text_len as f64;

    // text_len * (1 - link_density) rewards non-link text;
    // punctuation bonus; link count penalty.
    text_len * (1.0 - link_density) + punctuation * 10.0 - link_count as f64 * 10.0
}

/// Return the candidate block with the highest content score.
fn find_best_block<'a>(doc: &'a Html) -> Option<ElementRef<'a>> {
    let candidate_sel = Selector::parse("div, section, article, main, p").unwrap();

    let mut best: Option<(ElementRef<'a>, f64)> = None;
    for el in doc.select(&candidate_sel) {
        let score = score_block(&el);
        match best {
            Some((_, best_score)) if score <= best_score => {}
            _ => best = Some((el, score)),
        }
    }
    best.map(|(el, _)| el)
}

/// Extract text from an element, preserving paragraph breaks and code blocks.
fn extract_structured_text(el: &ElementRef) -> String {
    let mut result = String::new();
    for child in el.children() {
        walk_node(child, &mut result);
    }
    result
}

/// Recursively walk a node, appending text to `result`.
///
/// Block elements get newlines before and after; `<pre>` content keeps its
/// internal whitespace; inline text is whitespace-normalized.
fn walk_node(node: NodeRef<Node>, result: &mut String) {
    match node.value() {
        Node::Text(text) => {
            let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !normalized.is_empty() {
                result.push_str(&normalized);
            }
        }
        Node::Element(element) => {
            let name = element.name();
            match name {
                "pre" => {
                    // Preserve internal whitespace; trim only surrounding newlines.
                    let pre_text: String = ElementRef::wrap(node).unwrap().text().collect();
                    let pre_text = pre_text.trim_matches(|c: char| c == '\n' || c == '\r');
                    result.push_str(pre_text);
                    result.push('\n');
                }
                "br" => result.push('\n'),
                "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "blockquote"
                | "div" | "section" | "article" | "main" | "tr" | "ul" | "ol"
                | "table" | "thead" | "tbody" => {
                    if !result.is_empty() && !result.ends_with('\n') {
                        result.push('\n');
                    }
                    for child in node.children() {
                        walk_node(child, result);
                    }
                    if !result.ends_with('\n') {
                        result.push('\n');
                    }
                }
                _ => {
                    for child in node.children() {
                        walk_node(child, result);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Collapse consecutive newlines and trim the whole string.
///
/// Horizontal whitespace is left untouched so `<pre>` formatting survives.
fn normalize_final(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_newline = false;
    for c in s.chars() {
        if c == '\n' {
            if !prev_newline {
                out.push('\n');
                prev_newline = true;
            }
        } else {
            out.push(c);
            prev_newline = false;
        }
    }
    out.trim().to_string()
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

    #[test]
    fn test_sidebar_with_many_links_loses() {
        let html = r##"
        <html><body>
            <div class="sidebar">
                <a href="#">Home</a>
                <a href="#">About</a>
                <a href="#">Contact</a>
                <a href="#">Products</a>
                <a href="#">Services</a>
                <a href="#">Blog</a>
                <a href="#">FAQ</a>
                <a href="#">Support</a>
            </div>
            <div class="article">
                <p>This is the main article content. It has several sentences with punctuation.
                It describes a topic in detail and provides useful information to the reader.</p>
            </div>
        </body></html>
        "##;
        let (_title, content) = extract_content(html);
        assert!(content.contains("main article content"));
        assert!(!content.contains("Products"));
    }

    #[test]
    fn test_preserves_paragraph_breaks() {
        let html = r#"
        <article>
            <p>First paragraph.</p>
            <p>Second paragraph.</p>
        </article>
        "#;
        let (_title, content) = extract_content(html);
        assert!(content.contains("First paragraph."));
        assert!(content.contains("Second paragraph."));
        assert!(content.contains("First paragraph.\nSecond paragraph."));
    }

    #[test]
    fn test_preserves_code_block() {
        let html = r#"
        <article>
            <p>Here is some code:</p>
            <pre><code>fn main() {
    println!("hello");
}</code></pre>
        </article>
        "#;
        let (_title, content) = extract_content(html);
        assert!(content.contains("fn main()"));
        assert!(content.contains("println!(\"hello\")"));
        // Internal indentation inside <pre> must survive.
        assert!(content.contains("    println!"));
    }

    #[test]
    fn test_noise_tags_removed() {
        let html = r#"
        <html><body>
            <script>var x = 1;</script>
            <style>body { color: red; }</style>
            <nav>navigation</nav>
            <article><p>Real content here.</p></article>
            <footer>footer text</footer>
        </body></html>
        "#;
        let (_title, content) = extract_content(html);
        assert!(content.contains("Real content here"));
        assert!(!content.contains("navigation"));
        assert!(!content.contains("footer text"));
        assert!(!content.contains("var x"));
    }
}
