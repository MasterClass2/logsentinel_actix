//! Basic usage example for LogSentinel Actix SDK
//!
//! This example demonstrates how to integrate the middleware into an Actix-Web application.
//!
//! Run with:
//! ```bash
//! LOGSENTINEL_API_KEY=demo_key_123 \
//! LOGSENTINEL_BASE_URL=http://localhost:9000 \
//! cargo run --example basic_usage
//! ```

use actix_web::{web, App, HttpResponse, HttpServer};
use logsentinel_actix::LogSentinelMiddleware;

async fn index() -> HttpResponse {
    HttpResponse::Ok().body("Hello LogSentinel!")
}

async fn health() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "healthy",
        "service": "example-app"
    }))
}

async fn echo(body: String) -> HttpResponse {
    HttpResponse::Ok().body(body)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Set environment variables for demo
    // In production, these would be set in your deployment environment
    std::env::set_var("LOGSENTINEL_API_KEY", "demo_api_key_123");
    std::env::set_var("LOGSENTINEL_BASE_URL", "http://localhost:9000");

    println!("🚀 Starting example server on http://0.0.0.0:8080");
    println!("📊 LogSentinel middleware is active");
    println!("\nTry these endpoints:");
    println!("  GET  http://localhost:8080/");
    println!("  GET  http://localhost:8080/health");
    println!("  POST http://localhost:8080/echo");

    HttpServer::new(|| {
        App::new()
            // Add LogSentinel middleware - that's all you need!
            .wrap(LogSentinelMiddleware::new())
            // Your normal routes
            .service(web::resource("/").route(web::get().to(index)))
            .service(web::resource("/health").route(web::get().to(health)))
            .service(web::resource("/echo").route(web::post().to(echo)))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}
