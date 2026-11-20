//! Feedback service for the lucanian.app product fleet.
//!
//! Library crate so integration tests can build routers without TCP.
//! See `main.rs` for the binary entrypoint.

mod config;
mod db;
mod rate;
mod routes;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

pub use config::Config;
pub use db::{connect, AppState};
pub use rate::RateLimiter;

/// Public router: everything a browser may hit, behind Caddy.
pub fn public_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/feedback", post(routes::create_feedback))
        .route("/api/feedback/{id}/status", get(routes::status_json))
        .route("/f/{id}", get(routes::status_page))
        .route("/healthz", get(routes::healthz))
        .route("/widget.js", get(routes::widget_js))
        .layer(axum::middleware::from_fn_with_state(
            state.rate_limiter.clone(),
            rate::rate_limit_middleware,
        ))
        .layer(routes::cors_layer(&state.config))
        .with_state(state)
}

/// Admin router: bound to 127.0.0.1 only; no auth beyond loopback.
pub fn admin_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/admin/queue", get(routes::admin_queue))
        .route(
            "/admin/feedback/{id}/status",
            post(routes::admin_set_status),
        )
        .with_state(state)
}

/// Helper for tests and the binary: shared "who is calling" info type.
pub type ConnInfo = SocketAddr;
