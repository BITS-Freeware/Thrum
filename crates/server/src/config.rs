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
#[derive(Clone)]
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
    /// A value that does not parse, an unknown blob backend, or a trusted
    /// proxy that is not an address or CIDR is an error naming the
    /// variable, so a typo never leaves a default silently in force.
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|name| env::var(name).ok())
    }

    /// [`Config::from_env`] with variables read through `get`.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let mut cfg = Self::default();
        let mut errors = Vec::new();
        // The closure borrows `errors` until this block ends.
        {
            let mut number = |name: &str, field: &mut dyn FnMut(&str) -> bool| {
                if let Some(value) = get(name)
                    && !field(value.trim())
                {
                    errors.push(format!("{name}={value:?} is not a valid number"));
                }
            };
            macro_rules! num {
                ($name:literal, $field:expr) => {
                    number($name, &mut |value| match value.parse() {
                        Ok(parsed) => {
                            $field = parsed;
                            true
                        }
                        Err(_) => false,
                    })
                };
            }
            num!("THRUM_REGISTER_PER_MINUTE", cfg.register_per_minute);
            num!("THRUM_LOGIN_PER_MINUTE", cfg.login_per_minute);
            num!(
                "THRUM_LOGIN_PER_ACCOUNT_PER_MINUTE",
                cfg.login_per_account_per_minute
            );
            num!(
                "THRUM_LOGIN_FAILURES_BEFORE_BACKOFF",
                cfg.login_failures_before_backoff
            );
            num!(
                "THRUM_LOGIN_BACKOFF_BASE_SECONDS",
                cfg.login_backoff_base_seconds
            );
            num!(
                "THRUM_LOGIN_BACKOFF_CAP_SECONDS",
                cfg.login_backoff_cap_seconds
            );
            num!(
                "THRUM_LOGIN_FAILURES_PER_ACCOUNT_PER_HOUR",
                cfg.login_failures_per_account_per_hour
            );
            num!(
                "THRUM_KEY_PACKAGE_FETCH_PER_CALLER_PER_MINUTE",
                cfg.key_package_fetch_per_caller_per_minute
            );
            num!(
                "THRUM_KEY_PACKAGE_FETCH_PER_CALLER_DEVICE_PER_MINUTE",
                cfg.key_package_fetch_per_caller_device_per_minute
            );
            num!(
                "THRUM_KEY_PACKAGE_FETCH_PER_DEVICE_PER_MINUTE",
                cfg.key_package_fetch_per_device_per_minute
            );
            num!(
                "THRUM_KEY_PACKAGE_MAX_PER_DEVICE",
                cfg.key_package_max_per_device
            );
            num!("THRUM_MAX_GROUP_MEMBERS", cfg.max_group_members);
            num!("THRUM_MAX_DEVICES_PER_ACCOUNT", cfg.max_devices_per_account);
            num!(
                "THRUM_MESSAGES_PER_DEVICE_PER_MINUTE",
                cfg.messages_per_device_per_minute
            );
            num!(
                "THRUM_QUEUED_BYTES_PER_ACCOUNT",
                cfg.queued_bytes_per_account
            );
            num!("THRUM_ATTACHMENT_MAX_BYTES", cfg.attachment_max_bytes);
            num!("THRUM_DAILY_UPLOAD_BYTES", cfg.daily_upload_bytes);
            num!("THRUM_BLOB_RETENTION_SECONDS", cfg.blob_retention_seconds);
        }

        if let Some(value) = get("THRUM_TRUSTED_PROXIES") {
            cfg.trusted_proxies = value
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
                .collect();
            for entry in &cfg.trusted_proxies {
                if crate::client_ip::TrustedNet::parse(entry).is_none() {
                    errors.push(format!(
                        "THRUM_TRUSTED_PROXIES entry {entry:?} is not an address or CIDR \
                         (separate entries with commas)"
                    ));
                }
            }
        }
        if let Some(value) = get("THRUM_SOURCE_URL")
            && !value.trim().is_empty()
        {
            cfg.source_url = value;
        }
        if let Some(value) = get("THRUM_BLOB_BACKEND") {
            cfg.blob_backend = value.trim().to_ascii_lowercase();
        }
        if !matches!(cfg.blob_backend.as_str(), "disk" | "s3") {
            errors.push(format!(
                "THRUM_BLOB_BACKEND={:?} is not \"disk\" or \"s3\"",
                cfg.blob_backend
            ));
        }
        let text = |name: &str, field: &mut String| {
            if let Some(value) = get(name) {
                *field = value;
            }
        };
        text("THRUM_BLOB_DIR", &mut cfg.blob_dir);
        text("THRUM_S3_ENDPOINT", &mut cfg.s3_endpoint);
        text("THRUM_S3_BUCKET", &mut cfg.s3_bucket);
        text("THRUM_S3_REGION", &mut cfg.s3_region);
        text("THRUM_S3_ACCESS_KEY_ID", &mut cfg.s3_access_key);
        text("THRUM_S3_SECRET_ACCESS_KEY", &mut cfg.s3_secret_key);
        if errors.is_empty() {
            Ok(cfg)
        } else {
            Err(errors.join("; "))
        }
    }
}

