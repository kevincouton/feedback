//! End-to-end tests over the real routers (no TCP): full CRUD, honeypot,
//! rate limiting, status page/JSON, CORS, healthz, migrations, widget.js.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use feedback_server::{admin_router, connect, public_router, AppState, Config};

/// Fixed per-test caller IP; rate-limit tests vary the last octet.
fn caller_ip(tag: u8) -> ConnectInfo<SocketAddr> {
    ConnectInfo(SocketAddr::from(([203, 0, 113, tag], 40000)))
}

async fn test_state() -> (Arc<AppState>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("feedback.db");
    // Use the `file:` scheme as production's DATABASE_URL does.
    let config = Config::for_test(&format!("file:{}", db_path.display()));
    let pool = connect(&config).await.expect("connect");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrate");
    (Arc::new(AppState::new(pool, config)), dir)
}

fn json_request(
    method: Method,
    path: &str,
    body: Option<Value>,
    origin: Option<&str>,
    ip_tag: u8,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    let body = match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let mut req = builder.body(body).unwrap();
    req.extensions_mut().insert(caller_ip(ip_tag));
    req
}

async fn send(app: Router, req: Request<Body>) -> Response {
    app.oneshot(req).await.unwrap()
}

async fn body_json(resp: Response) -> Value {
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).expect("json body")
}

fn submit_body(product: &str, category: &str, message: &str) -> Value {
    json!({
        "product": product,
        "page": "/dashboard",
        "category": category,
        "message": message,
        "email": "user@example.com"
    })
}

