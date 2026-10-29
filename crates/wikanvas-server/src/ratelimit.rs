//! Rate limits: login, setup, per-token requests and bytes, failed bearer authentications (owner: W2).
//!
//! - Login and setup (§8, D19): 5 attempts per minute per client IP (burst 5, governor), and 10 **failed** logins per
//!   15 minutes per email ([`EmailFailures`], a sliding window, because governor cannot test a key without consuming
//!   it and only failures may count). Its keys are the sha256 of the normalized address, so an entry has a fixed
//!   size whatever the request sent; the login route records only well-formed addresses (at most 254 bytes).
//! - Per token (D47): `WIKANVAS_TOKEN_RATE` requests per minute with a burst of 120, and
//!   `WIKANVAS_TOKEN_BYTES_PER_HOUR` of request body counted in KiB.
//! - Per client IP: 30 failed bearer authentications per minute; over that a token that fails gets 429 instead of 401
//!   (valid tokens are still served).
//!
//! Every limiter runs on one [`Clock`]: the real monotonic clock, or a `FakeRelativeClock` in tests (the same clock
//! also gives [`Clock::now_ms`], so token last-use and session times move with it). Each check answers
//! `Err(retry_after)` when over the limit. Idle keys are dropped by [`prune`] (hourly, §12).

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Mutex;
use std::time::Duration;

use governor::clock::FakeRelativeClock;
use governor::middleware::NoOpMiddleware;
use governor::nanos::Nanos;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::{Quota, RateLimiter};
use sha2::{Digest, Sha256};

use crate::config::{Config, TOKEN_BURST};

/// Failed bearer authentications per IP per minute (D47).
pub const AUTH_FAILURES_PER_MIN: u32 = 30;
/// Login and setup attempts per IP per minute (burst 5, §8).
pub const LOGIN_PER_IP_PER_MIN: u32 = 5;
/// Failed logins per email per window (§8).
pub const LOGIN_FAILURES_PER_EMAIL: usize = 10;
/// The per-email window.
pub const LOGIN_FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);

/// The time source of every limiter, and of the server's "now" ([`Clock::now_ms`]).
#[derive(Clone, Debug)]
pub struct Clock {
    start: std::time::Instant,
    /// Unix ms at `start` (real) or at creation (fake).
    base_ms: i64,
    fake: Option<FakeRelativeClock>,
}

impl Clock {
    /// The real clock.
    pub fn real() -> Clock {
        Clock {
            start: std::time::Instant::now(),
            base_ms: crate::state::now_ms(),
            fake: None,
        }
    }

    /// A fake clock that starts at the current wall time and moves only with [`advance`](Self::advance).
    pub fn fake() -> Clock {
        Clock {
            start: std::time::Instant::now(),
            base_ms: crate::state::now_ms(),
            fake: Some(FakeRelativeClock::default()),
        }
    }

    /// Moves a fake clock forward (ignored by the real clock).
    pub fn advance(&self, by: Duration) {
        if let Some(fake) = &self.fake {
            fake.advance(by);
        }
    }

    /// Whether this is a fake clock.
    pub fn is_fake(&self) -> bool {
        self.fake.is_some()
    }

    fn elapsed(&self) -> Nanos {
        match &self.fake {
            Some(fake) => governor::clock::Clock::now(fake),
            None => Nanos::from(self.start.elapsed()),
        }
    }

    /// Unix milliseconds now: the wall clock, or the fake clock's start plus what it was advanced.
    pub fn now_ms(&self) -> i64 {
        match &self.fake {
            Some(_) => {
                let d: Duration = self.elapsed().into();
                self.base_ms + i64::try_from(d.as_millis()).unwrap_or(i64::MAX / 2)
            }
            None => crate::state::now_ms(),
        }
    }
}

impl governor::clock::Clock for Clock {
    type Instant = Nanos;

    fn now(&self) -> Nanos {
        self.elapsed()
    }
}

type Keyed<K> = RateLimiter<K, DefaultKeyedStateStore<K>, Clock, NoOpMiddleware<Nanos>>;

fn nonzero(n: u64) -> NonZeroU32 {
    NonZeroU32::new(u32::try_from(n).unwrap_or(u32::MAX).max(1)).expect("at least 1")
}

fn wait(not_until: &governor::NotUntil<Nanos>, clock: &Clock) -> Duration {
    not_until.wait_time_from(governor::clock::Clock::now(clock))
}

/// A 429's `Retry-After` in whole seconds (at least 1).
pub fn retry_after_secs(d: Duration) -> u64 {
    d.as_secs() + u64::from(d.subsec_nanos() > 0 || d.is_zero())
}

/// Requests per minute per token.
pub struct TokenLimiter {
    limiter: Keyed<String>,
    clock: Clock,
}

/// Request body KiB per hour per token.
pub struct TokenBytesLimiter {
    limiter: Keyed<String>,
    clock: Clock,
    capacity_kib: u32,
}

/// Failed bearer authentications per client IP.
pub struct AuthFailLimiter {
    limiter: Keyed<IpAddr>,
    clock: Clock,
}

