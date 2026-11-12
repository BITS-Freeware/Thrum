//! Shared application state.

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::blobs::BlobStore;
use crate::config::Config;
use crate::db;
use crate::hub::WsHub;
use crate::ratelimit::RateLimiter;

/// Everything handlers need: the database pool, configuration, the rate
/// limiter, the live-connection hub, and the blob storage backend.
/// Cheap to clone; the pool, hub, limiter and store share their internals.
#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub cfg: Config,
    pub limiter: RateLimiter,
    pub hub: WsHub,
    pub blob_store: Arc<dyn BlobStore>,
}

impl AppState {
    /// Open the SQLite database at `path`, migrate it, and build the state
    /// with a blob store chosen from `cfg.blob_backend`.
    pub async fn connect_sqlite(path: &str, cfg: Config) -> sqlx::Result<Self> {
        let database = db::connect_sqlite(path).await?;
        let blob_store: Arc<dyn BlobStore> = match cfg.blob_backend.as_str() {
            "s3" => Arc::new(
                crate::blobs::S3Store::new(&cfg)
                    .map_err(|e| sqlx::Error::Io(std::io::Error::other(e)))?,
            ),
            _ => Arc::new(crate::blobs::DiskStore::new(&cfg.blob_dir)?),
        };
        Ok(Self {
            db: database,
            cfg,
            limiter: RateLimiter::new(),
            hub: WsHub::new(),
            blob_store,
        })
    }
}
