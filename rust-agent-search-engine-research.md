# Rust Agent Search Engine — Research & Architecture

## 1. SearXNG 架构分析

### 核心搜索流程

```
SearchWithPlugins.search()
  ├─ pre_search hook (plugins)
  ├─ Search.search()
  │   ├─ search_external_bang()    → !bang 重定向
  │   ├─ search_answerers()        → 即时回答 (calculator, unit_converter...)
  │   └─ search_standard()
  │       ├─ _get_requests()       → 为每个引擎构建请求参数, 计算超时
  │       └─ search_multiple_requests()
  │           └─ 每个引擎一个 threading.Thread → processor.search()
  │               └─ join(remaining_time) 超时则标记 unresponsive
  └─ post_search hook → ResultContainer.close()
```

### 引擎系统

- **引擎定义**: 每个引擎是一个 Python 模块，实现 `request(query, params)` 和 `response(resp)` 函数
- **加载**: 从 `settings.yml` 读取引擎配置，`load_engine()` 动态加载模块
- **引擎属性**: `categories`, `language`, `timeout`, `weight`, `paging`, `safesearch`, `language_support` 等
- **处理器**: `OnlineProcessor`, `OfflineProcessor` 等，封装请求/响应逻辑

### 结果容器 (ResultContainer)

- `main_results_map: dict[int, Result]` — 基于 hash 去重
- `infoboxes`, `suggestions`, `answers`, `corrections`
- **评分**: `score = weight × Σ(weight/position)`，weight 来自引擎配置，position 是结果在引擎中的排名
- **合并**: 相同 URL 的结果合并，记录所有来源引擎

### 插件系统

- `pre_search(request, search)` — 搜索前，可阻止搜索
- `on_result(request, search, result)` — 每个结果，可修改/过滤
- `post_search(request, search)` — 搜索后

### API 接口

`GET /search?q=&format=json&categories=&language=&pageno=&time_range=&safesearch=`

```json
{
  "query": "rust search engine",
  "number_of_results": 10,
  "results": [
    {
      "title": "...",
      "url": "https://...",
      "content": "snippet...",
      "engine": "google",
      "engines": ["google", "bing"],
      "score": 2.5,
      "category": "general",
      "positions": [1, 3],
      "publishedDate": "2024-01-01"
    }
  ],
  "suggestions": [],
  "answers": [],
  "corrections": [],
  "infoboxes": []
}
```

### 关键问题 (Python 实现的瓶颈)

1. **线程模型**: 每个引擎一个 OS 线程，GIL 限制并发，上下文切换开销大
2. **GIL**: CPU 密集型解析/排序无法真正并行
3. **内存占用**: Python 对象开销大
4. **启动慢**: 引擎动态加载，冷启动延迟高
5. **无类型安全**: 引擎接口靠约定，无编译期检查

---

## 2. Websurfx — Rust 元搜索引擎参考

**项目**: https://github.com/neon-mmd/websurfx (1.2k stars, AGPL-3.0)

### 技术栈

| 组件 | 选型 |
|------|------|
| Web 框架 | actix-web |
| 异步运行时 | tokio (multi-thread) |
| HTTP 客户端 | reqwest (rustls, 连接池) |
| HTML 解析 | scraper |
| 并行计算 | rayon |
| 缓存 | moka (内存) / redis |
| 关键词提取 | keyword_extraction (TF-IDF) |
| 停用词 | stop-words |

### 核心架构

```
aggregate(query, page, config, engines, safe_search, user_agent)
  ├─ 共享 reqwest::Client (连接池, 超时, rustls)
  ├─ JoinSet::new()
  ├─ 对每个引擎: tasks.spawn(async { engine.results(...) })
  ├─ while let Some(Ok(response)) = tasks.join_next().await:
  │   ├─ 成功: 遍历 (url, SearchResult), URL 去重 (线性扫描 Vec)
  │   └─ 失败: 记录 EngineErrorInfo
  ├─ safe_search ≥ 3: blocklist/allowlist 正则过滤
  └─ spawn_blocking:
      ├─ par_iter: calculate_relevance (TF-IDF)
      └─ par_sort_unstable_by: 按 relevance_score 降序
```

### 引擎 Trait

