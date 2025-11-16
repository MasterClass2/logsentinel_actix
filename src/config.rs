//! Configuration management for LogSentinel SDK
//!
//! Handles loading and validating environment variables with fail-soft behavior.
//! If configuration is missing or invalid, the SDK disables itself gracefully
//! without affecting the host application.

use std::sync::Arc;

/// Configuration for LogSentinel SDK
///
/// Loaded from environment variables:
/// - `LOGSENTINEL_API_KEY`: Authentication key for the LogSentinel server
/// - `LOGSENTINEL_BASE_URL`: Base URL of the LogSentinel server (e.g., https://logs.example.com)
/// - `LOGSENTINEL_DEBUG`: Optional flag ("true"/"1") to enable verbose SDK debug output
#[derive(Debug, Clone)]
pub struct Config {
    /// Whether the SDK is active (has valid configuration)
    pub active: bool,

    /// API key for authentication
    pub api_key: Option<String>,

    /// Base URL for the LogSentinel server
    pub base_url: Option<String>,

    /// Whether debug logging is enabled (from env LOGSENTINEL_DEBUG)
    pub debug: bool,
}

impl Config {
    /// Load configuration from environment variables
    ///
    /// This function implements fail-soft behavior:
    /// - If both API key and base URL are present → SDK is active
    /// - If either is missing → SDK disables itself and prints a warning
    ///
    /// The SDK never panics or crashes the application due to missing config.
    pub fn from_env() -> Self {
        let api_key = std::env::var("LOGSENTINEL_API_KEY").ok();
        let base_url = std::env::var("LOGSENTINEL_BASE_URL").ok();

        // Optional debug flag (true if env var is "true" or "1")
        let debug = match std::env::var("LOGSENTINEL_DEBUG") {
            Ok(val) => matches!(val.to_lowercase().as_str(), "true" | "1" | "yes"),
            Err(_) => false,
        };

        // Validate that both required fields are present
        let active = api_key.is_some() && base_url.is_some();

        if !active {
            // Detailed warnings for missing config
            if api_key.is_none() && base_url.is_none() {
                eprintln!("[LogSentinel] SDK DISABLED: Both LOGSENTINEL_API_KEY and LOGSENTINEL_BASE_URL environment variables are missing");
            } else if api_key.is_none() {
                eprintln!("[LogSentinel] SDK DISABLED: LOGSENTINEL_API_KEY environment variable is missing");
            } else if base_url.is_none() {
                eprintln!("[LogSentinel] SDK DISABLED: LOGSENTINEL_BASE_URL environment variable is missing");
            }
            eprintln!("[LogSentinel]  Set these variables to enable request logging");
        } else if debug {
            println!("[LogSentinel] SDK initialized successfully (DEBUG MODE ACTIVE)");
            println!("[LogSentinel] All requests will be logged to: {}", base_url.as_ref().unwrap());
        } else if cfg!(debug_assertions) {
            println!("[LogSentinel] SDK initialized successfully");
        }

        Self {
            active,
            api_key,
            base_url,
            debug,
        }
    }

    /// Create configuration with explicit values (useful for testing)
    pub fn new(api_key: Option<String>, base_url: Option<String>) -> Self {
        let active = api_key.is_some() && base_url.is_some();
        Self {
            active,
            api_key,
            base_url,
            debug: false,
        }
    }

    /// Get API key (returns error if missing)
    pub fn get_api_key(&self) -> Result<&str, crate::error::LogError> {
        self.api_key
            .as_deref()
            .ok_or(crate::error::LogError::MissingConfig)
    }

    /// Get base URL (returns error if missing)
    pub fn get_base_url(&self) -> Result<&str, crate::error::LogError> {
        self.base_url
            .as_deref()
            .ok_or(crate::error::LogError::MissingConfig)
    }

    /// Wrap config in Arc for thread-safe sharing
    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::from_env()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_with_valid_env() {
        let config = Config::new(
            Some("test_api_key".to_string()),
            Some("https://logs.example.com".to_string()),
        );

        assert!(config.active);
        assert_eq!(config.get_api_key().unwrap(), "test_api_key");
        assert_eq!(config.get_base_url().unwrap(), "https://logs.example.com");
    }

    #[test]
    fn test_config_missing_api_key() {
        let config = Config::new(None, Some("https://logs.example.com".to_string()));

        assert!(!config.active);
        assert!(config.get_api_key().is_err());
    }

    #[test]
    fn test_config_missing_base_url() {
        let config = Config::new(Some("test_api_key".to_string()), None);

        assert!(!config.active);
        assert!(config.get_base_url().is_err());
    }

    #[test]
    fn test_config_completely_missing() {
        let config = Config::new(None, None);

        assert!(!config.active);
        assert!(config.get_api_key().is_err());
        assert!(config.get_base_url().is_err());
    }

    #[test]
    fn test_config_into_arc() {
        let config = Config::new(
            Some("key".to_string()),
            Some("url".to_string()),
        );

        let arc_config = config.into_arc();
        assert!(arc_config.active);
    }

    #[test]
    fn test_config_default_from_env() {
        let config = Config::default();
        let _ = config.active;
    }

    #[test]
    fn test_debug_flag_from_env() {
        std::env::set_var("LOGSENTINEL_DEBUG", "true");
        let config = Config::from_env();
        assert!(config.debug);
        std::env::remove_var("LOGSENTINEL_DEBUG");
    }
}