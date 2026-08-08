//! Error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SearchError {
    #[error("engine request failed: {0}")]
    Request(String),

    #[error("HTTP {0}")]
    HttpStatus(u16),

    #[error("engine returned empty result set")]
    EmptyResultSet,

    #[error("engine not found: {0}")]
    EngineNotFound(String),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("timeout")]
    Timeout,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),
}

impl From<reqwest::Error> for SearchError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            SearchError::Timeout
        } else {
            SearchError::Request(e.to_string())
        }
    }
}

impl From<serde_json::Error> for SearchError {
    fn from(e: serde_json::Error) -> Self {
        SearchError::Parse(e.to_string())
    }
}

pub type EngineResult<T> = Result<T, SearchError>;
