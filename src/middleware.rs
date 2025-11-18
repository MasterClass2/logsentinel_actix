//! Actix-Web middleware implementation for LogSentinel
//!
//! This middleware hooks into the request/response lifecycle to capture
//! HTTP metadata and forward it to the LogSentinel server. It's designed
//! to be completely non-intrusive - it never blocks or modifies requests.

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error,
};
use futures::future::{ok, LocalBoxFuture, Ready};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::config::Config;
use crate::logger::{forward_log, LogEvent, RequestData, ResponseData};
use crate::utils::{current_timestamp, extract_ip, generate_trace_id};
use crate::response_body_capture::capture_response_body;

/// Maximum body size to capture (10KB)
const MAX_BODY_SIZE: usize = 10 * 1024;

/// LogSentinel middleware for Actix-Web
///
/// Add this middleware to your Actix app via `.wrap()`:
///
/// ```rust,no_run
/// use actix_web::App;
/// use logsentinel_actix::LogSentinelMiddleware;
///
/// App::new()
///     .wrap(LogSentinelMiddleware::new())
///     .service(...)
/// ```
pub struct LogSentinelMiddleware {
    config: Arc<Config>,
}

impl LogSentinelMiddleware {
    /// Create new middleware instance
    ///
    /// Loads configuration from environment variables automatically.
    /// If config is invalid, middleware disables itself gracefully.
    pub fn new() -> Self {
        let config = Arc::new(Config::from_env());
        Self { config }
    }

    /// Create middleware with explicit configuration (useful for testing)
    pub fn with_config(config: Config) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

impl Default for LogSentinelMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

/// Implementation of Transform trait for middleware registration
impl<S, B> Transform<S, ServiceRequest> for LogSentinelMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: actix_web::body::MessageBody + 'static,
{
    type Response = ServiceResponse<actix_web::body::BoxBody>;
    type Error = Error;
    type InitError = ();
    type Transform = LogSentinelMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(LogSentinelMiddlewareService {
            service,
            config: self.config.clone(),
        })
    }
}

/// The actual service that handles each request
pub struct LogSentinelMiddlewareService<S> {
    service: S,
    config: Arc<Config>,
}

impl<S, B> Service<ServiceRequest> for LogSentinelMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: actix_web::body::MessageBody + 'static,
{
    type Response = ServiceResponse<actix_web::body::BoxBody>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        // Skip logging if SDK is inactive (missing config)
        if !self.config.active {
            let fut = self.service.call(req);
            return Box::pin(async move {
                let res = fut.await?;
                Ok(res.map_into_boxed_body())
            });
        }

        // Capture request start time for duration calculation
        let start_time = Instant::now();

        // Generate unique trace ID for this request
        let trace_id = generate_trace_id();

        // Capture request metadata (body capture happens later)
        let request_data = capture_request_data_with_body(&req, None);
        
        // Clone config for async task
        let config = self.config.clone();

        // Call the actual handler
        let fut = self.service.call(req);

        // Process response asynchronously after handler completes
        Box::pin(async move {
            // Await the response from the handler
            let res = fut.await?;

            // Calculate request duration
            let duration_ms = start_time.elapsed().as_millis() as u64;

            // Capture response status code
            let status = res.status().as_u16();

            // Capture response headers
            let response_headers = extract_response_headers(&res);

            // Capture response body asynchronously (non-blocking)
            let (modified_res, response_body) = capture_response_body(res).await;

            // Build response data with captured body
            let response_data = ResponseData {
                status,
                headers: response_headers,
                body: response_body,
                duration_ms,
            };

            // Create complete log event
            let event = LogEvent::new(
                current_timestamp(),
                trace_id,
                request_data,
                response_data,
            );

            // Forward log to server (non-blocking)
            tokio::spawn(async move {
                forward_log(event, config).await;
            });

            Ok(modified_res)
        })
    }
}

