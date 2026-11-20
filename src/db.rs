use std::str::FromStr;
use std::sync::Arc;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::rate::RateLimiter;
use crate::Config;

/// Shared application state handed to every handler.
pub struct AppState {
    pub pool: sqlx::SqlitePool,
    pub config: Config,
    pub rate_limiter: Arc<RateLimiter>,
}

impl AppState {
    pub fn new(pool: sqlx::SqlitePool, config: Config) -> Self {
        Self {
            pool,
            config,
            rate_limiter: Arc::new(RateLimiter::default()),
        }
    }
}

/// Open the SQLite pool: WAL mode, busy timeout, create-if-missing.
/// Accepts both `file:/path.db` (as used in /etc/feedback/env) and
/// `sqlite:/path.db` / `sqlite::memory:` URLs.
pub async fn connect(config: &Config) -> Result<sqlx::SqlitePool, sqlx::Error> {
    let url = config.database_url.trim();
    let options = if let Some(path) = url.strip_prefix("file:") {
        SqliteConnectOptions::from_str(&format!("sqlite:{path}"))?
    } else {
        SqliteConnectOptions::from_str(url)?
    };
    SqlitePoolOptions::new()
        .connect_with(
            options
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .busy_timeout(std::time::Duration::from_secs(5)),
        )
        .await
}