```rust
#[async_trait]
pub trait SearchEngine: Sync + Send {
    async fn fetch_html_from_upstream(&self, url, headers, client) -> EngineResult<String>;
    async fn fetch_json_as_bytes_from_upstream(&self, url, headers, client) -> EngineResult<Vec<u8>>;
    async fn results(&self, query, page, user_agent, client, safe_search)
        -> EngineResult<Vec<(String, SearchResult)>>;  // String = URL (去重键)
}
```

### 引擎工厂 (EngineHandler)

```rust
impl EngineHandler {
    pub fn new(engine_name: &str) -> EngineResult<Self> {
        match engine_name {
            "duckduckgo" => ("duckduckgo", Box::new(DuckDuckGo::new()?)),
            "bing" => ("bing", Box::new(Bing::new()?)),
            // ... 11 个引擎
            _ => Err(NoSuchEngineFound),
        }
    }
}
```

### 结果模型

```rust
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
    pub engine: Vec<String>,
    pub relevance_score: f32,
}
```

### 优点

- 异步并发引擎请求 (JoinSet)
- rayon 并行排序/评分
- 连接池复用
- TF-IDF 相关性排序
- 类型安全的引擎 trait

### 不足 (对 agent 场景)

- 去重用线性扫描 Vec (O(n²)，结果多时慢)
- 无 full-text 本地索引
- 无内容抓取 (只有 snippet)
- 无流式输出
- 引擎注册用 match 硬编码 (不易扩展)
- 无引擎级限流/退避

---

## 3. Rust 搜索引擎生态参考

### 3.1 Tantivy — 全文检索库

**项目**: https://github.com/quickwit-oss/tantivy (Apache-2.0)

- Lucene 风格的全文检索库
- 倒排索引 + BM25 评分
- 支持分词、短语查询、模糊查询、范围查询
- 可嵌入，无外部依赖
- **用途**: 本地结果缓存索引、重复查询加速

### 3.2 Meilisearch — 搜索引擎

**项目**: https://github.com/meilisearch/meilisearch (MIT)

- 基于自研存储引擎 (非 Tantivy)
- 排名规则: words → typo → proximity → attribute → sort → wordPosition → exactness
- 分词器: Charabia (多语言支持)
- 内置 typo tolerance
- **用途**: 参考其排名规则设计

### 3.3 Quickwit — 日志/可观测性搜索

**项目**: https://github.com/quickwit-oss/quickwit (AGPL-3.0)

- 基于 Tantivy
- 对象存储 (S3) 分布式搜索
- 列存 + 倒排索引
- **用途**: 参考分布式架构、对象存储设计

### 3.4 Sonic — 轻量搜索后端

**项目**: https://github.com/valeriansaliou/sonic (MPL-2.0)

- schema-less, 模糊匹配
- 超轻量，低内存
- **用途**: 参考轻量索引设计

### 3.5 spider — Rust 网页爬虫

**项目**: https://github.com/spider-rs/spider (MIT)

- 高性能并发爬虫
- 支持 robots.txt、限流、去重
- **用途**: 内容抓取、全文索引构建

---

## 4. Agent 搜索 API 设计

### Agent 需要什么 (对比 Tavily / Exa / Brave)

| 特性 | 说明 |
|------|------|
| 结构化 JSON | `{title, url, snippet, content, published_date, score}` |
| 全文内容抓取 | Agent 需要的不只是 snippet，还要正文 |
| 实时性 | 新鲜数据，减少幻觉 |
| 过滤 | 时间范围、域名、语言 |
| 引用/出处 | 每个结果带 URL，便于 LLM 引用 |
| 低延迟 | Agent 对响应时间敏感 |
| 流式输出 | 渐进式返回结果，减少等待 |
| 批量查询 | 支持多查询并发 |

### 理想的 Agent 搜索 API

```
POST /search
{
  "query": "rust async runtime comparison",
  "max_results": 10,
  "search_depth": "basic" | "advanced",
  "include_answer": false,
  "include_raw_content": false,
  "time_range": "month" | null,
  "domains": ["github.com", "reddit.com"] | null,
  "language": "en" | null
}

Response:
{
  "query": "...",
  "results": [
    {
      "title": "...",
      "url": "https://...",
      "snippet": "...",
      "content": null,  // 全文, 仅当 include_raw_content=true
      "published_date": "2024-01-01T00:00:00Z",
      "score": 0.85,
      "engine": "google"
    }
  ],
  "answer": null  // AI 生成的摘要, 仅当 include_answer=true
}
```

