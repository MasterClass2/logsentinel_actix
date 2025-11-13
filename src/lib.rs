//! # LogSentinel Actix SDK
//!
//! Non-intrusive request/response logging middleware for Actix-Web applications.
//!
//! This SDK captures HTTP request and response metadata and sends it to a central
//! LogSentinel server for AI-powered analysis. It's designed with these principles:
//!
//! - **Non-blocking**: Uses async background tasks to never slow down requests
//! - **Fail-safe**: Missing config or network errors never crash your app
//! - **Non-intrusive**: Logs are sent as-is, no modification of request/response
//! - **Secure**: API keys are never logged, sensitive headers are filtered
//!
//! ## Quick Start
//!
//! ```rust
//! use actix_web::{App, HttpServer, web, HttpResponse};
//! use logsentinel_actix::LogSentinelMiddleware;
//!
//! #[actix_web::main]
//! async fn main() -> std::io::Result<()> {
//!     // Configure via environment variables
//!     std::env::set_var("LOGSENTINEL_API_KEY", "your_api_key");
//!     std::env::set_var("LOGSENTINEL_BASE_URL", "https://logs.example.com");
//!
//!     HttpServer::new(|| {
//!         App::new()
//!             .wrap(LogSentinelMiddleware::new())
//!             .service(web::resource("/").to(|| async {
//!                 HttpResponse::Ok().body("Hello!")
//!             }))
//!     })
//!     .bind("0.0.0.0:8080")?
//!     .run()
//!     .await
//! }
//! ```
//!
//! ## Configuration
//!
//! The SDK is configured via environment variables:
//!
//! - `LOGSENTINEL_API_KEY`: Your authentication key (required)
//! - `LOGSENTINEL_BASE_URL`: LogSentinel server URL (required)
//!
//! If either is missing, the SDK disables itself gracefully without affecting your app.
//!
//! ## How It Works
//!
//! 1. Middleware captures request metadata (method, path, headers, IP)
//! 2. Request passes through to your handlers (no blocking)
//! 3. Response metadata is captured (status, headers, duration)
//! 4. Log is sent asynchronously in background via `tokio::spawn`
//! 5. Any errors are logged to stderr but never interrupt the app
//!
//! ## Architecture
//!
//! The SDK is structured into focused modules:
//!
//! - `middleware`: Actix-Web middleware implementation
//! - `config`: Environment-based configuration loading
//! - `logger`: Log event structure and forwarding logic
//! - `client`: HTTP client for sending logs to server
//! - `error`: Custom error types for clean error handling
//! - `utils`: Helper functions (timestamps, IPs, truncation)

pub mod client;
pub mod config;
pub mod error;
pub mod logger;
pub mod middleware;
pub mod utils;

// Re-export main components for easy access
pub use config::Config;
pub use error::LogError;
pub use middleware::LogSentinelMiddleware;

/// Convenience prelude for importing common types
pub mod prelude {
    pub use crate::config::Config;
    pub use crate::error::LogError;
    pub use crate::middleware::LogSentinelMiddleware;
}
