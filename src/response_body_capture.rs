//! Response body capture for LogSentinel middleware
//!
//! Buffers response bodies up to 10KB before sending to the client.
//! This approach adds latency equal to response generation time.

use actix_web::{
    body::{BodySize, BoxBody, MessageBody},
    dev::ServiceResponse,
    web::Bytes,
    HttpResponse,
};
use futures::StreamExt;
use std::collections::HashMap;
use std::pin::pin;

const MAX_BODY_SIZE: usize = 10 * 1024; // 10KB limit for memory safety

/// Captures response body by buffering it before sending to client
///
/// Returns the modified response and the captured body string.
/// Note: This buffers the entire response before streaming, adding latency.
pub async fn capture_response_body<B: MessageBody + 'static>(
    res: ServiceResponse<B>,
) -> (ServiceResponse<BoxBody>, Option<String>) {
    let headers = extract_response_headers(&res);
    
    // Only capture text-based responses to avoid binary bloat
    if !should_capture_body(&headers) {
        return (res.map_into_boxed_body(), None);
    }

    // Destructure response into parts
    let (req, res) = res.into_parts();
    let (res_parts, body) = res.into_parts();
    
    // Buffer entire body to bytes
    let body_bytes = match body_to_bytes(body).await {
        Ok(bytes) => bytes,
        Err(_) => {
            // Failed to read body, reconstruct and return
            let new_res = HttpResponse::from(res_parts).set_body(());
            return (ServiceResponse::new(req, new_res).map_into_boxed_body(), None);
        }
    };
    
    // Capture body string (up to MAX_BODY_SIZE)
    let captured_body = bytes_to_string(&body_bytes);
    
    // Reconstruct response with same bytes
    let new_res = HttpResponse::from(res_parts).set_body(body_bytes.clone());
    let new_service_res = ServiceResponse::new(req, new_res);
    
    (new_service_res.map_into_boxed_body(), captured_body)
}

/// Converts MessageBody to Bytes by buffering up to MAX_BODY_SIZE
async fn body_to_bytes<B: MessageBody>(body: B) -> Result<Bytes, B::Error> {
    use actix_web::web::BytesMut;
    
    let cap = match body.size() {
        BodySize::None | BodySize::Sized(0) => return Ok(Bytes::new()),
        BodySize::Sized(size) => size as usize,
        BodySize::Stream => MAX_BODY_SIZE,
    };
    
    let mut buf = BytesMut::with_capacity(cap.min(MAX_BODY_SIZE));
    let mut body = pin!(body);
    
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        if buf.len() + chunk.len() > MAX_BODY_SIZE {
            // Stop at size limit
            buf.extend_from_slice(&chunk[..MAX_BODY_SIZE - buf.len()]);
            break;
        }
        buf.extend_from_slice(&chunk);
    }
    
    Ok(buf.freeze())
}

/// Converts bytes to UTF-8 string safely, truncating if needed
fn bytes_to_string(bytes: &Bytes) -> Option<String> {
    match String::from_utf8(bytes.to_vec()) {
        Ok(s) => {
            if s.len() > MAX_BODY_SIZE {
                Some(s[..MAX_BODY_SIZE].to_string() + "...[truncated]")
            } else {
                Some(s)
            }
        }
        Err(_) => None, // Invalid UTF-8
    }
}

/// Checks if response body should be captured based on Content-Type
///
/// Returns true for text-based formats only (JSON, XML, HTML, plain text)
fn should_capture_body(headers: &HashMap<String, String>) -> bool {
    let content_type = match headers.get("content-type") {
        Some(ct) => ct.to_lowercase(),
        None => return false, // No content-type, assume binary
    };

    // Allow text-based formats only
    content_type.contains("application/json")
        || content_type.contains("application/xml")
        || content_type.contains("text/xml")
        || content_type.contains("text/html")
        || content_type.contains("text/plain")
        || content_type.contains("text/")
        || content_type.contains("application/graphql")
}

/// Extracts response headers into a HashMap for easy lookup
fn extract_response_headers<B>(res: &ServiceResponse<B>) -> HashMap<String, String> {
    res.headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_lowercase(), v.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_capture_json_response() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        assert!(should_capture_body(&headers));
    }

    #[test]
    fn test_should_capture_html_response() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "text/html; charset=utf-8".to_string());
        assert!(should_capture_body(&headers));
    }

    #[test]
    fn test_skip_binary_image_response() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "image/jpeg".to_string());
        assert!(!should_capture_body(&headers));
    }

    #[test]
    fn test_skip_binary_octet_stream() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/octet-stream".to_string());
        assert!(!should_capture_body(&headers));
    }

    #[test]
    fn test_skip_missing_content_type_response() {
        let headers = HashMap::new();
        assert!(!should_capture_body(&headers));
    }
}

