//! Shared application state.

use sqlx::SqlitePool;

use crate::config::Config;
use crate::db;
use crate::ratelimit::RateLimiter;

/// Everything handlers need: the database pool, configuration and the rate
/// limiter. Cheap to clone; the pool and limiter share their internals.
#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub cfg: Config,
    pub limiter: RateLimiter,
}

impl AppState {
    /// Open the SQLite database at `path`, migrate it, and build the state.
    pub async fn connect_sqlite(path: &str, cfg: Config) -> sqlx::Result<Self> {
        let database = db::connect_sqlite(path).await?;
        Ok(Self {
            db: database,
            cfg,
            limiter: RateLimiter::new(),
        })
    }
}
