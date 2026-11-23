//! Fixed-window rate limits, plus a per-account login backoff.
//!
//! Windows are keyed by a caller-chosen string (an IP, a user id, a device
//! id) so the same limiter covers registration, login, and key-package
//! fetches. Login failures also arm an exponential lockout for that account
//! at the failing address, under an hourly ceiling for the account.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::Config;
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
    /// Key-package fetches of one target device by one caller.
    KeyPackageFetchPair,
    /// Key-package fetches of one target device by everyone.
    KeyPackageFetchDevice,
    /// Messages sent by one device.
    Send,
}

#[derive(Debug)]
struct Bucket {
    started: Option<Instant>,
    count: u32,
}

#[derive(Debug)]
struct Failure {
    count: u32,
    locked_until: Option<Instant>,
    last: Instant,
}

/// How long a failure record, or an account's hourly failure count, is
/// remembered once it is no longer locking anything.
const FAILURE_MEMORY: Duration = Duration::from_secs(60 * 60);

/// Above this many entries, a map drops what has expired.
const PRUNE_ABOVE: usize = 10_000;

#[derive(Debug, Default)]
struct Inner {
    buckets: HashMap<(Limit, String), Bucket>,
    /// Keyed by account and client address: a lockout shuts out the
    /// address that guessed, not the owner elsewhere.
    failures: HashMap<String, Failure>,
    /// Failed logins per account in the current hour, from any address.
    account_failures: HashMap<String, Bucket>,
}

/// Counts requests in a fixed window and rejects anything over the limit.
#[derive(Debug, Default, Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<Inner>>,
}

/// The key limits use for a client address: the address itself for IPv4,
/// and its /64 for IPv6, which is what one host usually holds.
pub fn address_key(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let segments = v6.segments();
                format!(
                    "{:x}:{:x}:{:x}:{:x}::/64",
                    segments[0], segments[1], segments[2], segments[3]
                )
            }
        },
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one request of `limit` from `ip`.
    pub fn check(&self, ip: IpAddr, limit: Limit, max: u32) -> Result<(), ApiError> {
        self.check_keyed(limit, &address_key(ip), max)
    }

    /// Count one request of `limit` under `key`.
    pub fn check_keyed(&self, limit: Limit, key: &str, max: u32) -> Result<(), ApiError> {
        let mut inner = self.lock();
        if inner.buckets.len() > PRUNE_ABOVE {
            inner
                .buckets
                .retain(|(limit, _), bucket| bucket_is_live(bucket.started, limit.period()));
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
            .is_none_or(|window_start| now.duration_since(window_start) >= limit.period())
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

    /// Reject a password check for `account` from `ip` when that address is
    /// inside a failure lockout for the account, over its per-minute budget
    /// for the account, or when the account has failed too often this hour
    /// from anywhere. `account` comes from [`account_key`].
    pub fn check_login(&self, account: &str, ip: IpAddr, cfg: &Config) -> Result<(), ApiError> {
        let key = login_key(account, ip);
        {
            let inner = self.lock();
            let locked = inner
                .failures
                .get(&key)
                .and_then(|failure| failure.locked_until)
                .is_some_and(|until| Instant::now() < until);
            let ceiling = cfg.login_failures_per_account_per_hour;
            let over_ceiling = ceiling > 0
                && inner.account_failures.get(account).is_some_and(|bucket| {
                    bucket_is_live(bucket.started, HOUR) && bucket.count >= ceiling
                });
            if locked || over_ceiling {
                return Err(ApiError::too_many_requests());
            }
        }
        self.check_keyed(Limit::LoginAccount, &key, cfg.login_per_account_per_minute)
    }

    /// Record a failed password check. After `login_failures_before_backoff`
    /// failures from one address the account is locked for that address,
    /// and each further failure doubles the lock from the base up to the
    /// cap. Every failure also counts toward the account's hourly ceiling.
    pub fn record_login_failure(&self, account: &str, ip: IpAddr, cfg: &Config) {
        let now = Instant::now();
        let mut inner = self.lock();
        if inner.failures.len() > PRUNE_ABOVE {
            inner.failures.retain(|_, failure| {
                failure.locked_until.is_some_and(|until| now < until)
                    || now.duration_since(failure.last) < FAILURE_MEMORY
            });
        }
        if inner.account_failures.len() > PRUNE_ABOVE {
            inner
                .account_failures
                .retain(|_, bucket| bucket_is_live(bucket.started, HOUR));
        }
        let hourly = inner
            .account_failures
            .entry(account.to_string())
            .or_insert(Bucket {
                started: None,
                count: 0,
            });
        if !bucket_is_live(hourly.started, HOUR) {
            hourly.started = Some(now);
            hourly.count = 0;
        }
        hourly.count = hourly.count.saturating_add(1);

        let after = cfg.login_failures_before_backoff;
        if after == 0 {
            return;
        }
        let base_secs = cfg.login_backoff_base_seconds;
        let cap_secs = cfg.login_backoff_cap_seconds;
        let failure = inner
            .failures
            .entry(login_key(account, ip))
            .or_insert(Failure {
                count: 0,
                locked_until: None,
                last: now,
            });
        failure.count = failure.count.saturating_add(1);
        failure.last = now;
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
            failure.locked_until = Some(now + Duration::from_secs(seconds));
        }
    }

    /// A successful login clears that address's failure counter and lockout.
    pub fn clear_login_failures(&self, account: &str, ip: IpAddr) {
        self.lock().failures.remove(&login_key(account, ip));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(test)]
    fn failure_entries(&self) -> usize {
        self.lock().failures.len()
    }
}

