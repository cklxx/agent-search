//! Search aggregation: fan out, dedup, score, sort.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;

use crate::dedup::normalize_url;
use crate::engine::{EngineRef, EngineRegistry, EngineSuspensionManager};
use crate::models::error::{EngineResult, SearchError};
use crate::models::query::{SearchQuery, infer_categories};
use crate::models::result::{EngineErrorInfo, RawSearchResult, SearchResponse, SearchResult};
use crate::ranking::RankingStrategy;

/// Fan out to non-suspended engines, dedup by URL, score with `strategy`, sort.
pub async fn aggregate(
    query: &SearchQuery,
    registry: &EngineRegistry,
    suspension: &EngineSuspensionManager,
    strategy: Arc<dyn RankingStrategy>,
) -> EngineResult<SearchResponse> {
    let (dedup_map, errors) = fetch_raw_results(query, registry, suspension).await;

    if dedup_map.is_empty() && !errors.is_empty() {
        return Err(SearchError::Request("all engines failed".to_string()));
    }

    // Score on a blocking thread: cross-encoder reranking is CPU-heavy and
    // must not block the tokio runtime.
    let query_clone = query.clone();
    let results = tokio::task::spawn_blocking(move || {
        score_results(dedup_map, &query_clone, strategy.as_ref())
    })
    .await
    .map_err(|e| SearchError::Request(format!("scoring task panicked: {}", e)))?;

    Ok(SearchResponse {
        query: query.query.clone(),
        results,
        errors,
    })
}

/// Fan out to engines and dedup by URL. Returns (url -> (raw, engines, max_weight), errors).
/// Shared between `aggregate` and A/B comparison to avoid double upstream calls.
pub async fn fetch_raw_results(
    query: &SearchQuery,
    registry: &EngineRegistry,
    suspension: &EngineSuspensionManager,
) -> (
    HashMap<String, (RawSearchResult, Vec<String>, f32)>,
    Vec<EngineErrorInfo>,
) {
    let engines = select_engines(query, registry, suspension);

    if engines.is_empty() {
        return (
            HashMap::new(),
            vec![EngineErrorInfo {
                engine: "system".to_string(),
                error: "no engines available".to_string(),
            }],
        );
    }

    let weights: HashMap<String, f32> = engines
        .iter()
        .map(|e| (e.name().to_string(), e.weight()))
        .collect();

    let mut tasks: JoinSet<(String, EngineResult<Vec<RawSearchResult>>)> = JoinSet::new();

    for engine in &engines {
        let engine = engine.clone();
        let q = query.clone();
        tasks.spawn(async move {
            let name = engine.name().to_string();
            let timeout = Duration::from_secs(engine.timeout());
            let result = tokio::time::timeout(timeout, engine.search(&q)).await;
            match result {
                Ok(res) => (name, res),
                Err(_) => (name, Err(SearchError::Timeout)),
            }
        });
    }

    let mut dedup_map: HashMap<String, (RawSearchResult, Vec<String>, f32)> = HashMap::new();
    let mut errors: Vec<EngineErrorInfo> = Vec::new();

    while let Some(Ok((engine_name, result))) = tasks.join_next().await {
        match result {
            Ok(raw_results) => {
                suspension.record_success(&engine_name);
                let engine_weight = *weights.get(&engine_name).unwrap_or(&1.0);
                for raw in raw_results {
                    let key = normalize_url(&raw.url);
                    match dedup_map.get_mut(&key) {
                        Some((_, engines, weight)) => {
                            engines.push(engine_name.clone());
                            *weight = weight.max(engine_weight);
                        }
                        None => {
                            dedup_map.insert(key, (raw, vec![engine_name.clone()], engine_weight));
                        }
                    }
                }
            }
            Err(e) => {
                let suspended = suspension.record_error(&engine_name, &e);
                let error_msg = if let Some(dur) = suspended {
                    format!("{} (suspended for {}s)", e, dur.as_secs())
                } else {
                    e.to_string()
                };
                tracing::warn!(engine = %engine_name, error = %e, "engine error");
                errors.push(EngineErrorInfo {
                    engine: engine_name,
                    error: error_msg,
                });
            }
        }
    }

    (dedup_map, errors)
}

/// Score and sort deduped raw results with a given strategy.
pub fn score_results(
    dedup_map: HashMap<String, (RawSearchResult, Vec<String>, f32)>,
    query: &SearchQuery,
    strategy: &dyn RankingStrategy,
) -> Vec<SearchResult> {
    let items: Vec<(RawSearchResult, Vec<String>, f32)> = dedup_map.into_values().collect();
    let scores = strategy.score_batch(&items, query);

    let mut results: Vec<SearchResult> = items
        .into_iter()
        .zip(scores)
        .map(|((raw, engines, _weight), score)| SearchResult {
            title: raw.title,
            url: raw.url,
            snippet: raw.snippet,
            published_date: raw.published_date,
            score,
            engines,
        })
        .collect();

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(query.max_results);
    results
}

fn select_engines(
    query: &SearchQuery,
    registry: &EngineRegistry,
    suspension: &EngineSuspensionManager,
) -> Vec<EngineRef> {
    // Always include "general" engines; add inferred or specified categories.
    let mut categories = vec!["general".to_string()];
    if let Some(ref cat) = query.category {
        categories.push(cat.clone());
    } else {
        for cat in infer_categories(&query.query) {
            categories.push(cat.to_string());
        }
    }

    let mut seen = std::collections::HashSet::new();
    let mut engines = Vec::new();
    for category in &categories {
        for engine in registry.by_category(category) {
            if !suspension.is_suspended(engine.name()) && seen.insert(engine.name().to_string()) {
                engines.push(engine);
            }
        }
    }
    engines
}

