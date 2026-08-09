//! Error types.
//!
//! HTTP errors follow RFC 9457 (application/problem+json).
//! MCP tool failures return `isError: true` with a structured `ToolError`.

use thiserror::Error;

/// Machine-readable error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    ValidationError,
    NotFound,
    Timeout,
    UpstreamError,
    InternalError,
    EngineSuspended,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::ValidationError => "VALIDATION_ERROR",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::Timeout => "TIMEOUT",
            ErrorCode::UpstreamError => "UPSTREAM_ERROR",
            ErrorCode::InternalError => "INTERNAL_ERROR",
            ErrorCode::EngineSuspended => "ENGINE_SUSPENDED",
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            ErrorCode::ValidationError => 422,
            ErrorCode::NotFound => 404,
            ErrorCode::Timeout => 504,
            ErrorCode::UpstreamError => 502,
            ErrorCode::InternalError => 500,
            ErrorCode::EngineSuspended => 503,
        }
    }
}

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

impl SearchError {
    pub fn code(&self) -> ErrorCode {
        match self {
            SearchError::HttpStatus(404) => ErrorCode::NotFound,
            SearchError::HttpStatus(408) => ErrorCode::Timeout,
            SearchError::HttpStatus(429) => ErrorCode::EngineSuspended,
            SearchError::HttpStatus(502) | SearchError::HttpStatus(503) | SearchError::HttpStatus(504) => {
                ErrorCode::UpstreamError
            }
            SearchError::HttpStatus(_) => ErrorCode::UpstreamError,
            SearchError::EmptyResultSet => ErrorCode::NotFound,
            SearchError::EngineNotFound(_) => ErrorCode::NotFound,
            SearchError::Parse(_) => ErrorCode::InternalError,
            SearchError::Request(_) => ErrorCode::UpstreamError,
            SearchError::Timeout => ErrorCode::Timeout,
            SearchError::Io(_) => ErrorCode::InternalError,
            SearchError::Config(_) => ErrorCode::InternalError,
        }
    }
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

/// RFC 9457 problem details for HTTP error responses.
#[derive(Debug, serde::Serialize)]
pub struct ApiError {
    /// Resolvable URI identifying the error type.
    #[serde(rename = "type")]
    pub type_uri: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub invalid_params: Vec<InvalidParam>,
}

#[derive(Debug, serde::Serialize)]
pub struct InvalidParam {
    pub name: String,
    pub reason: String,
}

impl ApiError {
    pub fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            type_uri: format!("https://agent-search.dev/errors/{}", code.as_str().to_lowercase()),
            title: code.as_str().to_string(),
            status: code.http_status(),
            detail: detail.into(),
            instance: None,
            invalid_params: Vec::new(),
        }
    }

    pub fn with_param(mut self, name: impl Into<String>, reason: impl Into<String>) -> Self {
        self.invalid_params.push(InvalidParam {
            name: name.into(),
            reason: reason.into(),
        });
        self
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = axum::http::StatusCode::from_u16(self.status)
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        (
            status,
            [(axum::http::header::CONTENT_TYPE, "application/problem+json")],
            axum::Json(self),
        )
            .into_response()
    }
}

impl From<SearchError> for ApiError {
    fn from(e: SearchError) -> Self {
        ApiError::new(e.code(), e.to_string())
    }
}

/// Structured error for MCP tool failures (returned with `isError: true`).
#[derive(Debug, serde::Serialize)]
pub struct ToolError {
    pub error_code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
}

impl ToolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error_code: code.as_str(),
            field: None,
            message: message.into(),
            example: None,
        }
    }

    pub fn with_field(mut self, field: impl Into<String>, example: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self.example = Some(example.into());
        self
    }
}
