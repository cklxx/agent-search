//! Search aggregation: fan out queries to engines, dedup, score, sort.

use std::collections::HashMap;
use std::time::Duration;

use tokio::task::JoinSet;

use crate::dedup::normalize_url;
use crate::engine::{EngineRef, EngineRegistry, EngineSuspensionManager};
use crate::models::error::{EngineResult, SearchError};
use crate::models::query::SearchQuery;
use crate::models::result::{EngineErrorInfo, RawSearchResult, SearchResponse, SearchResult};
use crate::ranking::score_result;

/// Aggregate search results from multiple engines.
///
/// 1. Skips suspended engines
/// 2. Spawns an async task per engine via JoinSet
/// 3. Collects results, deduplicates by normalized URL
/// 4. Scores each result (TF + position + engine weight)
/// 5. Sorts by score descending
/// 6. Records success/error for suspension tracking
pub async fn aggregate(
    query: &SearchQuery,
    registry: &EngineRegistry,
    suspension: &EngineSuspensionManager,
) -> EngineResult<SearchResponse> {
    let engines = select_engines(query, registry, suspension);

    if engines.is_empty() {
        return Ok(SearchResponse {
            query: query.query.clone(),
            results: Vec::new(),
            errors: vec![EngineErrorInfo {
                engine: "system".to_string(),
                error: "no engines available".to_string(),
            }],
            answer: None,
        });
    }

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

    // url -> (raw_result, engines, weight)
    let mut dedup_map: HashMap<String, (RawSearchResult, Vec<String>, f32)> = HashMap::new();
    let mut errors: Vec<EngineErrorInfo> = Vec::new();

    while let Some(Ok((engine_name, result))) = tasks.join_next().await {
        match result {
            Ok(raw_results) => {
                suspension.record_success(&engine_name);
                // Find the engine weight
                let engine_weight = engines
                    .iter()
                    .find(|e| e.name() == engine_name)
                    .map(|e| e.weight())
                    .unwrap_or(1.0);
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
                errors.push(EngineErrorInfo {
                    engine: engine_name,
                    error: error_msg,
                });
            }
        }
    }

    // Score and sort
    let mut results: Vec<SearchResult> = dedup_map
        .into_iter()
        .map(|(_, (raw, engines, weight))| {
            let mut scored = score_result(raw, query, weight);
            scored.engine = engines[0].clone();
            scored.engines = engines;
            scored
        })
        .collect();

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    // Apply max_results
    results.truncate(query.max_results);

    Ok(SearchResponse {
        query: query.query.clone(),
        results,
        errors,
        answer: None,
    })
}

/// Select engines based on the query, skipping suspended ones.
fn select_engines(
    _query: &SearchQuery,
    registry: &EngineRegistry,
    suspension: &EngineSuspensionManager,
) -> Vec<EngineRef> {
    registry
        .by_category("general")
        .into_iter()
        .filter(|e| !suspension.is_suspended(e.name()))
        .collect()
}

