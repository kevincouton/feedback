//! Per-IP token-bucket rate limiting for submission intake.
//!
//! Default policy: 5 submissions per 10 minutes per IP. Rejects with
//! 429 + Retry-After. Hand-rolled (no tower-governor dep) to stay small,
//! dependency-free, and easy to unit-test; buckets live in memory and do
//! not survive restarts, which is acceptable for a single-node service.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

/// Token bucket capacity (max burst).
pub const CAPACITY: f64 = 5.0;
/// Full refill interval: 5 tokens per 600 seconds.
pub const REFILL_INTERVAL: Duration = Duration::from_secs(600);

const REFILL_RATE: f64 = CAPACITY / REFILL_INTERVAL.as_secs() as f64;

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl Bucket {
    fn new(now: Instant) -> Self {
        Self {
            tokens: CAPACITY,
            last_refill: now,
        }
    }

    /// Refill based on elapsed time, then try to consume one token.
    /// Returns `Some(retry_after)` when the bucket is empty.
    fn take(&mut self, now: Instant) -> Option<Duration> {
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * REFILL_RATE).min(CAPACITY);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            return None;
        }
        let deficit = 1.0 - self.tokens;
        let secs = (deficit / REFILL_RATE).ceil().max(1.0) as u64;
        Some(Duration::from_secs(secs))
    }
}

/// In-memory per-IP token buckets.
#[derive(Debug, Default)]
pub struct RateLimiter {
    buckets: Mutex<HashMap<std::net::IpAddr, Bucket>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one attempt from `ip`. `Ok(())` = allowed; `Err(retry_after)`
    /// = rejected, retry after the returned duration.
    pub fn check(&self, ip: std::net::IpAddr) -> Result<(), Duration> {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().expect("rate limiter poisoned");
        let bucket = buckets.entry(ip).or_insert_with(|| Bucket::new(now));
        bucket.take(now).map_or(Ok(()), Err)
    }
}

/// Axum middleware: applies the per-IP bucket to POST /api/feedback only.
/// Other routes (status pages, widget) are not intake and stay unlimited.
///
/// IP resolution: behind a proxy (this fleet sits behind Caddy, and
/// feedback.lucanian.app currently resolves via Cloudflare) the direct peer
/// is the proxy, so a present `CF-Connecting-IP` header takes precedence;
/// otherwise the socket peer IP is used. Spoofing that header only rotates
/// the per-identity bucket, which still caps each identity at the same rate.
pub async fn rate_limit_middleware(
    State(limiter): State<Arc<RateLimiter>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if request.method() != axum::http::Method::POST || request.uri().path() != "/api/feedback" {
        return next.run(request).await;
    }
    let ip = request
        .headers()
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| addr.ip());
    match limiter.check(ip) {
        Ok(()) => next.run(request).await,
        Err(retry_after) => {
            let retry_secs = retry_after.as_secs().max(1);
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", retry_secs.to_string())],
                format!("rate limit exceeded, retry in {retry_secs}s"),
            )
                .into_response()
        }
    }
}
