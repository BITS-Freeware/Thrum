//! Fixed-window rate limits, plus a per-account login backoff.
//!
//! Windows are keyed by a caller-chosen string (an IP, a user id, a device
//! id) so the same limiter covers registration, login, and key-package
//! fetches. Login failures also arm an exponential lockout for that account.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::error::ApiError;

/// The window length for every fixed-window limit.
const PERIOD: Duration = Duration::from_secs(60);

/// Which kind of request is being limited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Limit {
    Register,
    Login,
    /// Login attempts for one account, separate from the per-IP login limit.
    LoginAccount,
    /// Key-package fetches by one authenticated user.
    KeyPackageFetchCaller,
    /// Key-package fetches of one target device.
    KeyPackageFetchDevice,
}

#[derive(Debug)]
struct Bucket {
    started: Option<Instant>,
    count: u32,
}

#[derive(Debug, Default)]
struct Failure {
    count: u32,
    locked_until: Option<Instant>,
}

#[derive(Debug, Default)]
struct Inner {
    buckets: HashMap<(Limit, String), Bucket>,
    failures: HashMap<String, Failure>,
}

/// Counts requests in a fixed window and rejects anything over the limit.
#[derive(Debug, Default, Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<Inner>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one request of `limit` from `ip`.
    pub fn check(&self, ip: IpAddr, limit: Limit, max: u32) -> Result<(), ApiError> {
        self.check_keyed(limit, &ip.to_string(), max)
    }

    /// Count one request of `limit` under `key`.
    pub fn check_keyed(&self, limit: Limit, key: &str, max: u32) -> Result<(), ApiError> {
        let mut inner = self.lock();
        if inner.buckets.len() > 10_000 {
            inner
                .buckets
                .retain(|_, bucket| bucket_is_live(bucket.started));
        }
        let bucket = inner
            .buckets
            .entry((limit, key.to_string()))
            .or_insert(Bucket {
                started: None,
                count: 0,
            });
        let now = Instant::now();
        if bucket
            .started
            .is_none_or(|window_start| now.duration_since(window_start) >= PERIOD)
        {
            bucket.started = Some(now);
            bucket.count = 0;
        }
        bucket.count += 1;
        if bucket.count > max {
            Err(ApiError::too_many_requests())
        } else {
            Ok(())
        }
    }

    /// Reject the login when this account is inside a failure lockout or
    /// over its per-minute attempt budget.
    pub fn check_login_account(&self, username: &str, max_per_minute: u32) -> Result<(), ApiError> {
        if self.login_locked(username) {
            return Err(ApiError::too_many_requests());
        }
        self.check_keyed(Limit::LoginAccount, username, max_per_minute)
    }

    fn login_locked(&self, username: &str) -> bool {
        let inner = self.lock();
        inner
            .failures
            .get(username)
            .and_then(|failure| failure.locked_until)
            .is_some_and(|until| Instant::now() < until)
    }

    /// Record a failed password check. After `after` failures the account
    /// is locked, and each further failure doubles the lock from `base`
    /// seconds up to `cap` seconds.
    pub fn record_login_failure(&self, username: &str, after: u32, base_secs: u64, cap_secs: u64) {
        if after == 0 {
            return;
        }
        let mut inner = self.lock();
        let failure = inner.failures.entry(username.to_string()).or_default();
        failure.count = failure.count.saturating_add(1);
        if failure.count >= after {
            let steps = failure.count - after;
            let mut seconds = base_secs.max(1);
            for _ in 0..steps {
                if seconds >= cap_secs.max(1) {
                    seconds = cap_secs.max(1);
                    break;
                }
                seconds = seconds.saturating_mul(2).min(cap_secs.max(1));
            }
            seconds = seconds.min(cap_secs.max(1));
            failure.locked_until = Some(Instant::now() + Duration::from_secs(seconds));
        }
    }

    /// A successful login clears the failure counter and any lockout.
    pub fn clear_login_failures(&self, username: &str) {
        self.lock().failures.remove(username);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn bucket_is_live(started: Option<Instant>) -> bool {
    started.is_some_and(|window_start| window_start.elapsed() < PERIOD)
}
