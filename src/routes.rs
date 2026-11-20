//! HTTP handlers: public intake + status, admin triage, widget, health.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::AppState;
use crate::Config;

/// Maximum stored message length in Unicode scalars.
const MAX_MESSAGE_CHARS: usize = 5000;
/// Length of the public excerpt on status responses/pages.
const EXCERPT_CHARS: usize = 280;

// ─── Input / output models ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Useful,
    Bug,
    Feature,
}

#[derive(Debug, Deserialize)]
pub struct FeedbackInput {
    pub product: String,
    pub page: String,
    #[serde(default)]
    pub category: Option<Category>,
    pub message: String,
    /// Optional contact address. It is stored ONLY so the team can reply
    /// about this specific submission. It is never required, never shared,
    /// and never used for marketing — by policy and by schema (there are no
    /// marketing columns and no mailing-list code anywhere in this service).
    pub email: Option<String>,
    /// Honeypot: humans leave this empty; bots fill it and are silently
    /// dropped (fake success, nothing stored).
    #[serde(default)]
    pub website: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreatedFeedback {
    pub id: String,
    pub status_url: String,
}

#[derive(Debug, FromRow, Serialize)]
pub struct FeedbackRow {
    pub id: String,
    pub product: String,
    pub page: String,
    pub category: String,
    pub message: String,
    pub email: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Public status view: safe to expose to whoever holds the unguessable id.
#[derive(Debug, Serialize)]
pub struct StatusView {
    pub id: String,
    pub product: String,
    pub category: String,
    pub status: String,
    pub excerpt: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<FeedbackRow> for StatusView {
    fn from(row: FeedbackRow) -> Self {
        Self {
            id: row.id,
            product: row.product,
            category: row.category,
            status: row.status,
            excerpt: excerpt(&row.message),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct StatusUpdate {
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct QueueQuery {
    pub product: Option<String>,
    pub status: Option<String>,
}

// ─── Public handlers ──────────────────────────────────────────────────────

pub async fn create_feedback(
    State(state): State<Arc<AppState>>,
    Json(input): Json<FeedbackInput>,
) -> Result<(StatusCode, Json<CreatedFeedback>), (StatusCode, String)> {
    // Honeypot trip: pretend success, store nothing.
    if let Some(website) = &input.website {
        if !website.trim().is_empty() {
            let fake_id = new_id();
            return Ok((
                StatusCode::CREATED,
                Json(CreatedFeedback {
                    status_url: format!("/f/{fake_id}"),
                    id: fake_id,
                }),
            ));
        }
    }

    let validated = validate(input)?;
    let now = now_rfc3339();
    let id = new_id();

    sqlx::query(
        "INSERT INTO feedback (id, product, page, category, message, email, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'received', ?, ?)",
    )
    .bind(&id)
    .bind(&validated.product)
    .bind(&validated.page)
    .bind(category_str(validated.category))
    .bind(&validated.message)
    .bind(&validated.email)
    .bind(&now)
    .bind(&now)
    .execute(&state.pool)
    .await
    .map_err(internal_error)?;

    Ok((
        StatusCode::CREATED,
        Json(CreatedFeedback {
            status_url: format!("/f/{id}"),
            id,
        }),
    ))
}

pub async fn status_json(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<StatusView>, StatusCode> {
    let row = fetch_by_id(&state, &id).await?;
    Ok(Json(StatusView::from(row)))
}

pub async fn status_page(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Html<String>, StatusCode> {
    let row = fetch_by_id(&state, &id).await?;
    let view = StatusView::from(row);
    Ok(Html(status_page_html(&view)))
}

pub async fn healthz() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "ok",
    )
        .into_response()
}

pub async fn widget_js() -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        include_str!("../widget.js"),
    )
        .into_response()
}

// ─── Admin handlers (127.0.0.1 only) ──────────────────────────────────────

pub async fn admin_queue(
    State(state): State<Arc<AppState>>,
    Query(query): Query<QueueQuery>,
) -> Result<Json<Vec<FeedbackRow>>, (StatusCode, String)> {
    let rows = sqlx::query_as::<_, FeedbackRow>(
        "SELECT id, product, page, category, message, email, status, created_at, updated_at
         FROM feedback
         WHERE (? IS NULL OR product = ?) AND (? IS NULL OR status = ?)
         ORDER BY created_at DESC LIMIT 500",
    )
    .bind(query.product.clone())
    .bind(query.product)
    .bind(query.status.clone())
    .bind(query.status)
    .fetch_all(&state.pool)
    .await
    .map_err(internal_error)?;
    Ok(Json(rows))
}

pub async fn admin_set_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(update): Json<StatusUpdate>,
) -> Result<Json<StatusView>, StatusCode> {
    if !is_valid_status(&update.status) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let now = now_rfc3339();
    let result = sqlx::query("UPDATE feedback SET status = ?, updated_at = ? WHERE id = ?")
        .bind(&update.status)
        .bind(&now)
        .bind(&id)
        .execute(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }
    let row = fetch_by_id(&state, &id).await?;
    Ok(Json(StatusView::from(row)))
}

// ─── CORS ─────────────────────────────────────────────────────────────────

pub fn cors_layer(config: &Config) -> tower_http::cors::CorsLayer {
    use tower_http::cors::{AllowOrigin, CorsLayer};
    let origins: Vec<HeaderValue> = config
        .cors_origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([header::CONTENT_TYPE])
        .max_age(std::time::Duration::from_secs(7200))
}

// ─── Internals ────────────────────────────────────────────────────────────

struct ValidatedInput {
    product: String,
    page: String,
    category: Category,
    message: String,
    email: Option<String>,
}

fn validate(input: FeedbackInput) -> Result<ValidatedInput, (StatusCode, String)> {
    let product = input.product.trim().to_string();
    if product.is_empty() || product.chars().count() > 64 {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, "invalid product".into()));
    }
    let page = input.page.trim().to_string();
    if page.is_empty() || page.chars().count() > 512 || !page.starts_with('/') {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, "invalid page".into()));
    }
    let category = input.category.ok_or((
        StatusCode::UNPROCESSABLE_ENTITY,
        "category must be one of useful|bug|feature".to_string(),
    ))?;
    let message = input.message.trim().to_string();
    if message.is_empty() || message.chars().count() > MAX_MESSAGE_CHARS {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, "invalid message".into()));
    }
    // Email is optional; an empty string means "not provided". See the field
    // doc on FeedbackInput for the no-marketing policy.
    let email = input.email.and_then(|e| {
        let e = e.trim().to_string();
        if e.is_empty() {
            None
        } else {
            Some(e)
        }
    });
    if let Some(e) = &email {
        let ok = e.chars().count() <= 254
            && e.contains('@')
            && !e.chars().any(|c| c.is_whitespace() || c.is_control());
        if !ok {
            return Err((StatusCode::UNPROCESSABLE_ENTITY, "invalid email".into()));
        }
    }
    Ok(ValidatedInput {
        product,
        page,
        category,
        message,
        email,
    })
}

