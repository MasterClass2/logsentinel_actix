//! Log event handling and forwarding
//!
//! This module defines the log event structure and provides a thin passthrough
//! layer for forwarding logs to the HTTP client. No modification or processing
//! of log content occurs here - logs are sent as-is to the server.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::client;
use crate::config::Config;

/// Complete log event containing request and response data
///
/// This structure is serialized to JSON and sent to the LogSentinel server.
/// All fields are captured as-is from the HTTP request/response cycle.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogEvent {
    /// RFC3339 timestamp when the request was received
    pub timestamp: String,

    /// Unique trace ID for correlating this request across systems
    pub trace_id: String,

    /// Request metadata
    pub request: RequestData,

    /// Response metadata
    pub response: ResponseData,
}

/// HTTP request metadata
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RequestData {
    /// HTTP method (GET, POST, etc.)
    pub method: String,

    /// Request path (e.g., /api/users)
    pub path: String,

    /// Request headers (sanitized - no auth tokens)
    pub headers: HashMap<String, String>,

    /// Request body (truncated if too large)
    pub body: Option<String>,

    /// Client IP address (extracted from headers or connection)
    pub ip: Option<String>,
}

/// HTTP response metadata
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResponseData {
    /// HTTP status code (e.g., 200, 404, 500)
    pub status: u16,

    /// Response headers
    pub headers: HashMap<String, String>,

    /// Response body (truncated if too large, None for binary content)
    pub body: Option<String>,

    /// Request processing duration in milliseconds
    pub duration_ms: u64,
}

impl LogEvent {
    /// Create a new log event
    pub fn new(
        timestamp: String,
        trace_id: String,
        request: RequestData,
        response: ResponseData,
    ) -> Self {
        Self {
            timestamp,
            trace_id,
            request,
            response,
        }
    }
}

/// Forward log event to the LogSentinel server
///
/// This function implements the core principle of non-blocking, fail-safe logging:
/// 1. Check if SDK is active (has valid config)
/// 2. Spawn async task to send log in background
/// 3. Never block the main request/response cycle
/// 4. Print errors to stderr but never crash
///
/// # Arguments
/// * `event` - The log event to send
/// * `config` - SDK configuration (must be Arc for sharing across tasks)
pub async fn forward_log(event: LogEvent, config: Arc<Config>) {
    // Silent skip if SDK is disabled (missing config)
    if !config.active {
        return;
    }

    // Spawn background task to send log without blocking
    // If this fails, the host application is unaffected
    tokio::spawn(async move {
        if let Err(e) = client::send(event, &config).await {
            // Log error to stderr but don't crash
            eprintln!("[LogSentinel] Failed to send log: {:?}", e);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_event_serialization() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        let request = RequestData {
            method: "POST".to_string(),
            path: "/api/test".to_string(),
            headers: headers.clone(),
            body: Some(r#"{"key":"value"}"#.to_string()),
            ip: Some("192.168.1.1".to_string()),
        };

        let response = ResponseData {
            status: 200,
            headers: headers.clone(),
            body: Some(r#"{"result":"success"}"#.to_string()),
            duration_ms: 42,
        };

        let event = LogEvent::new(
            "2024-01-15T10:30:00Z".to_string(),
            "trace-123".to_string(),
            request,
            response,
        );

        // Test serialization to JSON
        let json = serde_json::to_string(&event).expect("Failed to serialize");
        assert!(json.contains("POST"));
        assert!(json.contains("/api/test"));
        assert!(json.contains("trace-123"));
        assert!(json.contains("\"status\":200"));
    }

    #[test]
    fn test_log_event_deserialization() {
        let json = r#"{
            "timestamp": "2024-01-15T10:30:00Z",
            "trace_id": "trace-456",
            "request": {
                "method": "GET",
                "path": "/health",
                "headers": {},
                "body": null,
                "ip": "127.0.0.1"
            },
            "response": {
                "status": 200,
                "headers": {},
                "body": null,
                "duration_ms": 5
            }
        }"#;

        let event: LogEvent = serde_json::from_str(json).expect("Failed to deserialize");
        assert_eq!(event.trace_id, "trace-456");
        assert_eq!(event.request.method, "GET");
        assert_eq!(event.response.status, 200);
    }

    #[tokio::test]
    async fn test_forward_log_with_inactive_config() {
        // Create inactive config
        let config = Arc::new(Config::new(None, None));
        assert!(!config.active);

        let event = create_test_event();

        // Should not panic or block
        forward_log(event, config).await;
        // If we get here, test passed (no crash)
    }

    #[tokio::test]
    async fn test_forward_log_with_active_config() {
        // Create active config (will fail to send, but shouldn't crash)
        let config = Arc::new(Config::new(
            Some("test_key".to_string()),
            Some("http://localhost:9999".to_string()),
        ));

        let event = create_test_event();

        // Should spawn task and return immediately
        forward_log(event, config).await;
        // Test passes if no panic occurs
    }

    fn create_test_event() -> LogEvent {
        LogEvent::new(
            "2024-01-15T10:30:00Z".to_string(),
            "test-trace".to_string(),
            RequestData {
                method: "GET".to_string(),
                path: "/test".to_string(),
                headers: HashMap::new(),
                body: None,
                ip: Some("127.0.0.1".to_string()),
            },
            ResponseData {
                status: 200,
                headers: HashMap::new(),
                body: None,
                duration_ms: 10,
            },
        )
    }
}
