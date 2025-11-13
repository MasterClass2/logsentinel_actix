//! Actix-Web middleware implementation for LogSentinel
//!
//! This middleware hooks into the request/response lifecycle to capture
//! HTTP metadata and forward it to the LogSentinel server. It's designed
//! to be completely non-intrusive - it never blocks or modifies requests.

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    web::Bytes,
    Error, HttpMessage,
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
    pub fn new() -> Self {
        let config = Arc::new(Config::from_env());
        Self { config }
    }

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

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        // Skip logging if SDK is inactive (missing config)
        if !self.config.active {
            let fut = self.service.call(req);
            return Box::pin(async move {
                let res = fut.await?;
                Ok(res.map_into_boxed_body())
            });
        }

        let start_time = Instant::now();
        let trace_id = generate_trace_id();
        let config = self.config.clone();

        // ==== NEW: Capture request body ====
        // Reading the body is tricky because once consumed, the handler cannot read it.
        // We extract it fully here for logging, then replace it into the request.
        // We limit capture to MAX_BODY_SIZE to avoid memory issues.
        let fut_req_body = async move {
            let mut body_bytes = Bytes::new();
            if let Ok(payload) = req.take_payload().try_fold(Bytes::new(), |mut acc, chunk| async move {
                let remaining = MAX_BODY_SIZE.saturating_sub(acc.len());
                if remaining > 0 {
                    let to_take = chunk.len().min(remaining);
                    acc.extend_from_slice(&chunk[..to_take]);
                }
                Ok::<_, Error>(acc)
            }).await {
                body_bytes = payload;
            }

            // Re-insert body into the request so handlers can still read it
            let cloned_body = body_bytes.clone();
            let new_payload = actix_web::dev::Payload::from(cloned_body.clone());
            req.set_payload(new_payload);

            // Convert to UTF-8 string if possible for logging
            let body_str = if !body_bytes.is_empty() {
                Some(String::from_utf8_lossy(&body_bytes[..]).to_string())
            } else {
                None
            };

            body_str
        };

        // Capture headers and IP as before
        let headers = extract_headers(&req);
        let peer_addr = req.peer_addr().map(|addr| addr.to_string());
        let ip = extract_ip(&headers, peer_addr.as_deref());
        let method = req.method().to_string();
        let path = req.path().to_string();

        let fut = self.service.call(req);

        Box::pin(async move {
            let request_body = fut_req_body.await;
            let request_data = RequestData {
                method,
                path,
                headers,
                body: request_body,
                ip,
            };

            let res = fut.await?;

            let duration_ms = start_time.elapsed().as_millis() as u64;

            // Capture response metadata and body
            let (res, body_logger) = wrap_response_body(res);
            let response_headers = extract_response_headers(&res);
            let status = res.status().as_u16();

            tokio::spawn(async move {
                let body = body_logger.get_captured_body().await;
                let response_data = ResponseData {
                    status,
                    headers: response_headers,
                    body,
                    duration_ms,
                };

                let event = LogEvent::new(
                    current_timestamp(),
                    trace_id,
                    request_data,
                    response_data,
                );

                forward_log(event, config).await;
            });

            Ok(res)
        })
    }
}

/// Capture request metadata from ServiceRequest
/// Now includes the body fully captured
fn capture_request_data(_req: &ServiceRequest) -> RequestData {
    // The real capture is now done in `call()` because we need to consume and restore the body
    RequestData {
        method: "".to_string(),
        path: "".to_string(),
        headers: HashMap::new(),
        body: None,
        ip: None,
    }
}

/// Wrap response body to capture it while streaming to client
/// Fully captures small bodies using try_into_bytes() if possible
fn wrap_response_body<B>(
    res: ServiceResponse<B>,
) -> (ServiceResponse<actix_web::body::BoxBody>, Arc<BodyLoggerState>)
where
    B: actix_web::body::MessageBody + 'static,
{
    let state = Arc::new(BodyLoggerState::new());
    let state_for_body = state.clone();

    let mapped = res.map_body(move |_, body| {
        // Check if body is already available as bytes for small responses
        if let Ok(bytes) = body.try_into_bytes() {
            let mut buffer = futures::executor::block_on(state_for_body.buffer.lock());
            let to_take = bytes.len().min(MAX_BODY_SIZE);
            buffer.extend_from_slice(&bytes[..to_take]);
            state_for_body.completed.notify_one();
            actix_web::body::BoxBody::new(bytes)
        } else {
            let logged_body = BodyLogger {
                body,
                state: state_for_body.clone(),
            };
            logged_body.boxed()
        }
    });

    (mapped, state)
}

// ---- rest of the helpers remain unchanged ----

fn extract_headers(req: &ServiceRequest) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    for (name, value) in req.headers() {
        let name_lower = name.as_str().to_lowercase();
        if is_sensitive_header(&name_lower) {
            continue;
        }
        if let Ok(value_str) = value.to_str() {
            headers.insert(name_lower, value_str.to_string());
        }
    }
    headers
}

fn extract_response_headers<B>(res: &ServiceResponse<B>) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    for (name, value) in res.headers() {
        if let Ok(value_str) = value.to_str() {
            headers.insert(name.as_str().to_lowercase(), value_str.to_string());
        }
    }
    headers
}

fn is_sensitive_header(name: &str) -> bool {
    matches!(
        name,
        "authorization" | "cookie" | "set-cookie" | "proxy-authorization" | "x-api-key" | "x-auth-token"
    )
}

fn should_capture_body(headers: &HashMap<String, String>) -> bool {
    if let Some(content_type) = headers.get("content-type") {
        let content_type_lower = content_type.to_lowercase();
        content_type_lower.contains("json")
            || content_type_lower.contains("text")
            || content_type_lower.contains("xml")
            || content_type_lower.contains("x-www-form-urlencoded")
    } else {
        false
    }
}

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

    async fn get_captured_body(&self) -> Option<String> {
        self.completed.notified().await;
        let buffer = self.buffer.lock().await;
        if buffer.is_empty() {
            return None;
        }
        String::from_utf8(buffer.clone()).ok().map(|s| truncate_body(&s, MAX_BODY_SIZE))
    }
}

struct BodyLogger<B> {
    body: B,
    state: Arc<BodyLoggerState>,
}

impl<B> MessageBody for BodyLogger<B>
where
    B: MessageBody,
{
    type Error = B::Error;

    fn size(&self) -> actix_web::body::BodySize {
        self.body.size()
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        let this = unsafe { self.get_unchecked_mut() };
        let body = unsafe { Pin::new_unchecked(&mut this.body) };

        match body.poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                let state = this.state.clone();
                let chunk_clone = chunk.clone();
                tokio::spawn(async move {
                    let mut buffer = state.buffer.lock().await;
                    if buffer.len() < MAX_BODY_SIZE {
                        let remaining = MAX_BODY_SIZE - buffer.len();
                        let to_capture = chunk_clone.len().min(remaining);
                        buffer.extend_from_slice(&chunk_clone[..to_capture]);
                    }
                });
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(None) => {
                this.state.completed.notify_one();
                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(e))) => {
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
        let _ = middleware.config.active;
    }
}
