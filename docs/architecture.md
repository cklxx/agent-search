# 搜索引擎全链路架构

## 请求链路（同步）

```
用户查询
   │
   ▼
┌─────────────────────────────────────────────┐
│ 1. QueryCache (moka)                         │
│    精确匹配 query:page:max_results            │
└──────────────┬──────────────────────────────┘
               │ miss
               ▼
┌─────────────────────────────────────────────┐
│ 2. 本地精确查询缓存 (search_cached)           │
│    Tantivy STRING 字段精确匹配 query          │
└──────────────┬──────────────────────────────┘
               │ miss
               ▼
┌─────────────────────────────────────────────┐
│ 3. 本地全文索引 (search_fulltext)            │
│    Tantivy BM25 over title + content         │
│    CJK 2-gram 分词                            │
│    命中 ≥3 条则直接返回                        │
└──────────────┬──────────────────────────────┘
               │ miss
               ▼
┌─────────────────────────────────────────────┐
│ 4. 召回 (Recall)                             │
│    ┌──────────────────────────────────────┐  │
│    │ 外部引擎聚合 (fetch_raw_results)      │  │
│    │  - SearXNG (general)                 │  │
│    │  - 垂直引擎 (GitHub/StackExchange/…) │  │
│    │  - 域名级限流 (DomainRateLimiter)    │  │
│    │  - 全局并发限流 (Semaphore)          │  │
│    └──────────────────────────────────────┘  │
└──────────────┬──────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────┐
│ 5. 去重 (DedupService)                       │
│    normalize_url: 小写 host / 去跟踪参数 /    │
│    去尾斜杠 / 解包 archive 快照               │
└──────────────┬──────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────┐
│ 6. 粗排 (Coarse Ranking)                     │
│    BM25F (title×3 + url×1.5 + snippet×1)    │
│    × 位置衰减 (1/log2(pos+1))                │
│    × 引擎权重                                │
│    × 域名权威度 (白名单/黑名单)               │
│    × 时效性 (exp(-age/180d))                 │
└──────────────┬──────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────┐
│ 7. 精排 (Fine Ranking)                       │
│    bge_reranker (jina-reranker-v2)           │
│    cross-encoder, 512 tokens                 │
│    取粗排 top-50 送入精排                     │
│    score = authority² × relevance × coverage │
└──────────────┬──────────────────────────────┘
               │
               ▼
          返回结果 (max_results)
```

## 建库链路（异步）

```
搜索返回 top-5 结果
   │
   ▼
┌─────────────────────────────────────────────┐
│ 爬虫 (crawler::fetch_and_extract)            │
│  - HTTP GET (浏览器 UA)                       │
│  - 正文提取 (readability 评分)                │
│    · 移除 script/style/nav/header/footer      │
│    · 评分: text_len×(1-link_density)          │
│           + punctuation×10 - link_count×10   │
│    · 保留段落换行和 <pre> 代码块              │
└──────────────┬──────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────┐
│ 去重 (DedupService.insert)                   │
│    已索引的 URL 跳过                          │
└──────────────┬──────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────┐
│ 建库 (LocalIndex::index_page)                │
│    Tantivy 索引                              │
│    - title: CJK 2-gram TEXT                  │
│    - content: CJK 2-gram TEXT                │
│    - url: TEXT STORED                        │
│    - engine: "local"                         │
└─────────────────────────────────────────────┘
```

## 关键组件

| 组件 | 文件 | 职责 |
|------|------|------|
| QueryCache | `src/cache.rs` | 精确查询缓存 (moka, TTL) |
| LocalIndex | `src/index.rs` | Tantivy 全文索引 + 精确查询缓存 |
| DedupService | `src/dedup.rs` | URL 规范化 + 去重 |
| DomainRateLimiter | `src/engine/domain_rate_limit.rs` | 域名级并发限流 |
| EngineSuspensionManager | `src/engine/suspension.rs` | 引擎错误退避 |
| aggregator | `src/aggregator.rs` | 召回 + 去重 + 粗排 |
| RankingStrategy | `src/ranking.rs` | 粗排 (BM25) + 精排 (bge_reranker) |
| crawler | `src/crawler.rs` | 页面爬取 + 正文提取 |

## 排序公式

### 粗排 (BM25)
```
score = normalize(
    bm25f × position_weight × engine_weight × authority × freshness
)

bm25f = Σ term [ tf × (k1+1) / (tf + k1) ] × (0.5 + 0.5×coverage)
tf = tf_title×3 + tf_url×1.5 + tf_snippet×1
position_weight = 1 / log2(pos+1)
freshness = 0.5 ^ (age_days / 180)
```

### 精排 (bge_reranker)
```
score = clamp(authority² × sigmoid(cross_encoder_score) × coverage, 0, 1)
coverage = matched_query_terms / total_query_terms
```
