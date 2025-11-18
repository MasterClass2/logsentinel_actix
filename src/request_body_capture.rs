//! Request body capture for LogSentinel middleware
//!
//! Captures incoming payloads without blocking handlers or consuming the stream.
//! Only text-based content types are captured to avoid binary bloat.

use actix_web::{dev::ServiceRequest, web::Bytes, HttpMessage};
use futures::StreamExt;
use std::collections::HashMap;
use std::pin::Pin;

const MAX_BODY_SIZE: usize = 10 * 1024; // 10KB limit for memory safety

/// Captures request body if it's text-based and under size limit
///
/// Returns None for binary content, oversized payloads, or on read errors.
/// The original payload remains available for downstream handlers.
pub async fn capture_request_body(req: &mut ServiceRequest) -> Option<String> {
    let headers = extract_request_headers(req);
    
    // Only capture text-based payloads to avoid binary bloat
    if !should_capture_body(&headers) {
        return None;
    }

    // Check Content-Length header to skip oversized payloads early
    if let Some(content_length) = headers.get("content-length") {
        if let Ok(size) = content_length.parse::<usize>() {
            if size > MAX_BODY_SIZE {
                return None; // Skip capture, will be truncated anyway
            }
        }
    }

    // Stream body into buffer without blocking (async operation)
    let bytes = match read_and_buffer_body(req).await {
        Ok(b) => b,
        Err(_) => return None, // Failed to read, skip capture
    };

    // Convert to UTF-8 string, handle invalid encoding gracefully
    match String::from_utf8(bytes.to_vec()) {
        Ok(body_str) => {
            // Truncate if exceeds max size
            if body_str.len() > MAX_BODY_SIZE {
                Some(body_str[..MAX_BODY_SIZE].to_string() + "...[truncated]")
            } else {
                Some(body_str)
            }
        }
        Err(_) => None, // Invalid UTF-8, skip capture
    }
}

/// Checks if body should be captured based on Content-Type
///
/// Returns true for text-based formats only (JSON, XML, form data, plain text)
fn should_capture_body(headers: &HashMap<String, String>) -> bool {
    let content_type = match headers.get("content-type") {
        Some(ct) => ct.to_lowercase(),
        None => return false, // No content-type, assume binary
    };

    // Allow text-based formats only
    content_type.contains("application/json")
        || content_type.contains("application/xml")
        || content_type.contains("text/xml")
        || content_type.contains("application/x-www-form-urlencoded")
        || content_type.contains("text/")
        || content_type.contains("application/graphql")
}

/// Reads request payload into memory buffer up to MAX_BODY_SIZE
///
/// This consumes the payload stream, so we must reconstruct it afterwards
/// for downstream handlers to access.
async fn read_and_buffer_body(req: &mut ServiceRequest) -> Result<Bytes, std::io::Error> {
    use actix_web::web::BytesMut;
    use std::io::{Error, ErrorKind};

    let mut payload = req.take_payload();
    let mut buffer = BytesMut::new();

    // Stream chunks into buffer until complete or size limit reached
    while let Some(chunk) = payload.next().await {
        let chunk = chunk.map_err(|e| Error::new(ErrorKind::Other, e))?;
        
        if buffer.len() + chunk.len() > MAX_BODY_SIZE {
            // Stop buffering, we'll truncate later
            buffer.extend_from_slice(&chunk[..MAX_BODY_SIZE - buffer.len()]);
            break;
        }
        
        buffer.extend_from_slice(&chunk);
    }

    // Reconstruct payload for downstream handlers
    // This is critical: handlers must still be able to read the body
    let bytes = buffer.freeze();
    let cloned_bytes = bytes.clone();
    
    // Re-insert payload into request using the bytes we just read
    use actix_web::dev::Payload;
    use actix_web::error::PayloadError;
    use futures::stream::{self, Stream};
    
    let new_payload = Payload::Stream {
        payload: Box::pin(stream::once(async move { 
            Ok::<_, PayloadError>(cloned_bytes) 
        })) as Pin<Box<dyn Stream<Item = Result<Bytes, PayloadError>>>>,
    };
    req.set_payload(new_payload);

    Ok(bytes)
}

/// Extracts request headers into a HashMap for easy lookup
fn extract_request_headers(req: &ServiceRequest) -> HashMap<String, String> {
    req.headers()
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
    fn test_should_capture_json() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        assert!(should_capture_body(&headers));
    }

    #[test]
    fn test_should_capture_xml() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/xml".to_string());
        assert!(should_capture_body(&headers));
    }

    #[test]
    fn test_should_capture_form_data() {
        let mut headers = HashMap::new();
        headers.insert(
            "content-type".to_string(),
            "application/x-www-form-urlencoded".to_string(),
        );
        assert!(should_capture_body(&headers));
    }

    #[test]
    fn test_skip_binary_image() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "image/png".to_string());
        assert!(!should_capture_body(&headers));
    }

    #[test]
    fn test_skip_binary_pdf() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/pdf".to_string());
        assert!(!should_capture_body(&headers));
    }

    #[test]
    fn test_skip_missing_content_type() {
        let headers = HashMap::new();
        assert!(!should_capture_body(&headers));
    }
}