impl TokenLimiter {
    pub fn new(config: &Config, clock: Clock) -> TokenLimiter {
        let quota = Quota::per_minute(nonzero(config.token_rate_per_min.into()))
            .allow_burst(nonzero(TOKEN_BURST.into()));
        TokenLimiter {
            limiter: RateLimiter::dashmap_with_clock(quota, clock.clone()),
            clock,
        }
    }

    /// Counts one request of `token_id`; `Err(retry_after)` over the limit.
    pub fn check(&self, token_id: &str) -> Result<(), Duration> {
        self.limiter
            .check_key(&token_id.to_owned())
            .map_err(|n| wait(&n, &self.clock))
    }

    /// The number of tokens it tracks (until [`prune`] drops the idle ones).
    pub fn keys(&self) -> usize {
        self.limiter.len()
    }
}

impl TokenBytesLimiter {
    pub fn new(config: &Config, clock: Clock) -> TokenBytesLimiter {
        let capacity_kib = nonzero(config.token_bytes_per_hour.div_ceil(1024));
        let quota = Quota::with_period(Duration::from_secs(3600) / capacity_kib.get())
            .unwrap_or_else(|| Quota::per_second(capacity_kib))
            .allow_burst(capacity_kib);
        TokenBytesLimiter {
            limiter: RateLimiter::dashmap_with_clock(quota, clock.clone()),
            clock,
            capacity_kib: capacity_kib.get(),
        }
    }

    /// Counts `bytes` of request body of `token_id` (rounded up to KiB; zero bytes count nothing);
    /// `Err(retry_after)` over the budget. A body larger than the whole budget counts as the whole budget.
    pub fn check(&self, token_id: &str, bytes: u64) -> Result<(), Duration> {
        if bytes == 0 {
            return Ok(());
        }
        let kib = nonzero(bytes.div_ceil(1024)).min(nonzero(self.capacity_kib.into()));
        match self.limiter.check_key_n(&token_id.to_owned(), kib) {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(not_until)) => Err(wait(&not_until, &self.clock)),
            Err(_) => Err(Duration::from_secs(3600)),
        }
    }

    /// The number of tokens it tracks (until [`prune`] drops the idle ones).
    pub fn keys(&self) -> usize {
        self.limiter.len()
    }
}

impl AuthFailLimiter {
    pub fn new(clock: Clock) -> AuthFailLimiter {
        AuthFailLimiter {
            limiter: RateLimiter::dashmap_with_clock(
                Quota::per_minute(nonzero(AUTH_FAILURES_PER_MIN.into())),
                clock.clone(),
            ),
            clock,
        }
    }

    /// Counts one failed bearer authentication from `ip`; `Err(retry_after)` once over the limit.
    pub fn record_failure(&self, ip: IpAddr) -> Result<(), Duration> {
        self.limiter
            .check_key(&ip)
            .map_err(|n| wait(&n, &self.clock))
    }

    /// The number of client IPs it tracks (until [`prune`] drops the idle ones).
    pub fn keys(&self) -> usize {
        self.limiter.len()
    }
}

/// The failure window per email: at most `max` failures within `window`. Keyed by the sha256 of the trimmed,
/// lowercased address, so an entry is at most 32 bytes of key and `max` instants.
pub struct EmailFailures {
    clock: Clock,
    max: usize,
    window: Duration,
    failures: Mutex<HashMap<[u8; 32], VecDeque<Nanos>>>,
}

impl EmailFailures {
    pub fn new(clock: Clock, max: usize, window: Duration) -> EmailFailures {
        EmailFailures {
            clock,
            max,
            window,
            failures: Mutex::new(HashMap::new()),
        }
    }

    fn key(email: &str) -> [u8; 32] {
        Sha256::digest(email.trim().to_lowercase().as_bytes()).into()
    }

    /// The number of addresses it tracks (until [`prune`](Self::prune) drops those without a failure in the window).
    pub fn keys(&self) -> usize {
        self.failures
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    /// `Err(retry_after)` while `email` has `max` failures within the window. Counts nothing.
    pub fn check(&self, email: &str) -> Result<(), Duration> {
        let now = governor::clock::Clock::now(&self.clock);
        let window = Nanos::from(self.window);
        let mut map = self.failures.lock().unwrap_or_else(|p| p.into_inner());
        let Some(times) = map.get_mut(&Self::key(email)) else {
            return Ok(());
        };
        while times
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= window)
        {
            times.pop_front();
        }
        if times.len() >= self.max {
            let oldest = *times.front().expect("not empty");
            let waited = now.saturating_sub(oldest);
            return Err(Duration::from(window.saturating_sub(waited)).max(Duration::from_secs(1)));
        }
        Ok(())
    }

    /// Records one failed login for `email`.
    pub fn record_failure(&self, email: &str) {
        let now = governor::clock::Clock::now(&self.clock);
        let mut map = self.failures.lock().unwrap_or_else(|p| p.into_inner());
        let times = map.entry(Self::key(email)).or_default();
        times.push_back(now);
        while times.len() > self.max {
            times.pop_front();
        }
    }

