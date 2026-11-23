//! Server configuration.
//!
//! Most settings come from environment variables so the operator can tune
//! limits without rebuilding.  Defaults are reasonable for a small
//! self-hosted install.

use std::env;

/// The repository operators offer when `THRUM_SOURCE_URL` is unset.
pub const DEFAULT_SOURCE_URL: &str = "https://github.com/BITS-Freeware/Thrum";

/// Runtime configuration. Rate limits are per client IP per minute unless
/// a setting says otherwise.
#[derive(Debug, Clone)]
pub struct Config {
    /// Maximum registration attempts per IP per minute.
    pub register_per_minute: u32,
    /// Maximum login attempts per IP per minute.
    pub login_per_minute: u32,
    /// Maximum login attempts per account per minute, on top of the IP limit.
    pub login_per_account_per_minute: u32,
    /// Failed logins against one account before the backoff lockout starts.
    pub login_failures_before_backoff: u32,
    /// First lockout length, in seconds. Each further failure doubles it.
    pub login_backoff_base_seconds: u64,
    /// Longest login lockout, in seconds.
    pub login_backoff_cap_seconds: u64,
    /// Failed password checks against one account per hour, from any
    /// address, before every address is refused for the rest of the hour.
    /// Zero means no ceiling.
    pub login_failures_per_account_per_hour: u32,
    /// Proxies whose forwarded client address may be trusted. Empty means
    /// forwarded headers are ignored.
    pub trusted_proxies: Vec<String>,
    /// Key-package fetches per authenticated user per minute.
    pub key_package_fetch_per_caller_per_minute: u32,
    /// Key-package fetches of one device by one user per minute.
    pub key_package_fetch_per_caller_device_per_minute: u32,
    /// Key-package fetches of one device per minute, from everyone. Past
    /// this, fetches get the device's last-resort package and leave its
    /// one-time stock alone.
    pub key_package_fetch_per_device_per_minute: u32,
    /// One-time key packages stored per device. The last-resort package
    /// does not count toward this.
    pub key_package_max_per_device: u32,
    /// Members in one conversation, including the creator. Zero means no cap.
    pub max_group_members: u32,
    /// Signed-in (not revoked) devices per account. Zero means no cap.
    pub max_devices_per_account: u32,
    /// Messages one device may send per minute.
    pub messages_per_device_per_minute: u32,
    /// Bytes of messages one account has sent that some device has not
    /// received yet. Zero means no cap.
    pub queued_bytes_per_account: u64,
    /// Link the web client shows for this server's source code.
    pub source_url: String,
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
            login_per_account_per_minute: 10,
            login_failures_before_backoff: 5,
            login_backoff_base_seconds: 30,
            login_backoff_cap_seconds: 15 * 60,
            login_failures_per_account_per_hour: 100,
            trusted_proxies: Vec::new(),
            key_package_fetch_per_caller_per_minute: 120,
            key_package_fetch_per_caller_device_per_minute: 10,
            key_package_fetch_per_device_per_minute: 60,
            key_package_max_per_device: 100,
            max_group_members: 50,
            max_devices_per_account: 20,
            messages_per_device_per_minute: 120,
            queued_bytes_per_account: 100 * 1024 * 1024,
            source_url: DEFAULT_SOURCE_URL.to_string(),
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
        cfg.register_per_minute = env_u32("THRUM_REGISTER_PER_MINUTE", cfg.register_per_minute);
        cfg.login_per_minute = env_u32("THRUM_LOGIN_PER_MINUTE", cfg.login_per_minute);
        cfg.login_per_account_per_minute = env_u32(
            "THRUM_LOGIN_PER_ACCOUNT_PER_MINUTE",
            cfg.login_per_account_per_minute,
        );
        cfg.login_failures_before_backoff = env_u32(
            "THRUM_LOGIN_FAILURES_BEFORE_BACKOFF",
            cfg.login_failures_before_backoff,
        );
        cfg.login_backoff_base_seconds = env_u64(
            "THRUM_LOGIN_BACKOFF_BASE_SECONDS",
            cfg.login_backoff_base_seconds,
        );
        cfg.login_backoff_cap_seconds = env_u64(
            "THRUM_LOGIN_BACKOFF_CAP_SECONDS",
            cfg.login_backoff_cap_seconds,
        );
        cfg.login_failures_per_account_per_hour = env_u32(
            "THRUM_LOGIN_FAILURES_PER_ACCOUNT_PER_HOUR",
            cfg.login_failures_per_account_per_hour,
        );
        if let Ok(value) = env::var("THRUM_TRUSTED_PROXIES") {
            cfg.trusted_proxies = value
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
                .collect();
        }
        cfg.key_package_fetch_per_caller_per_minute = env_u32(
            "THRUM_KEY_PACKAGE_FETCH_PER_CALLER_PER_MINUTE",
            cfg.key_package_fetch_per_caller_per_minute,
        );
        cfg.key_package_fetch_per_caller_device_per_minute = env_u32(
            "THRUM_KEY_PACKAGE_FETCH_PER_CALLER_DEVICE_PER_MINUTE",
            cfg.key_package_fetch_per_caller_device_per_minute,
        );
        cfg.key_package_fetch_per_device_per_minute = env_u32(
            "THRUM_KEY_PACKAGE_FETCH_PER_DEVICE_PER_MINUTE",
            cfg.key_package_fetch_per_device_per_minute,
        );
        cfg.key_package_max_per_device = env_u32(
            "THRUM_KEY_PACKAGE_MAX_PER_DEVICE",
            cfg.key_package_max_per_device,
        );
        cfg.max_group_members = env_u32("THRUM_MAX_GROUP_MEMBERS", cfg.max_group_members);
        cfg.max_devices_per_account =
            env_u32("THRUM_MAX_DEVICES_PER_ACCOUNT", cfg.max_devices_per_account);
        cfg.messages_per_device_per_minute = env_u32(
            "THRUM_MESSAGES_PER_DEVICE_PER_MINUTE",
            cfg.messages_per_device_per_minute,
        );
        cfg.queued_bytes_per_account = env_u64(
            "THRUM_QUEUED_BYTES_PER_ACCOUNT",
            cfg.queued_bytes_per_account,
        );
        if let Ok(value) = env::var("THRUM_SOURCE_URL")
            && !value.trim().is_empty()
        {
            cfg.source_url = value;
        }
        if let Ok(v) = env::var("THRUM_ATTACHMENT_MAX_BYTES") {
            cfg.attachment_max_bytes = v.parse().unwrap_or(cfg.attachment_max_bytes);
        }
        if let Ok(v) = env::var("THRUM_DAILY_UPLOAD_BYTES") {
            cfg.daily_upload_bytes = v.parse().unwrap_or(cfg.daily_upload_bytes);
        }
        if let Ok(v) = env::var("THRUM_BLOB_RETENTION_SECONDS") {
            cfg.blob_retention_seconds = v.parse().unwrap_or(cfg.blob_retention_seconds);
        }
        if let Ok(v) = env::var("THRUM_BLOB_BACKEND") {
            cfg.blob_backend = v;
        }
        if let Ok(v) = env::var("THRUM_BLOB_DIR") {
            cfg.blob_dir = v;
        }
        if let Ok(v) = env::var("THRUM_S3_ENDPOINT") {
            cfg.s3_endpoint = v;
        }
        if let Ok(v) = env::var("THRUM_S3_BUCKET") {
            cfg.s3_bucket = v;
        }
        if let Ok(v) = env::var("THRUM_S3_REGION") {
            cfg.s3_region = v;
        }
        if let Ok(v) = env::var("THRUM_S3_ACCESS_KEY_ID") {
            cfg.s3_access_key = v;
        }
        if let Ok(v) = env::var("THRUM_S3_SECRET_ACCESS_KEY") {
            cfg.s3_secret_key = v;
        }
        cfg
    }
}

fn env_u32(name: &str, default: u32) -> u32 {
    match env::var(name) {
        Ok(value) => value.parse().unwrap_or(default),
        Err(_) => default,
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    match env::var(name) {
        Ok(value) => value.parse().unwrap_or(default),
        Err(_) => default,
    }
}