impl std::fmt::Debug for Config {
    /// Every setting, with the S3 secret key shown only as set or unset, so
    /// the effective configuration can be printed at startup.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let secret = if self.s3_secret_key.is_empty() {
            "(unset)"
        } else {
            "(set)"
        };
        f.debug_struct("Config")
            .field("register_per_minute", &self.register_per_minute)
            .field("login_per_minute", &self.login_per_minute)
            .field(
                "login_per_account_per_minute",
                &self.login_per_account_per_minute,
            )
            .field(
                "login_failures_before_backoff",
                &self.login_failures_before_backoff,
            )
            .field(
                "login_backoff_base_seconds",
                &self.login_backoff_base_seconds,
            )
            .field("login_backoff_cap_seconds", &self.login_backoff_cap_seconds)
            .field(
                "login_failures_per_account_per_hour",
                &self.login_failures_per_account_per_hour,
            )
            .field("trusted_proxies", &self.trusted_proxies)
            .field(
                "key_package_fetch_per_caller_per_minute",
                &self.key_package_fetch_per_caller_per_minute,
            )
            .field(
                "key_package_fetch_per_caller_device_per_minute",
                &self.key_package_fetch_per_caller_device_per_minute,
            )
            .field(
                "key_package_fetch_per_device_per_minute",
                &self.key_package_fetch_per_device_per_minute,
            )
            .field(
                "key_package_max_per_device",
                &self.key_package_max_per_device,
            )
            .field("max_group_members", &self.max_group_members)
            .field("max_devices_per_account", &self.max_devices_per_account)
            .field(
                "messages_per_device_per_minute",
                &self.messages_per_device_per_minute,
            )
            .field("queued_bytes_per_account", &self.queued_bytes_per_account)
            .field("source_url", &self.source_url)
            .field("message_max_bytes", &self.message_max_bytes)
            .field("queue_ttl_seconds", &self.queue_ttl_seconds)
            .field("attachment_max_bytes", &self.attachment_max_bytes)
            .field("daily_upload_bytes", &self.daily_upload_bytes)
            .field("blob_retention_seconds", &self.blob_retention_seconds)
            .field("blob_backend", &self.blob_backend)
            .field("blob_dir", &self.blob_dir)
            .field("s3_endpoint", &self.s3_endpoint)
            .field("s3_bucket", &self.s3_bucket)
            .field("s3_region", &self.s3_region)
            .field("s3_access_key", &self.s3_access_key)
            .field("s3_secret_key", &secret)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(vars: &[(&str, &str)]) -> Result<Config, String> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()))
    }

    #[test]
    fn valid_settings_apply() {
        let cfg = with(&[
            ("THRUM_ATTACHMENT_MAX_BYTES", "52428800"),
            ("THRUM_BLOB_BACKEND", "S3"),
            ("THRUM_TRUSTED_PROXIES", "10.0.0.0/8, 172.16.0.0/12"),
        ])
        .unwrap();
        assert_eq!(cfg.attachment_max_bytes, 52_428_800);
        assert_eq!(cfg.blob_backend, "s3");
        assert_eq!(cfg.trusted_proxies.len(), 2);
    }

    #[test]
    fn bad_settings_are_refused_by_name() {
        let error = with(&[
            ("THRUM_ATTACHMENT_MAX_BYTES", "50MiB"),
            ("THRUM_BLOB_BACKEND", "s4"),
            ("THRUM_TRUSTED_PROXIES", "10.0.0.0/8;172.16.0.0/12"),
        ])
        .unwrap_err();
        assert!(error.contains("THRUM_ATTACHMENT_MAX_BYTES"), "{error}");
        assert!(error.contains("THRUM_BLOB_BACKEND"), "{error}");
        assert!(error.contains("THRUM_TRUSTED_PROXIES"), "{error}");
    }

    #[test]
    fn the_secret_key_is_not_printed() {
        let cfg = Config {
            s3_secret_key: "super-secret-value".into(),
            ..Config::default()
        };
        let printed = format!("{cfg:?}");
        assert!(!printed.contains("super-secret-value"));
        assert!(printed.contains("s3_secret_key: \"(set)\""));
    }
}