/// The limiter key for a login name: the canonical name when it is valid.
/// Every invalid name shares one key, because no invalid name can log in,
/// and keeping attacker-chosen text as keys would grow memory without
/// bound.
pub fn account_key(username: &str) -> String {
    thrum_core::UserName::new(username)
        .map(|name| name.as_str().to_string())
        .unwrap_or_else(|_| "\u{0}invalid".to_string())
}

fn login_key(account: &str, ip: IpAddr) -> String {
    format!("{account}|{}", address_key(ip))
}

const HOUR: Duration = Duration::from_secs(60 * 60);

impl Limit {
    fn period(self) -> Duration {
        PERIOD
    }
}

fn bucket_is_live(started: Option<Instant>, period: Duration) -> bool {
    started.is_some_and(|window_start| window_start.elapsed() < period)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_names_share_one_key_and_long_names_are_not_kept() {
        assert_eq!(account_key("Alice"), "alice");
        assert_eq!(account_key(&"x".repeat(10_000)), account_key("not valid!"));
        assert!(account_key(&"x".repeat(10_000)).len() < 16);
    }

    #[test]
    fn ipv6_addresses_are_keyed_by_their_64() {
        let a: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:ffff:ffff:ffff:ffff".parse().unwrap();
        let c: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(address_key(a), address_key(b));
        assert_ne!(address_key(a), address_key(c));
        let mapped: IpAddr = "::ffff:192.0.2.1".parse().unwrap();
        assert_eq!(address_key(mapped), "192.0.2.1");
    }

    #[test]
    fn old_failure_records_are_pruned() {
        let limiter = RateLimiter::new();
        let cfg = Config::default();
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        {
            let mut inner = limiter.lock();
            let long_ago = Instant::now() - FAILURE_MEMORY - Duration::from_secs(1);
            for n in 0..=PRUNE_ABOVE {
                inner.failures.insert(
                    format!("user{n}|x"),
                    Failure {
                        count: 1,
                        locked_until: None,
                        last: long_ago,
                    },
                );
            }
        }
        limiter.record_login_failure("alice", ip, &cfg);
        assert_eq!(limiter.failure_entries(), 1);
    }
}