```
POST /search/stream  (SSE)
  → 每个结果一条 event: data: {title, url, snippet, ...}

GET /content?url=https://...
  → { url, title, content, extracted_at }
```

---

## 5. 推荐架构设计

### 设计原则

1. **Agent-first**: 无 HTML UI，纯 JSON API，结构化输出
2. **异步优先**: tokio + JoinSet 并发引擎请求
3. **类型安全**: trait 定义引擎接口，编译期检查
4. **高性能**: 连接池、并行排序、高效去重
5. **可扩展**: 引擎注册用注册表而非硬编码 match

### 技术栈

| 组件 | 推荐 | 理由 |
|------|------|------|
| Web 框架 | axum | 更现代，tower 生态，异步原生 |
| HTTP 客户端 | reqwest (rustls) | 成熟，连接池，超时 |
| HTML 解析 | tl (or scraper) | tl 更快 (零拷贝) |
| 全文索引 | tantivy | BM25，本地缓存加速 |
| 爬虫 | spider | 内容抓取 |
| 缓存 | moka + redis | 内存 + 分布式 |
| 序列化 | serde + serde_json | 标准 |
| 异步 | tokio | 标准 |
| 并行 | rayon | CPU 密集任务 |
| 配置 | serde + toml | 引擎配置 |

### 核心模块

```
src/
├── main.rs
├── lib.rs
├── config.rs           # 配置加载 (TOML)
├── engine/
│   ├── mod.rs          # Engine trait, EngineRegistry
│   ├── trait.rs        # SearchEngine trait
│   ├── html.rs         # HTML 搜索引擎基类
│   ├── json.rs         # JSON API 引擎基类
│   └── engines/
│       ├── google.rs
│       ├── bing.rs
│       ├── duckduckgo.rs
│       └── ...
├── aggregator.rs       # 搜索聚合 (并发, 去重, 排序)
├── ranking.rs          # 相关性评分 (BM25/TF-IDF)
├── dedup.rs            # 去重 (URL 归一化 + 内容 hash)
├── cache/
│   ├── mod.rs
│   ├── memory.rs       # moka
│   └── redis.rs
├── fetcher.rs          # 全文内容抓取 (spider)
├── index/
│   ├── mod.rs          # tantivy 本地索引
│   └── schema.rs
├── models/
│   ├── result.rs       # SearchResult
│   ├── query.rs        # SearchQuery
│   └── error.rs
├── routes/
│   ├── search.rs       # POST /search
│   ├── stream.rs       # POST /search/stream
│   ├── content.rs      # GET /content
│   └── engines.rs      # GET /engines
└── middleware/
    ├── rate_limit.rs   # 引擎级限流
    └── proxy.rs        # 代理轮换
```

### 引擎 Trait 设计

```rust
#[async_trait]
pub trait SearchEngine: Send + Sync {
    fn name(&self) -> &'static str;
    fn categories(&self) -> &[&'static str];

    async fn search(&self, query: &SearchQuery) -> Result<Vec<RawSearchResult>, EngineError>;

    // 可选: 健康检查
    async fn health_check(&self) -> bool { true }
}

pub struct RawSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub published_date: Option<DateTime<Utc>>,
    pub position: u32,  // 引擎内排名
}
```

### 聚合流程

```rust
pub async fn aggregate(query: &SearchQuery, engines: &[EngineRef]) -> SearchResults {
    let mut tasks = JoinSet::new();

    for engine in engines {
        let q = query.clone();
        let e = engine.clone();
        tasks.spawn(async move {
            tokio::select! {
                res = e.search(&q) => (e.name(), res),
                _ = tokio::time::sleep(Duration::from_secs(e.timeout())) => (e.name(), Err(EngineError::Timeout)),
            }
        });
    }

    let mut dedup = DedupMap::new();  // HashMap<NormalizedUrl, SearchResult>
    let mut errors = Vec::new();

    while let Some(Ok((name, result))) = tasks.join_next().await {
        match result {
            Ok(results) => {
                for raw in results {
                    dedup.insert_or_merge(raw, name);  // URL 归一化去重
                }
            }
            Err(e) => errors.push(EngineErrorInfo::new(name, e)),
        }
    }

    // 并行评分 + 排序
    let mut scored: Vec<_> = dedup.into_values().collect();
    scored.par_iter_mut().for_each(|r| r.calculate_score(query));
    scored.par_sort_unstable_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    SearchResults { results: scored.into_boxed_slice(), errors }
}
```

