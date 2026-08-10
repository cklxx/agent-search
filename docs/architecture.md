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
│ 2. 本地全文索引 (search_fulltext)            │
│    Tantivy BM25 over title + content         │
│    混合分词: ASCII 按词, CJK 2-gram           │
│    AND 查询 (所有词必须匹配)                  │
└──────────────┬──────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────┐
│ 3. 召回 (Recall)                             │
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
│ 4. 合并 + 去重                               │
│    本地全文结果 ⊕ 外部引擎结果                │
│    normalize_url 去重 (小写 host / 去跟踪参数 │
│    / 去尾斜杠 / 解包 archive 快照)            │
└──────────────┬──────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────┐
│ 5. 统一打分 (RankingStrategy)                │
│    粗排 BM25F (title×3 + url×1.5 + snippet×1)│
│    × 位置衰减 × 引擎权重 × 域名权威度 × 时效性│
│    精排 bge_reranker (top-50 cross-encoder)  │
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
│    - title: 混合分词 TEXT (ASCII 词 + CJK 2-gram) │
│    - content: 混合分词 TEXT                  │
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
