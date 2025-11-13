//! Utility functions for LogSentinel SDK
//!
//! Provides helper functions for generating trace IDs, timestamps,
//! and handling data truncation.

use chrono::Utc;
use uuid::Uuid;

/// Generate a unique trace ID for correlating requests
///
/// Uses UUID v4 for guaranteed uniqueness across distributed systems.
pub fn generate_trace_id() -> String {
    Uuid::new_v4().to_string()
}

/// Get current timestamp in RFC3339 format
///
/// Returns ISO 8601 formatted timestamp with timezone (e.g., "2024-01-15T10:30:00Z")
/// This format is widely supported and human-readable.
pub fn current_timestamp() -> String {
    Utc::now().to_rfc3339()
}

/// Truncate body content to prevent sending massive payloads
///
/// Limits body size to `max_len` characters to avoid:
/// - Network bandwidth waste
/// - Memory issues on the server
/// - Slow serialization
///
/// Adds "... (truncated)" suffix when truncation occurs.
pub fn truncate_body(body: &str, max_len: usize) -> String {
    if body.len() > max_len {
        format!("{}... (truncated)", &body[..max_len])
    } else {
        body.to_string()
    }
}

/// Extract real IP address from request, handling proxies
///
/// Checks headers in this order:
/// 1. X-Real-IP (set by nginx)
/// 2. X-Forwarded-For (standard proxy header, takes first IP)
/// 3. Falls back to connection peer address
pub fn extract_ip(
    headers: &std::collections::HashMap<String, String>,
    peer_addr: Option<&str>,
) -> Option<String> {
    // Check X-Real-IP header first
    if let Some(ip) = headers.get("x-real-ip") {
        return Some(ip.clone());
    }

    // Check X-Forwarded-For (may contain multiple IPs, take first)
    if let Some(forwarded) = headers.get("x-forwarded-for") {
        if let Some(ip) = forwarded.split(',').next() {
            return Some(ip.trim().to_string());
        }
    }

    // Fall back to peer address
    peer_addr.map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_generate_trace_id() {
        let id1 = generate_trace_id();
        let id2 = generate_trace_id();

        // UUIDs should be unique
        assert_ne!(id1, id2);

        // Should be valid UUID format (36 chars with dashes)
        assert_eq!(id1.len(), 36);
        assert!(id1.contains('-'));
    }

    #[test]
    fn test_current_timestamp() {
        let ts = current_timestamp();

        // Should contain date and time separators
        assert!(ts.contains('T'));
        assert!(ts.contains(':'));

        // Should be parseable as RFC3339
        chrono::DateTime::parse_from_rfc3339(&ts).expect("Invalid RFC3339 timestamp");
    }

    #[test]
    fn test_truncate_body_short() {
        let body = "Hello, World!";
        let result = truncate_body(body, 100);
        assert_eq!(result, "Hello, World!");
    }

    #[test]
    fn test_truncate_body_long() {
        let body = "A".repeat(1000);
        let result = truncate_body(&body, 50);

        assert_eq!(result.len(), 50 + "... (truncated)".len());
        assert!(result.starts_with("AAAA"));
        assert!(result.ends_with("... (truncated)"));
    }

    #[test]
    fn test_truncate_body_exact_limit() {
        let body = "A".repeat(50);
        let result = truncate_body(&body, 50);
        assert_eq!(result, body);
        assert!(!result.contains("truncated"));
    }

    #[test]
    fn test_extract_ip_from_x_real_ip() {
        let mut headers = HashMap::new();
        headers.insert("x-real-ip".to_string(), "192.168.1.100".to_string());

        let ip = extract_ip(&headers, Some("10.0.0.1"));
        assert_eq!(ip, Some("192.168.1.100".to_string()));
    }

    #[test]
    fn test_extract_ip_from_x_forwarded_for() {
        let mut headers = HashMap::new();
        headers.insert(
            "x-forwarded-for".to_string(),
            "192.168.1.100, 10.0.0.1".to_string(),
        );

        let ip = extract_ip(&headers, Some("127.0.0.1"));
        assert_eq!(ip, Some("192.168.1.100".to_string()));
    }

    #[test]
    fn test_extract_ip_fallback_to_peer() {
        let headers = HashMap::new();
        let ip = extract_ip(&headers, Some("203.0.113.1"));
        assert_eq!(ip, Some("203.0.113.1".to_string()));
    }

    #[test]
    fn test_extract_ip_no_source() {
        let headers = HashMap::new();
        let ip = extract_ip(&headers, None);
        assert_eq!(ip, None);
    }
}