### 去重策略

1. **URL 归一化**: 去除 utm_* 参数、统一大小写、去尾斜杠
2. **内容 hash**: 对 title+snippet 做 simhash，检测近似重复
3. **域名权重**: 同一域名最多 N 条结果

### 评分公式

```
score = engine_weight × position_weight × bm25_score × freshness_weight
```

- `engine_weight`: 引擎可信度权重
- `position_weight`: 1 / position (排名越前越高)
- `bm25_score`: query 对 title+url+snippet 的 BM25 得分
- `freshness_weight`: 越新越高 (可选)

### 缓存策略

- **查询缓存**: `(query, engines, params) → results`，TTL 5-60s
- **引擎数据缓存**: token、cookie 等 (参考 SearXNG EngineCache)
- **内容缓存**: 抓取的页面内容，TTL 较长

### 引擎级限流

```rust
struct EngineRateLimiter {
    semaphore: Semaphore,  // 并发限制
    last_request: AtomicInstant,
    min_interval: Duration,
}
```

---

## 6. 与 Websurfx 的关键差异

| 维度 | Websurfx | 本设计 (Agent 优化) |
|------|----------|---------------------|
| 去重 | Vec 线性扫描 O(n²) | HashMap + URL 归一化 O(n) |
| 评分 | TF-IDF | BM25 (Tantivy) + 位置 + 引擎权重 |
| 内容 | 仅 snippet | 可选全文抓取 (spider) |
| 输出 | HTML + JSON | 纯 JSON (agent-first) |
| 流式 | 无 | SSE 流式输出 |
| 引擎注册 | match 硬编码 | 注册表 (可动态加载) |
| 限流 | 无 | 引擎级令牌桶 |
| 本地索引 | 无 | Tantivy (缓存热门查询) |
| 代理 | 单一代理 | 代理池轮换 |

---

## 7. 实施路线

### Phase 1: MVP (1-2 周)

- [ ] axum HTTP server
- [ ] SearchEngine trait + 3 个引擎 (DuckDuckGo, Bing, Brave)
- [ ] 聚合器 (JoinSet 并发, HashMap 去重)
- [ ] TF-IDF 评分
- [ ] JSON API: POST /search
- [ ] moka 查询缓存

### Phase 2: Agent 优化 (1-2 周)

- [ ] 全文内容抓取 (spider)
- [ ] SSE 流式输出
- [ ] BM25 评分 (Tantivy)
- [ ] 引擎级限流
- [ ] URL 归一化去重

### Phase 3: 生产级 (2-4 周)

- [ ] Tantivy 本地索引
- [ ] 代理池
- [ ] redis 分布式缓存
- [ ] 健康检查 + 引擎自动暂停
- [ ] 更多引擎 (10+)
- [ ] 监控指标

---

## 8. 参考项目汇总

| 项目 | 语言 | 类型 | 参考价值 |
|------|------|------|----------|
| [SearXNG](https://github.com/searxng/searxng) | Python | 元搜索 | 引擎系统、插件、结果合并 |
| [Websurfx](https://github.com/neon-mmd/websurfx) | Rust | 元搜索 | Rust 元搜索架构、trait 设计 |
| [Tantivy](https://github.com/quickwit-oss/tantivy) | Rust | 全文检索库 | BM25、倒排索引 |
| [Meilisearch](https://github.com/meilisearch/meilisearch) | Rust | 搜索引擎 | 排名规则、分词 |
| [Quickwit](https://github.com/quickwit-oss/quickwit) | Rust | 分布式搜索 | 对象存储、分布式 |
| [spider](https://github.com/spider-rs/spider) | Rust | 爬虫 | 内容抓取 |
| [Sonic](https://github.com/valeriansaliou/sonic) | Rust | 轻量搜索 | 轻量索引 |