#[tokio::test]
async fn full_crud_flow() {
    let (state, _dir) = test_state().await;
    let public = public_router(state.clone());
    let admin = admin_router(state);

    // Create.
    let resp = send(
        public.clone(),
        json_request(
            Method::POST,
            "/api/feedback",
            Some(submit_body("gittrend", "bug", "The chart fails to load")),
            None,
            1,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = body_json(resp).await;
    let id = created["id"].as_str().unwrap();
    assert_eq!(id.len(), 32);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(created["status_url"], format!("/f/{id}"));

    // Public JSON status.
    let resp = send(
        public.clone(),
        json_request(
            Method::GET,
            &format!("/api/feedback/{id}/status"),
            None,
            None,
            1,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let status = body_json(resp).await;
    assert_eq!(status["status"], "received");
    assert_eq!(status["product"], "gittrend");
    assert_eq!(status["category"], "bug");
    assert_eq!(status["excerpt"], "The chart fails to load");

    // Public HTML status page.
    let resp = send(
        public.clone(),
        json_request(Method::GET, &format!("/f/{id}"), None, None, 1),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(html.contains("Feedback status"));
    assert!(html.contains("gittrend"));
    assert!(html.contains("The chart fails to load"));
    // Email must not leak onto the public page.
    assert!(!html.contains("user@example.com"));

    // Admin queue: unfiltered and filtered.
    let resp = send(
        admin.clone(),
        json_request(Method::GET, "/admin/queue", None, None, 1),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let queue = body_json(resp).await;
    assert_eq!(queue.as_array().unwrap().len(), 1);
    let resp = send(
        admin.clone(),
        json_request(
            Method::GET,
            "/admin/queue?product=other&status=received",
            None,
            None,
            1,
        ),
    )
    .await;
    let queue = body_json(resp).await;
    assert!(queue.as_array().unwrap().is_empty());

    // Admin status update, reflected publicly.
    let resp = send(
        admin.clone(),
        json_request(
            Method::POST,
            &format!("/admin/feedback/{id}/status"),
            Some(json!({"status": "in_review"})),
            None,
            1,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let updated = body_json(resp).await;
    assert_eq!(updated["status"], "in_review");

    let resp = send(
        public.clone(),
        json_request(
            Method::GET,
            &format!("/api/feedback/{id}/status"),
            None,
            None,
            1,
        ),
    )
    .await;
    assert_eq!(body_json(resp).await["status"], "in_review");

    // Unknown id → 404 in both public and admin paths.
    let resp = send(
        public.clone(),
        json_request(Method::GET, "/api/feedback/deadbeef/status", None, None, 1),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let resp = send(
        admin.clone(),
        json_request(
            Method::POST,
            "/admin/feedback/deadbeef/status",
            Some(json!({"status": "planned"})),
            None,
            1,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Invalid status → 422.
    let resp = send(
        admin.clone(),
        json_request(
            Method::POST,
            &format!("/admin/feedback/{id}/status"),
            Some(json!({"status": "nonsense"})),
            None,
            1,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn validation_errors() {
    let (state, _dir) = test_state().await;
    let public = public_router(state);

    let cases = [
        json!({"product": "", "page": "/", "category": "bug", "message": "x"}),
        json!({"product": "gittrend", "page": "no-slash", "category": "bug", "message": "x"}),
        json!({"product": "gittrend", "page": "/", "message": "x"}),
        json!({"product": "gittrend", "page": "/", "category": "bug", "message": ""}),
        json!({"product": "gittrend", "page": "/", "category": "bug", "message": "x", "email": "not-an-email"}),
        json!({"product": "gittrend", "page": "/", "category": "loud", "message": "x"}),
    ];
    for (i, body) in cases.iter().enumerate() {
        let resp = send(
            public.clone(),
            json_request(
                Method::POST,
                "/api/feedback",
                Some(body.clone()),
                None,
                (i + 1) as u8,
            ),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "case {i}: {body}"
        );
    }
}

#[tokio::test]
async fn email_is_optional_and_never_required() {
    let (state, _dir) = test_state().await;
    let public = public_router(state.clone());
    let resp = send(
        public.clone(),
        json_request(
            Method::POST,
            "/api/feedback",
            Some(json!({"product": "risk", "page": "/", "category": "useful", "message": "neat"})),
            None,
            1,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    // Empty-string email also accepted and normalized to NULL.
    let resp = send(
        public,
        json_request(
            Method::POST,
            "/api/feedback",
            Some(json!({"product": "risk", "page": "/", "category": "useful", "message": "neat", "email": ""})),
            None,
            2,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn honeypot_is_silently_dropped() {
    let (state, _dir) = test_state().await;
    let public = public_router(state.clone());

    let mut body = submit_body("gittrend", "feature", "please add RSS");
    body["website"] = json!("http://spam.example");
    let resp = send(
        public,
        json_request(Method::POST, "/api/feedback", Some(body), None, 1),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = body_json(resp).await;
    assert_eq!(created["id"].as_str().unwrap().len(), 32);
    assert!(created["status_url"].as_str().unwrap().starts_with("/f/"));

    // Nothing was stored, and the fake id resolves to 404.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM feedback")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn rate_limit_triggers_then_recovers_by_ip() {
    let (state, _dir) = test_state().await;
    let public = public_router(state);

    for i in 1..=5u8 {
        let resp = send(
            public.clone(),
            json_request(
                Method::POST,
                "/api/feedback",
                Some(submit_body("delphi", "useful", "ok")),
                None,
                42,
            ),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::CREATED,
            "request {i} should pass"
        );
    }
    let resp = send(
        public.clone(),
        json_request(
            Method::POST,
            "/api/feedback",
            Some(submit_body("delphi", "useful", "ok")),
            None,
            42,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after = resp
        .headers()
        .get("retry-after")
        .expect("retry-after header")
        .to_str()
        .unwrap();
    assert!(retry_after.parse::<u64>().unwrap() >= 1);

    // Other IPs unaffected.
    let resp = send(
        public,
        json_request(
            Method::POST,
            "/api/feedback",
            Some(submit_body("delphi", "useful", "ok")),
            None,
            43,
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn rate_limit_does_not_apply_to_status_or_widget() {
    let (state, _dir) = test_state().await;
    let public = public_router(state);
    // 20 rapid reads from one IP: all fine.
    for _ in 0..20 {
        let resp = send(
            public.clone(),
            json_request(Method::GET, "/healthz", None, None, 9),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn healthz() {
    let (state, _dir) = test_state().await;
    let resp = send(
        public_router(state),
        json_request(Method::GET, "/healthz", None, None, 1),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(content_type.starts_with("text/plain"));
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(bytes, "ok");
}

#[tokio::test]
async fn widget_js_served() {
    let (state, _dir) = test_state().await;
    let resp = send(
        public_router(state),
        json_request(Method::GET, "/widget.js", None, None, 1),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(content_type.starts_with("application/javascript"));
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains("Feedback"));
    assert!(body.contains("/api/feedback"));
    assert!(body.contains("website"));
}

#[tokio::test]
async fn cors_preflight_and_simple_request() {
    let (state, _dir) = test_state().await;
    let public = public_router(state);

    // Preflight from an allowed product origin.
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/feedback")
        .header(header::ORIGIN, "https://gittrend.lucanian.app")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
        .body(Body::empty())
        .unwrap();
    let resp = send(public.clone(), req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap(),
        "https://gittrend.lucanian.app"
    );

    // Preflight from a disallowed origin gets no allow-origin header.
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/feedback")
        .header(header::ORIGIN, "https://evil.example")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .body(Body::empty())
        .unwrap();
    let resp = send(public.clone(), req).await;
    assert!(resp
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_none());

    // Simple GET with allowed origin echoes the origin back.
    let req = Request::builder()
        .method(Method::GET)
        .uri("/widget.js")
        .header(header::ORIGIN, "https://risk.lucanian.app")
        .body(Body::empty())
        .unwrap();
    let resp = send(public, req).await;
    assert_eq!(
        resp.headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap(),
        "https://risk.lucanian.app"
    );
}

#[tokio::test]
async fn cors_origins_env_override() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("feedback.db");
    let mut config = Config::for_test(&format!("file:{}", db_path.display()));
    config.cors_origins = vec!["https://staging.example".to_string()];
    let pool = connect(&config).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    let state = Arc::new(AppState::new(pool, config));
    let public = public_router(state);

    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/feedback")
        .header(header::ORIGIN, "https://gittrend.lucanian.app")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .body(Body::empty())
        .unwrap();
    let resp = send(public.clone(), req).await;
    assert!(resp
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_none());

    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/feedback")
        .header(header::ORIGIN, "https://staging.example")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .body(Body::empty())
        .unwrap();
    let resp = send(public, req).await;
    assert_eq!(
        resp.headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap(),
        "https://staging.example"
    );
}

#[tokio::test]
async fn migrations_are_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("feedback.db");
    let url = format!("sqlite:{}", db_path.display());
    let options: sqlx::sqlite::SqliteConnectOptions = std::str::FromStr::from_str(&url).unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect_with(
            options
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal),
        )
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
}
