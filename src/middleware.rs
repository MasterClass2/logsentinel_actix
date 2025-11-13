//! Actix-Web middleware implementation for LogSentinel
//!
//! This middleware hooks into the request/response lifecycle to capture
//! HTTP metadata and forward it to the LogSentinel server. It's designed
//! to be completely non-intrusive - it never blocks or modifies requests.

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    web::Bytes,
    Error,
};
use actix_web::body::MessageBody;
use futures::future::{ok, LocalBoxFuture, Ready};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use crate::config::Config;
use crate::logger::{forward_log, LogEvent, RequestData, ResponseData};
use crate::utils::{current_timestamp, extract_ip, generate_trace_id, truncate_body};

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

        // Capture request metadata before passing to handler
        let request_data = capture_request_data(&req);

        // Clone config for async task (Arc makes this cheap)
        let config = self.config.clone();

        // Call the next service in the chain (the actual handler)
        let fut = self.service.call(req);

        Box::pin(async move {
            // Await the response from the handler
            // This is where the actual request processing happens
            let res = fut.await?;

            // Calculate request duration
            let duration_ms = start_time.elapsed().as_millis() as u64;

            // Capture response metadata and wrap body for capture
            let (res, body_logger) = wrap_response_body(res);
            let response_headers = extract_response_headers(&res);
            let status = res.status().as_u16();

            // Spawn background task to capture body and send log
            // This runs after the response is returned to avoid blocking
            tokio::spawn(async move {
                // Wait for body to be captured
                let body = body_logger.get_captured_body().await;
                
                // Build response data with captured body
                let response_data = ResponseData {
                    status,
                    headers: response_headers,
                    body,
                    duration_ms,
                };

                // Build complete log event
                let event = LogEvent::new(
                    current_timestamp(),
                    trace_id,
                    request_data,
                    response_data,
                );

                // Forward log to server (non-blocking)
                forward_log(event, config).await;
            });

            // Return response to client immediately
            Ok(res)
        })
    }
}

/// Capture request metadata from ServiceRequest
///
/// Extracts method, path, headers, and optional body.
/// Headers are sanitized to remove sensitive auth tokens.
fn capture_request_data(req: &ServiceRequest) -> RequestData {
    // Extract basic request info
    let method = req.method().to_string();
    let path = req.path().to_string();

    // Capture headers (sanitized)
    let headers = extract_headers(req);

    // Extract IP address from headers or connection
    let peer_addr = req.peer_addr().map(|addr| addr.to_string());
    let ip = extract_ip(&headers, peer_addr.as_deref());

    // Capture request body if it's text-based content
    // Note: We only capture small, text-based bodies to avoid performance impact
    let body = if should_capture_body(&headers) {
        // Body capture from ServiceRequest is complex because we can't easily
        // read and reconstruct the payload. For now, we skip request body capture
        // to maintain simplicity and avoid interfering with request processing.
        // Response body capture is more feasible via body wrapping.
        None
    } else {
        None
    };

    RequestData {
        method,
        path,
        headers,
        body,
        ip,
    }
}

/// Wrap response body to capture it while streaming to client
///
/// Returns the modified response and a BodyLogger that will capture the body
fn wrap_response_body<B>(
    res: ServiceResponse<B>,
) -> (ServiceResponse<actix_web::body::BoxBody>, Arc<BodyLoggerState>)
where
    B: actix_web::body::MessageBody + 'static,
{
    let state = Arc::new(BodyLoggerState::new());
    let state_for_body = state.clone(); // clone before the closure

    let mapped = res.map_body(move |_, body| {
        let logged_body = BodyLogger { body, state: state_for_body.clone() };
        logged_body.boxed()
    });

    (mapped, state)
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

/// State shared between BodyLogger and the async task
struct BodyLoggerState {
    buffer: tokio::sync::Mutex<Vec<u8>>,
    completed: tokio::sync::Notify,
}

impl BodyLoggerState {
    fn new() -> Self {
        Self {
            buffer: tokio::sync::Mutex::new(Vec::new()),
            completed: tokio::sync::Notify::new(),
        }
    }

    /// Wait for body capture to complete and return the captured body
    async fn get_captured_body(&self) -> Option<String> {
        // Wait for body to be fully captured
        self.completed.notified().await;
        
        let buffer = self.buffer.lock().await;
        
        // Convert to string if valid UTF-8
        if buffer.is_empty() {
            return None;
        }
        
        String::from_utf8(buffer.clone())
            .ok()
            .map(|s| truncate_body(&s, MAX_BODY_SIZE))
    }
}

/// Body wrapper that captures response body while streaming to client
///
/// This implements the MessageBody trait to intercept body chunks
/// as they're sent to the client, allowing us to capture them without
/// blocking or modifying the response.
struct BodyLogger<B> {
    body: B,
    state: Arc<BodyLoggerState>,
}

impl<B> actix_web::body::MessageBody for BodyLogger<B>
where
    B: actix_web::body::MessageBody,
{
    type Error = B::Error;

    fn size(&self) -> actix_web::body::BodySize {
        self.body.size()
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        // SAFETY: We're not moving the body, just getting a mutable reference
        let this = unsafe { self.get_unchecked_mut() };
        let body = unsafe { Pin::new_unchecked(&mut this.body) };
        
        match body.poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                // Capture chunk in background without blocking
                let state = this.state.clone();
                let chunk_clone = chunk.clone();
                
                tokio::spawn(async move {
                    let mut buffer = state.buffer.lock().await;
                    
                    // Only capture up to MAX_BODY_SIZE to avoid memory issues
                    if buffer.len() < MAX_BODY_SIZE {
                        let remaining = MAX_BODY_SIZE - buffer.len();
                        let to_capture = chunk_clone.len().min(remaining);
                        buffer.extend_from_slice(&chunk_clone[..to_capture]);
                    }
                });
                
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(None) => {
                // Body complete - notify waiting task
                this.state.completed.notify_one();
                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(e))) => {
                // Error occurred - notify waiting task
                this.state.completed.notify_one();
                Poll::Ready(Some(Err(e)))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn try_into_bytes(self) -> Result<Bytes, Self>
    where
        Self: Sized,
    {
        Err(self)
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