//! Shared application state.

use sqlx::SqlitePool;

use crate::config::Config;
use crate::db;
use crate::hub::WsHub;
use crate::ratelimit::RateLimiter;

/// Everything handlers need: the database pool, configuration, the rate
/// limiter and the live-connection hub. Cheap to clone; the pool, hub and
/// limiter share their internals.
#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub cfg: Config,
    pub limiter: RateLimiter,
    pub hub: WsHub,
}

impl AppState {
    /// Open the SQLite database at `path`, migrate it, and build the state.
    pub async fn connect_sqlite(path: &str, cfg: Config) -> sqlx::Result<Self> {
        let database = db::connect_sqlite(path).await?;
        Ok(Self {
            db: database,
            cfg,
            limiter: RateLimiter::new(),
            hub: WsHub::new(),
        })
    }
}
