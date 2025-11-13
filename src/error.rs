//! Error types for LogSentinel SDK
//!
//! All errors are designed to be non-fatal - they're caught internally
//! and logged to stderr without crashing the host application.

use thiserror::Error;

/// Main error type for LogSentinel operations
#[derive(Error, Debug)]
pub enum LogError {
    /// Configuration is missing or invalid (API key or base URL)
    #[error("Missing configuration: API key or base URL not provided")]
    MissingConfig,

    /// Network-related errors (connection, DNS, etc.)
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    /// HTTP request failed with non-success status code
    #[error("Failed to send log: HTTP {0}")]
    SendFailed(reqwest::StatusCode),

    /// JSON serialization/deserialization failed
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Generic error for unexpected failures
    #[error("Unexpected error: {0}")]
    Other(String),
}

impl LogError {
    /// Create a generic error from any error type
    pub fn from_string(msg: impl Into<String>) -> Self {
        LogError::Other(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = LogError::MissingConfig;
        assert!(err.to_string().contains("Missing configuration"));

        let err = LogError::from_string("test error");
        assert_eq!(err.to_string(), "Unexpected error: test error");
    }

    #[test]
    fn test_error_from_json() {
        let json_err = serde_json::from_str::<()>("invalid").unwrap_err();
        let log_err: LogError = json_err.into();
        assert!(matches!(log_err, LogError::Serialization(_)));
    }
}
