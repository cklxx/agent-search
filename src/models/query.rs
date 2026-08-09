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

    /// Trace ID for distributed tracing. Not deserialized from request body;
    /// populated by the trace middleware from the x-trace-id header.
    #[serde(skip)]
    pub trace_id: Option<String>,
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
            trace_id: None,
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
        || q.contains("quantum")
        || q.contains("physics")
        || q.contains("chemistry")
        || q.contains("biology")
        || q.contains("genome")
        || q.contains("molecule")
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

    // Programming (languages, code) — routes to stackoverflow, softwareengineering
    if q.contains("rust")
        || q.contains("python")
        || q.contains("javascript")
        || q.contains("typescript")
        || q.contains("golang")
        || q.contains("java")
        || q.contains("c++")
        || q.contains("cpp")
        || q.contains("code")
        || q.contains("programming")
        || q.contains("function")
        || q.contains("class")
        || q.contains("variable")
        || q.contains("compile")
        || q.contains("bug")
        || q.contains("error")
    {
        cats.push("programming");
    }

    // Security — routes to security_stackexchange
    if q.contains("security")
        || q.contains("exploit")
        || q.contains("vulnerability")
        || q.contains("cve")
        || q.contains("malware")
        || q.contains("ransomware")
        || q.contains("penetration")
        || q.contains("pentest")
        || q.contains("firewall")
        || q.contains("encryption")
        || q.contains("cryptography")
        || q.contains("authentication")
        || q.contains("authorization")
        || q.contains("oauth")
        || q.contains("jwt")
        || q.contains("injection")
        || q.contains("xss")
        || q.contains("csrf")
    {
        cats.push("security");
    }

    // Databases — routes to dba_stackexchange
    if q.contains("database")
        || q.contains("sql")
        || q.contains("mysql")
        || q.contains("postgresql")
        || q.contains("postgres")
        || q.contains("mongodb")
        || q.contains("redis")
        || q.contains("nosql")
        || q.contains("orm")
        || q.contains("transaction")
        || q.contains("index")
        || q.contains("query")
    {
        cats.push("databases");
    }

    // Linux / Unix — routes to askubuntu, unix_stackexchange
    if q.contains("linux")
        || q.contains("unix")
        || q.contains("ubuntu")
        || q.contains("debian")
        || q.contains("kernel")
        || q.contains("systemd")
        || q.contains("bash")
        || q.contains("shell")
        || q.contains("command line")
        || q.contains("terminal")
    {
        cats.push("linux");
    }

    // DevOps — routes to devops_stackexchange
    if q.contains("docker")
        || q.contains("kubernetes")
        || q.contains("k8s")
        || q.contains("devops")
        || q.contains("ci/cd")
        || q.contains("ci")
        || q.contains("cd")
        || q.contains("jenkins")
        || q.contains("terraform")
        || q.contains("ansible")
        || q.contains("container")
    {
        cats.push("devops");
    }

    // Sysadmin — routes to serverfault
    if q.contains("server")
        || q.contains("nginx")
        || q.contains("apache")
        || q.contains("dns")
        || q.contains("load balanc")
        || q.contains("monitoring")
        || q.contains("backup")
        || q.contains("sysadmin")
    {
        cats.push("sysadmin");
    }

    // AI / LLM — routes to ai_stackexchange
    if q.contains("ai")
        || q.contains("llm")
        || q.contains("gpt")
        || q.contains("bert")
        || q.contains("transformer")
        || q.contains("neural")
        || q.contains("deep learning")
        || q.contains("machine learning")
        || q.contains("reinforcement")
        || q.contains("embedding")
        || q.contains("diffusion")
        || q.contains("tokeniz")
        || q.contains("fine-tun")
        || q.contains("pretrain")
        || q.contains("attention")
    {
        cats.push("ai");
    }

    // Data science — routes to datascience_stackexchange
    if q.contains("data science")
        || q.contains("statistics")
        || q.contains("machine learning")
        || q.contains("deep learning")
        || q.contains("pandas")
        || q.contains("numpy")
        || q.contains("scikit")
        || q.contains("regression")
        || q.contains("classification")
        || q.contains("clustering")
    {
        cats.push("datascience");
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

    // Movies / TV
    if q.contains("movie")
        || q.contains("film")
        || q.contains("tv show")
        || q.contains("tv series")
        || q.contains("actor")
        || q.contains("actress")
        || q.contains("director")
        || q.contains("imdb")
        || q.contains("netflix")
    {
        cats.push("movies");
    }

    // Videos
    if q.contains("video")
        || q.contains("youtube")
        || q.contains("vimeo")
        || q.contains("dailymotion")
        || q.contains("twitch")
        || q.contains("stream")
        || q.contains("tutorial")
    {
        cats.push("videos");
    }

    // Shopping
    if q.contains("buy")
        || q.contains("shop")
        || q.contains("shopping")
        || q.contains("price")
        || q.contains("amazon")
        || q.contains("ebay")
        || q.contains("product")
        || q.contains("review")
    {
        cats.push("shopping");
    }

    // Jobs
    if q.contains("job")
        || q.contains("jobs")
        || q.contains("career")
        || q.contains("salary")
        || q.contains("resume")
        || q.contains("interview")
        || q.contains("hiring")
        || q.contains("indeed")
        || q.contains("glassdoor")
    {
        cats.push("jobs");
    }

    // Social media
    if q.contains("social media")
        || q.contains("twitter")
        || q.contains("x.com")
        || q.contains("facebook")
        || q.contains("instagram")
        || q.contains("linkedin")
        || q.contains("pinterest")
        || q.contains("tweet")
        || q.contains("post")
    {
        cats.push("social media");
    }

    cats
}
