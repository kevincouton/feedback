use std::str::FromStr;

/// Runtime configuration, sourced from the environment
/// (systemd EnvironmentFile=/etc/feedback/env).
#[derive(Clone, Debug)]
pub struct Config {
    /// Public listen port (behind Caddy).
    pub api_port: u16,
    /// Admin listen port (127.0.0.1 only).
    pub admin_port: u16,
    /// SQLite URL, e.g. `file:/var/lib/feedback/feedback.db` or `sqlite:…`.
    pub database_url: String,
    /// Optional Sentry DSN. Accepted for forward-compat; not wired to an SDK yet.
    pub sentry_dsn: Option<String>,
    /// Allowed CORS origins. Defaults to the deployed fleet product origins.
    pub cors_origins: Vec<String>,
}

/// Default public origins: the actually deployed product hostnames
/// (verified against /etc/caddy/Caddyfile, 2026-11-20). health-travel-aggregator
/// is not deployed publicly yet, so it is intentionally absent.
pub const DEFAULT_ORIGINS: &[&str] = &[
    "https://gittrend.lucanian.app",
    "https://delphi.lucanian.app",
    "https://fablayer.lucanian.app",
    "https://finroadmap.lucanian.app",
    "https://handbook.lucanian.app",
    "https://marketplace.lucanian.app",
    "https://risk.lucanian.app",
];

impl Config {
    pub fn from_env() -> Self {
        Self {
            api_port: env_u16("API_PORT", 31012),
            admin_port: env_u16("ADMIN_PORT", 31013),
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "file:/var/lib/feedback/feedback.db".to_string()),
            sentry_dsn: std::env::var("SENTRY_DSN").ok().filter(|v| !v.is_empty()),
            cors_origins: std::env::var("CORS_ORIGINS")
                .map(|v| {
                    v.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .ok()
                .filter(|v: &Vec<String>| !v.is_empty())
                .unwrap_or_else(|| DEFAULT_ORIGINS.iter().map(|s| s.to_string()).collect()),
        }
    }

    /// Test helper: ephemeral in-memory config.
    pub fn for_test(database_url: &str) -> Self {
        Self {
            api_port: 0,
            admin_port: 0,
            database_url: database_url.to_string(),
            sentry_dsn: None,
            cors_origins: DEFAULT_ORIGINS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

fn env_u16(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|v| u16::from_str(&v).ok())
        .unwrap_or(default)
}
