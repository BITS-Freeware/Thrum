//! Server configuration.
//!
//! Most settings come from environment variables so the operator can tune
//! limits without rebuilding.  Defaults are reasonable for a small
//! self-hosted install.

use std::env;

/// Runtime configuration. Rate limits are per client IP per minute.
#[derive(Debug, Clone)]
pub struct Config {
    /// Maximum registration attempts per IP per minute.
    pub register_per_minute: u32,
    /// Maximum login attempts per IP per minute.
    pub login_per_minute: u32,
    /// Largest accepted message payload in bytes.
    pub message_max_bytes: usize,
    /// How long the server keeps undelivered messages, in seconds.
    pub queue_ttl_seconds: i64,
    /// Largest accepted attachment blob in bytes.
    pub attachment_max_bytes: usize,
    /// Per-user daily upload quota (bytes).
    pub daily_upload_bytes: u64,
    /// How long to keep blobs before expiry, in seconds.
    pub blob_retention_seconds: i64,
    /// Storage backend: "disk" or "s3".
    pub blob_backend: String,
    /// Local directory for disk-backed blobs.
    pub blob_dir: String,
    /// S3-compatible endpoint (e.g. `https://r2.cloudflarestorage.com`).
    pub s3_endpoint: String,
    /// S3 bucket name.
    pub s3_bucket: String,
    /// S3 region.
    pub s3_region: String,
    /// S3 access key id.
    pub s3_access_key: String,
    /// S3 secret access key.
    pub s3_secret_key: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            register_per_minute: 20,
            login_per_minute: 60,
            message_max_bytes: 256 * 1024,
            queue_ttl_seconds: 7 * 24 * 60 * 60,
            attachment_max_bytes: 20 * 1024 * 1024,
            daily_upload_bytes: 100 * 1024 * 1024,
            blob_retention_seconds: 30 * 24 * 60 * 60,
            blob_backend: "disk".to_string(),
            blob_dir: "blobs".to_string(),
            s3_endpoint: String::new(),
            s3_bucket: String::new(),
            s3_region: String::new(),
            s3_access_key: String::new(),
            s3_secret_key: String::new(),
        }
    }
}

impl Config {
    /// Build configuration from environment variables.
    ///
    /// Unrecognised or unparseable values fall back to the defaults.
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = env::var("GEIST3_ATTACHMENT_MAX_BYTES") {
            cfg.attachment_max_bytes = v.parse().unwrap_or(cfg.attachment_max_bytes);
        }
        if let Ok(v) = env::var("GEIST3_DAILY_UPLOAD_BYTES") {
            cfg.daily_upload_bytes = v.parse().unwrap_or(cfg.daily_upload_bytes);
        }
        if let Ok(v) = env::var("GEIST3_BLOB_RETENTION_SECONDS") {
            cfg.blob_retention_seconds = v.parse().unwrap_or(cfg.blob_retention_seconds);
        }
        if let Ok(v) = env::var("GEIST3_BLOB_BACKEND") {
            cfg.blob_backend = v;
        }
        if let Ok(v) = env::var("GEIST3_BLOB_DIR") {
            cfg.blob_dir = v;
        }
        if let Ok(v) = env::var("GEIST3_S3_ENDPOINT") {
            cfg.s3_endpoint = v;
        }
        if let Ok(v) = env::var("GEIST3_S3_BUCKET") {
            cfg.s3_bucket = v;
        }
        if let Ok(v) = env::var("GEIST3_S3_REGION") {
            cfg.s3_region = v;
        }
        if let Ok(v) = env::var("GEIST3_S3_ACCESS_KEY_ID") {
            cfg.s3_access_key = v;
        }
        if let Ok(v) = env::var("GEIST3_S3_SECRET_ACCESS_KEY") {
            cfg.s3_secret_key = v;
        }
        cfg
    }
}
