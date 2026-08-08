//! Search query model.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,

    #[serde(default = "default_max_results")]
    pub max_results: usize,

    /// "day", "week", "month", "year", or null.
    #[serde(default)]
    pub time_range: Option<String>,

    /// e.g. "en", "zh".
    #[serde(default)]
    pub language: Option<String>,

    /// 0-indexed.
    #[serde(default)]
    pub page: u32,

    /// 0 = off, 1 = moderate, 2 = strict.
    #[serde(default)]
    pub safe_search: u8,

    /// Optional category hint. If absent, inferred from query keywords.
    #[serde(default)]
    pub category: Option<String>,
}

fn default_max_results() -> usize {
    10
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            max_results: default_max_results(),
            time_range: None,
            language: None,
            page: 0,
            safe_search: 0,
            category: None,
        }
    }
}

/// Infer relevant engine categories from query keywords.
/// Returns categories to search in addition to "general".
pub fn infer_categories(query: &str) -> Vec<&'static str> {
    let q = query.to_lowercase();
    let mut cats = Vec::new();

    // Academic / scientific publications
    if q.contains("arxiv")
        || q.contains("paper")
        || q.contains("publication")
        || q.contains("citation")
        || q.contains("doi")
        || q.contains("research")
        || q.contains("study")
        || q.contains("scholar")
        || q.contains("theorem")
        || q.contains("proof")
        || q.contains("hypothesis")
        || q.contains("experiment")
        || q.contains("peer review")
        || q.contains("journal")
        || q.contains("conference")
    {
        cats.push("science");
        cats.push("scientific publications");
    }

    // IT / programming / systems
    if q.contains("rust")
        || q.contains("python")
        || q.contains("javascript")
        || q.contains("typescript")
        || q.contains("golang")
        || q.contains("java")
        || q.contains("c++")
        || q.contains("cpp")
        || q.contains("error")
        || q.contains("bug")
        || q.contains("compile")
        || q.contains("stack")
        || q.contains("api")
        || q.contains("function")
        || q.contains("class")
        || q.contains("variable")
        || q.contains("code")
        || q.contains("programming")
        || q.contains("algorithm")
        || q.contains("database")
        || q.contains("sql")
        // LLM / AI
        || q.contains("transformer")
        || q.contains("attention")
        || q.contains("bert")
        || q.contains("gpt")
        || q.contains("llm")
        || q.contains("diffusion")
        || q.contains("embedding")
        || q.contains("neural")
        || q.contains("deep learning")
        || q.contains("machine learning")
        || q.contains("reinforcement")
        || q.contains("fine-tun")
        || q.contains("pretrain")
        || q.contains("tokeniz")
        // Systems / infra
        || q.contains("docker")
        || q.contains("kubernetes")
        || q.contains("k8s")
        || q.contains("tcp")
        || q.contains("http")
        || q.contains("nginx")
        || q.contains("systemd")
        || q.contains("linux")
        || q.contains("kernel")
        || q.contains("distributed")
        || q.contains("concurrency")
        || q.contains("async")
        || q.contains("network")
        || q.contains("protocol")
        || q.contains("cache")
        || q.contains("index")
    {
        cats.push("it");
    }

    // Code repositories / packages
    if q.contains("github")
        || q.contains("git")
        || q.contains("crate")
        || q.contains("npm")
        || q.contains("pypi")
        || q.contains("pip")
        || q.contains("package")
        || q.contains("library")
        || q.contains("dependency")
        || q.contains("repo")
        || q.contains("framework")
        || q.contains("sdk")
    {
        cats.push("repos");
        cats.push("packages");
    }

    // News
    if q.contains("news") || q.contains("announce") || q.contains("release") {
        cats.push("news");
    }

    cats
}