/// Capture request metadata from ServiceRequest with optional body
///
/// Extracts method, path, headers, and optional body.
/// Headers are sanitized to remove sensitive auth tokens.
fn capture_request_data_with_body(req: &ServiceRequest, body: Option<String>) -> RequestData {
    // Extract basic request info
    let method = req.method().to_string();
    let path = req.path().to_string();

    // Capture headers (sanitized)
    let headers = extract_headers(req);

    // Extract IP address from headers or connection
    let peer_addr = req.peer_addr().map(|addr| addr.to_string());
    let ip = extract_ip(&headers, peer_addr.as_deref());

    RequestData {
        method,
        path,
        headers,
        body,
        ip,
    }
}

/// Extract and sanitize request headers
///
/// Removes sensitive headers like Authorization, Cookie, etc.
fn extract_headers(req: &ServiceRequest) -> HashMap<String, String> {
    let mut headers = HashMap::new();

    for (name, value) in req.headers() {
        let name_lower = name.as_str().to_lowercase();

        // Skip sensitive headers
        if is_sensitive_header(&name_lower) {
            continue;
        }

        // Convert header value to string (skip if invalid UTF-8)
        if let Ok(value_str) = value.to_str() {
            headers.insert(name_lower, value_str.to_string());
        }
    }

    headers
}

/// Extract response headers
fn extract_response_headers<B>(res: &ServiceResponse<B>) -> HashMap<String, String> {
    let mut headers = HashMap::new();

    for (name, value) in res.headers() {
        let name_lower = name.as_str().to_lowercase();

        if let Ok(value_str) = value.to_str() {
            headers.insert(name_lower, value_str.to_string());
        }
    }

    headers
}

/// Check if header contains sensitive information
fn is_sensitive_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "cookie"
            | "set-cookie"
            | "proxy-authorization"
            | "x-api-key"
            | "x-auth-token"
    )
}

/// Check if we should attempt to capture body based on content type
fn should_capture_body(headers: &HashMap<String, String>) -> bool {
    if let Some(content_type) = headers.get("content-type") {
        let content_type_lower = content_type.to_lowercase();
        
        // Capture text-based content types only
        content_type_lower.contains("json")
            || content_type_lower.contains("text")
            || content_type_lower.contains("xml")
            || content_type_lower.contains("x-www-form-urlencoded")
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test, web, App, HttpResponse};

    #[actix_rt::test]
    async fn test_middleware_with_inactive_config() {
        // Create app with inactive middleware
        let config = Config::new(None, None);
        assert!(!config.active);

        let app = test::init_service(
            App::new()
                .wrap(LogSentinelMiddleware::with_config(config))
                .route("/test", web::get().to(|| async { HttpResponse::Ok().body("test") })),
        )
        .await;

        let req = test::TestRequest::get().uri("/test").to_request();
        let resp = test::call_service(&app, req).await;

        assert!(resp.status().is_success());
    }

    #[actix_rt::test]
    async fn test_middleware_with_active_config() {
        // Create app with active middleware (will try to send logs but fail gracefully)
        let config = Config::new(
            Some("test_key".to_string()),
            Some("http://localhost:9999".to_string()),
        );
        assert!(config.active);

        let app = test::init_service(
            App::new()
                .wrap(LogSentinelMiddleware::with_config(config))
                .route("/test", web::get().to(|| async { HttpResponse::Ok().body("test") })),
        )
        .await;

        let req = test::TestRequest::get().uri("/test").to_request();
        let resp = test::call_service(&app, req).await;

        // Request should succeed even if logging fails
        assert!(resp.status().is_success());
    }

    #[actix_rt::test]
    async fn test_is_sensitive_header() {
        assert!(is_sensitive_header("authorization"));
        assert!(is_sensitive_header("cookie"));
        assert!(is_sensitive_header("set-cookie"));
        assert!(is_sensitive_header("x-api-key"));

        assert!(!is_sensitive_header("content-type"));
        assert!(!is_sensitive_header("accept"));
        assert!(!is_sensitive_header("user-agent"));
    }

    #[actix_rt::test]
    async fn test_middleware_creation() {
        let middleware = LogSentinelMiddleware::new();
        // Should not panic
        let _ = middleware.config.active;
    }
}