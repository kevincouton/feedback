//! Feedback service binary: public listener (Caddy-facing) + localhost-only
//! admin listener for the w8 triage workflow.

use std::net::SocketAddr;
use std::sync::Arc;

use feedback_server::{admin_router, connect, public_router, AppState, Config};

#[tokio::main]
async fn main() {
    let config = Config::from_env();
    if config.sentry_dsn.is_some() {
        eprintln!("SENTRY_DSN is set but no Sentry SDK is wired yet; logging locally only");
    }

    let pool = connect(&config).await.expect("connect to sqlite");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    let state = Arc::new(AppState::new(pool, config.clone()));

    let admin_addr = SocketAddr::new([127, 0, 0, 1].into(), config.admin_port);
    let admin_state = Arc::clone(&state);
    let admin = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(admin_addr)
            .await
            .expect("bind admin listener on 127.0.0.1");
        axum::serve(listener, admin_router(admin_state))
            .await
            .expect("admin server");
    });

    let public_addr = SocketAddr::new([0, 0, 0, 0].into(), config.api_port);
    let listener = tokio::net::TcpListener::bind(public_addr)
        .await
        .expect("bind public listener");
    axum::serve(
        listener,
        public_router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .expect("public server");

    admin.abort();
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
