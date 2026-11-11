//! Basic fixed-window rate limiting per client IP.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::error::ApiError;

/// The window length for every limit.
const PERIOD: Duration = Duration::from_secs(60);

/// Which kind of request is being limited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Register,
    Login,
}

#[derive(Debug, Default)]
struct Windows {
    register_started: Option<Instant>,
    register_count: u32,
    login_started: Option<Instant>,
    login_count: u32,
}

/// Counts requests per IP in a fixed window and rejects over the limit.
#[derive(Debug, Default, Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<IpAddr, Windows>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one request of `limit` from `ip`; error if over `max` for the
    /// current window.
    pub fn check(&self, ip: IpAddr, limit: Limit, max: u32) -> Result<(), ApiError> {
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if map.len() > 10_000 {
            map.retain(|_, windows| windows.has_live_window());
        }
        let entry = map.entry(ip).or_default();
        let now = Instant::now();
        let (started, count) = match limit {
            Limit::Register => (&mut entry.register_started, &mut entry.register_count),
            Limit::Login => (&mut entry.login_started, &mut entry.login_count),
        };
        if started.is_none_or(|window_start| now.duration_since(window_start) >= PERIOD) {
            *started = Some(now);
            *count = 0;
        }
        *count += 1;
        if *count > max {
            Err(ApiError::too_many_requests())
        } else {
            Ok(())
        }
    }
}

impl Windows {
    fn has_live_window(&self) -> bool {
        [self.register_started, self.login_started]
            .iter()
            .any(|started| started.is_some_and(|window_start| window_start.elapsed() < PERIOD))
    }
}
