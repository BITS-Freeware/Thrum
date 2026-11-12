//! Server configuration.

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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            register_per_minute: 20,
            login_per_minute: 60,
            message_max_bytes: 256 * 1024,
            queue_ttl_seconds: 7 * 24 * 60 * 60,
        }
    }
}
