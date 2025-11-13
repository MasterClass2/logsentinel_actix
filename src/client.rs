//! HTTP client for sending logs to LogSentinel server
//!
//! Handles POST requests with proper authentication, timeouts, and error handling.
//! Designed to be non-blocking and fail-safe - network errors never crash the app.

use reqwest::Client;
use std::time::Duration;

use crate::config::Config;
use crate::error::LogError;
use crate::logger::LogEvent;

/// Send log event to LogSentinel server via HTTP POST
///
/// This function:
/// 1. Validates configuration (API key and base URL)
/// 2. Serializes log event to JSON
/// 3. Sends POST request with authentication header
/// 4. Enforces 5-second timeout to prevent hanging
/// 5. Returns error on failure (caller handles gracefully)
///
/// # Arguments
/// * `event` - The log event to send
/// * `config` - SDK configuration containing API key and base URL
///
/// # Returns
/// * `Ok(())` -  successfully
/// * `Err(LogError)` - Network error, timeout, or server error
pub async fn send(event: LogEvent, config: &Config) -> Result<(), LogError> {
    // Validate configuration before attempting to send
    let base_url = config.get_base_url()?;
    let api_key = config.get_api_key()?;

    // Build HTTP client with reasonable defaults
    let client = Client::builder()
        .timeout(Duration::from_secs(5)) // Don't wait forever
        .build()
        .map_err(|e| LogError::Network(e))?;

    // Construct endpoint URL
    let endpoint = base_url.trim_end_matches('/').to_string();

    // Convert log event to JSON for optional debug logging
    let json_payload = serde_json::to_string_pretty(&event)
        .map_err(LogError::Serialization)?; // ✅ FIXED: remove `.to_string()`

    // Debug: print payload being sent if debug mode is enabled
    if config.debug {
        println!("[LogSentinel SDK Debug] Sending payload to backend:\n{}", json_payload);
        println!("[LogSentinel SDK Debug] Endpoint: {}", endpoint);
    }

    // Send POST request with JSON payload and authentication
    let response = client
        .post(&endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .body(json_payload.clone())
        .send()
        .await?;

    // Check response status
    if !response.status().is_success() {
        return Err(LogError::SendFailed(response.status()));
    }

    // Log success message
    println!("[LogSentinel] ✓ Log sent successfully to server");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logger::{RequestData, ResponseData};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_send_with_missing_config() {
        let config = Config::new(None, None);
        let event = create_test_event();

        let result = send(event, &config).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), LogError::MissingConfig));
    }

    #[tokio::test]
    async fn test_send_with_missing_api_key() {
        let config = Config::new(None, Some("http://localhost:8080".to_string()));
        let event = create_test_event();

        let result = send(event, &config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_send_with_missing_base_url() {
        let config = Config::new(Some("test_key".to_string()), None);
        let event = create_test_event();

        let result = send(event, &config).await;
        assert!(result.is_err());
    }

    fn create_test_event() -> LogEvent {
        LogEvent::new(
            "2024-01-15T10:30:00Z".to_string(),
            "test-trace-123".to_string(),
            RequestData {
                method: "GET".to_string(),
                path: "/api/test".to_string(),
                headers: HashMap::new(),
                body: None,
                ip: Some("192.168.1.1".to_string()),
            },
            ResponseData {
                status: 200,
                headers: HashMap::new(),
                body: None,
                duration_ms: 42,
            },
        )
    }
}