async fn fetch_by_id(state: &Arc<AppState>, id: &str) -> Result<FeedbackRow, StatusCode> {
    sqlx::query_as::<_, FeedbackRow>(
        "SELECT id, product, page, category, message, email, status, created_at, updated_at
         FROM feedback WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)
}

fn is_valid_status(status: &str) -> bool {
    matches!(status, "received" | "in_review" | "planned" | "resolved")
}

fn category_str(category: Category) -> &'static str {
    match category {
        Category::Useful => "useful",
        Category::Bug => "bug",
        Category::Feature => "feature",
    }
}

fn excerpt(message: &str) -> String {
    let mut chars = message.chars();
    let prefix: String = chars.by_ref().take(EXCERPT_CHARS).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

/// 128-bit unguessable public token (32 lowercase hex chars).
fn new_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    let mut hex = String::with_capacity(32);
    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    format_rfc3339(secs)
}

/// `YYYY-MM-DDTHH:MM:SSZ` from Unix seconds (UTC), no chrono dependency.
fn format_rfc3339(epoch_secs: i64) -> String {
    let days = epoch_secs.div_euclid(86_400);
    let sod = epoch_secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

/// Howard Hinnant's civil-from-days algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + i64::from(m <= 2), m as u32, d as u32)
}

fn internal_error(err: sqlx::Error) -> (StatusCode, String) {
    eprintln!("database error: {err}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal error".to_string(),
    )
}

fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn status_page_html(view: &StatusView) -> String {
    let title = format!("Feedback status — {}", view.product);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
body{{margin:0;font:15px/1.5 system-ui,sans-serif;background:#f9fafb;color:#111827}}
main{{max-width:560px;margin:48px auto;padding:0 16px}}
.card{{background:#fff;border:1px solid #e5e7eb;border-radius:12px;padding:24px}}
.badge{{display:inline-block;padding:4px 12px;border-radius:999px;background:#eef2ff;color:#3730a3;font-size:13px;text-transform:capitalize}}
dl{{display:grid;grid-template-columns:120px 1fr;gap:6px 12px;margin:16px 0}}
dt{{color:#6b7280}}
dd{{margin:0;overflow-wrap:anywhere}}
blockquote{{margin:16px 0 0;padding:12px;border-left:3px solid #d1d5db;background:#f9fafb;overflow-wrap:anywhere;white-space:pre-wrap}}
footer{{margin-top:16px;color:#9ca3af;font-size:12px}}
</style>
</head>
<body>
<main>
<div class="card">
<h1>Feedback status</h1>
<p><span class="badge">{status}</span></p>
<dl>
<dt>Product</dt><dd>{product}</dd>
<dt>Category</dt><dd>{category}</dd>
<dt>Submitted</dt><dd>{created}</dd>
<dt>Last update</dt><dd>{updated}</dd>
</dl>
<blockquote>{message}</blockquote>
</div>
<footer>No account needed — anyone with this link can view this status. The link is unguessable.</footer>
</main>
</body>
</html>"#,
        title = escape_html(&title),
        status = escape_html(&view.status),
        product = escape_html(&view.product),
        category = escape_html(&view.category),
        created = escape_html(&view.created_at),
        updated = escape_html(&view.updated_at),
        message = escape_html(&view.excerpt),
    )
}

#[cfg(test)]
mod tests {
    use super::{civil_from_days, format_rfc3339};

    #[test]
    fn rfc3339_from_epoch() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(86_400), "1970-01-02T00:00:00Z");
        assert_eq!(format_rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn civil_from_days_boundaries() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(18_992), (2021, 12, 31));
        assert_eq!(civil_from_days(19_417), (2023, 3, 1)); // non-leap year
        assert_eq!(civil_from_days(19_051), (2022, 2, 28));
    }
}
