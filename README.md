# LogSentinel Actix SDK

[![Crates.io](https://img.shields.io/crates/v/logsentinel-actix.svg)](https://crates.io/crates/logsentinel-actix)
[![Documentation](https://docs.rs/logsentinel-actix/badge.svg)](https://docs.rs/logsentinel-actix)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Non-intrusive request/response logging middleware for Actix-Web applications that forwards logs to a central LogSentinel server for AI-powered analysis.

## Features

- **Fail-Safe**: Never crashes your application, even with missing config or network errors
- **Non-Blocking**: Zero performance impact - all logging happens in background async tasks
- **Secure**: API keys are never logged, sensitive headers automatically filtered
- **Minimal**: Small dependency footprint, easy to integrate
- **Well-Tested**: Comprehensive unit and integration tests
- **Raw Logging**: Sends request/response data as-is, no modification

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
logsentinel-actix = "0.1"
```

## Quick Start

```rust
use actix_web::{App, HttpServer, web, HttpResponse};
use logsentinel_actix::LogSentinelMiddleware;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Configure via environment variables
    std::env::set_var("LOGSENTINEL_API_KEY", "your_api_key");
    std::env::set_var("LOGSENTINEL_BASE_URL", "https://logs.example.com");

    HttpServer::new(|| {
        App::new()
            // Add LogSentinel middleware - that's all you need!
            .wrap(LogSentinelMiddleware::new())
            .service(web::resource("/").to(|| async {
                HttpResponse::Ok().body("Hello!")
            }))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}
```

## Configuration

The SDK is configured via environment variables:

| Variable | Required | Description |
|----------|----------|-------------|
| `LOGSENTINEL_API_KEY` | Yes | Your LogSentinel API authentication key |
| `LOGSENTINEL_BASE_URL` | Yes | LogSentinel server URL (e.g., `https://logs.example.com`) |

### Fail-Safe Behavior

If configuration is missing or invalid:
- SDK disables itself gracefully
- Prints a warning to stderr
- **Never crashes your application**
- Requests continue to work normally

