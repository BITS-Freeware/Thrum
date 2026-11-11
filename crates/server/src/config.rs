//! Server configuration.

/// Runtime configuration. Limits are per client IP per minute.
#[derive(Debug, Clone)]
pub struct Config {
    /// Maximum registration attempts per IP per minute.
    pub register_per_minute: u32,
    /// Maximum login attempts per IP per minute.
    pub login_per_minute: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            register_per_minute: 20,
            login_per_minute: 60,
        }
    }
}
