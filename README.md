# LogSentinel Actix SDK

[![Crates.io](https://img.shields.io/crates/v/logsentinel-actix.svg)](https://crates.io/crates/logsentinel-actix)
[![Documentation](https://docs.rs/logsentinel-actix/badge.svg)](https://docs.rs/logsentinel-actix)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Non-intrusive request/response logging middleware for Actix-Web applications that forwards logs to a central LogSentinel server for AI-powered analysis.

## Features

- 🔒 **Fail-Safe**: Never crashes your application, even with missing config or network errors
- ⚡ **Non-Blocking**: Zero performance impact - all logging happens in background async tasks
- 🛡️ **Secure**: API keys are never logged, sensitive headers automatically filtered
- 📦 **Minimal**: Small dependency footprint, easy to integrate
- 🧪 **Well-Tested**: Comprehensive unit and integration tests
- 📝 **Raw Logging**: Sends request/response data as-is, no modification

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

Example warning:
```
⚠️  LogSentinel SDK disabled: missing LOGSENTINEL_API_KEY or LOGSENTINEL_BASE_URL environment variable
```

## How It Works

1. **Request Phase**: Middleware captures request metadata (method, path, headers, IP)
2. **Handler Execution**: Request passes through to your handlers (no blocking)
3. **Response Phase**: Response metadata is captured (status, headers, duration)
4. **Background Logging**: Log is sent asynchronously via `tokio::spawn`
5. **Error Handling**: Any errors are logged to stderr but never interrupt the app

### What Gets Logged

**Request Data:**
- HTTP method (GET, POST, etc.)
- Request path
- Headers (sanitized - auth tokens removed)
- Client IP address
- Timestamp

**Response Data:**
- HTTP status code
- Response headers
- Request processing duration

**Security:**
- Sensitive headers are automatically filtered (Authorization, Cookie, etc.)
- API keys are never logged
- Body size is limited to prevent memory issues

## Architecture

The SDK is organized into focused modules:

```
src/
├── lib.rs          # Public API and documentation
├── middleware.rs   # Actix-Web middleware implementation
├── config.rs       # Environment-based configuration
├── logger.rs       # Log event structure and forwarding
├── client.rs       # HTTP client for sending logs
├── error.rs        # Custom error types
└── utils.rs        # Helper functions
```

## Testing

Run the test suite:

```bash
cargo test
```

Run the example application:

```bash
LOGSENTINEL_API_KEY=demo_key \
LOGSENTINEL_BASE_URL=http://localhost:9000 \
cargo run --example basic_usage
```

## Performance

The SDK is designed to have **zero performance impact** on your application:

- Logging happens in background async tasks via `tokio::spawn`
- Main request/response cycle never awaits network operations
- Failed log transmissions don't retry (prevents cascading delays)
- Memory footprint is minimal (no buffering/batching)

## Error Handling

All errors are caught internally and handled gracefully:

```rust
pub enum LogError {
    MissingConfig,       // Config invalid - SDK disables itself
    Network(Error),      // Network error - logged to stderr
    SendFailed(Status),  // HTTP error - logged to stderr
    Serialization(Error), // JSON error - logged to stderr
}
```

Errors are printed to stderr but **never panic or stop your application**.

## Advanced Usage

### Custom Configuration

```rust
use logsentinel_actix::{Config, LogSentinelMiddleware};

let config = Config::new(
    Some("api_key".to_string()),
    Some("https://logs.example.com".to_string()),
);

App::new()
    .wrap(LogSentinelMiddleware::with_config(config))
    .service(...)
```

### Using the Prelude

```rust
use logsentinel_actix::prelude::*;

// Imports: LogSentinelMiddleware, Config, LogError
```

## Examples

See the [examples/](examples/) directory for complete working examples:

- `basic_usage.rs` - Simple integration example

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Support

For issues, questions, or feature requests, please open an issue on GitHub.

## Changelog

### 0.1.0 (Initial Release)
- ✅ Core middleware implementation
- ✅ Environment-based configuration
- ✅ Non-blocking async logging
- ✅ Fail-safe error handling
- ✅ Comprehensive test coverage
- ✅ Example applications
- ✅ Full documentation
