//! Agent Search — high-performance search engine for AI agents.

pub mod aggregator;
pub mod cache;
pub mod config;
pub mod crawler;
pub mod dedup;
pub mod engine;
pub mod fetcher;
pub mod index;
pub mod mcp;
pub mod models;
pub mod proxy;
pub mod ranking;
pub mod routes;

tokio::task_local! {
    /// Trace ID for the current request. Set by the trace middleware in
    /// `main.rs` and read by downstream code (e.g. engine requests) for
    /// distributed tracing via the `x-trace-id` header.
    pub static TRACE_ID: String;
}