    /// Drops emails whose failures are all outside the window.
    pub fn prune(&self) {
        let now = governor::clock::Clock::now(&self.clock);
        let window = Nanos::from(self.window);
        let mut map = self.failures.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, times| {
            times
                .back()
                .is_some_and(|t| now.saturating_sub(*t) < window)
        });
    }
}

/// The login and setup limiters (§8): attempts per IP, failures per email.
pub struct LoginLimiter {
    per_ip: Keyed<IpAddr>,
    clock: Clock,
    pub per_email: EmailFailures,
}

impl LoginLimiter {
    pub fn new(clock: Clock) -> LoginLimiter {
        let quota = Quota::per_minute(nonzero(LOGIN_PER_IP_PER_MIN.into()))
            .allow_burst(nonzero(LOGIN_PER_IP_PER_MIN.into()));
        LoginLimiter {
            per_ip: RateLimiter::dashmap_with_clock(quota, clock.clone()),
            per_email: EmailFailures::new(
                clock.clone(),
                LOGIN_FAILURES_PER_EMAIL,
                LOGIN_FAILURE_WINDOW,
            ),
            clock,
        }
    }

    /// Counts one login or setup attempt from `ip`; `Err(retry_after)` over the limit.
    pub fn attempt(&self, ip: IpAddr) -> Result<(), Duration> {
        self.per_ip
            .check_key(&ip)
            .map_err(|n| wait(&n, &self.clock))
    }

    /// The number of client IPs the per-IP limit tracks (until [`prune`] drops the idle ones).
    pub fn ip_keys(&self) -> usize {
        self.per_ip.len()
    }
}

/// The limiters of [`crate::state::AppState`].
pub struct Limiters<'a> {
    pub token: &'a TokenLimiter,
    pub bytes: &'a TokenBytesLimiter,
    pub auth: &'a AuthFailLimiter,
    pub login: &'a LoginLimiter,
}

/// Drops idle keys of every limiter (`retain_recent`, §12).
pub fn prune(limiters: Limiters<'_>) {
    limiters.token.limiter.retain_recent();
    limiters.bytes.limiter.retain_recent();
    limiters.auth.limiter.retain_recent();
    limiters.login.per_ip.retain_recent();
    limiters.login.per_email.prune();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_burst_then_limited_and_refilled() {
        let config = Config::for_tests("/nonexistent");
        let clock = Clock::fake();
        let limiter = TokenLimiter::new(&config, clock.clone());
        for _ in 0..TOKEN_BURST {
            limiter.check("t1").unwrap();
        }
        let retry = limiter.check("t1").unwrap_err();
        assert!(
            retry > Duration::ZERO && retry <= Duration::from_millis(100),
            "{retry:?}"
        );
        limiter.check("t2").unwrap();
        clock.advance(Duration::from_millis(100));
        limiter.check("t1").unwrap();
    }

    #[test]
    fn byte_budget() {
        let mut config = Config::for_tests("/nonexistent");
        config.token_bytes_per_hour = 10 * 1024;
        let clock = Clock::fake();
        let limiter = TokenBytesLimiter::new(&config, clock.clone());
        limiter.check("t", 8 * 1024).unwrap();
        limiter.check("t", 2 * 1024).unwrap();
        limiter.check("t", 0).unwrap();
        let retry = limiter.check("t", 1).unwrap_err();
        assert!(retry <= Duration::from_secs(360), "{retry:?}");
        assert!(
            limiter.check("u", 1 << 40).is_ok(),
            "one oversized body fits the whole budget"
        );
        clock.advance(Duration::from_secs(360));
        limiter.check("t", 1).unwrap();
    }

    #[test]
    fn auth_failures_per_ip() {
        let limiter = AuthFailLimiter::new(Clock::fake());
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        for _ in 0..AUTH_FAILURES_PER_MIN {
            limiter.record_failure(ip).unwrap();
        }
        assert!(limiter.record_failure(ip).is_err());
        assert!(limiter.record_failure("192.0.2.2".parse().unwrap()).is_ok());
    }

    #[test]
    fn email_failures_slide() {
        let clock = Clock::fake();
        let f = EmailFailures::new(clock.clone(), 3, Duration::from_secs(60));
        for _ in 0..3 {
            f.check("A@x").unwrap();
            f.record_failure("a@x ");
            clock.advance(Duration::from_secs(10));
        }
        let retry = f.check("a@X").unwrap_err();
        assert_eq!(retry, Duration::from_secs(30));
        clock.advance(Duration::from_secs(30));
        f.check("a@x").unwrap();
        f.prune();
        clock.advance(Duration::from_secs(60));
        f.prune();
        assert!(f.failures.lock().unwrap().is_empty());
    }

    #[test]
    fn fake_clock_moves_now_ms() {
        let clock = Clock::fake();
        let t0 = clock.now_ms();
        clock.advance(Duration::from_secs(61));
        assert_eq!(clock.now_ms() - t0, 61_000);
        assert_eq!(retry_after_secs(Duration::from_millis(1500)), 2);
        assert_eq!(retry_after_secs(Duration::from_secs(3)), 3);
        assert_eq!(retry_after_secs(Duration::ZERO), 1);
    }
}
