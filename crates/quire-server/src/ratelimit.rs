//! Rate limits: login, setup, per-token requests and bytes, failed bearer authentications (owner: W2).
//!
//! Keyed governor limiters (D47): per token id, `QUIRE_TOKEN_RATE` requests per minute with a burst of 120 and
//! `QUIRE_TOKEN_BYTES_PER_HOUR` of request body counted in KiB; per client IP, 30 failed bearer authentications per
//! minute. Each check answers `Err(retry_after)` when over the limit. Idle keys are dropped by [`Limiters::prune`]
//! (hourly, §12).

use std::net::IpAddr;
use std::num::NonZeroU32;
use std::time::Duration;

use governor::clock::{Clock, DefaultClock};
use governor::{DefaultKeyedRateLimiter, Quota};

use crate::config::{Config, TOKEN_BURST};

/// Failed bearer authentications per IP per minute (D47).
pub const AUTH_FAILURES_PER_MIN: u32 = 30;

fn nonzero(n: u64) -> NonZeroU32 {
    NonZeroU32::new(u32::try_from(n).unwrap_or(u32::MAX).max(1)).expect("at least 1")
}

/// Requests per minute per token.
pub struct TokenLimiter {
    limiter: DefaultKeyedRateLimiter<String>,
    clock: DefaultClock,
}

/// Request body KiB per hour per token.
pub struct TokenBytesLimiter {
    limiter: DefaultKeyedRateLimiter<String>,
    clock: DefaultClock,
    capacity_kib: u32,
}

/// Failed bearer authentications per client IP.
pub struct AuthFailLimiter {
    limiter: DefaultKeyedRateLimiter<IpAddr>,
    clock: DefaultClock,
}

impl TokenLimiter {
    pub fn new(config: &Config) -> TokenLimiter {
        let quota = Quota::per_minute(nonzero(config.token_rate_per_min.into()))
            .allow_burst(nonzero(TOKEN_BURST.into()));
        TokenLimiter {
            limiter: DefaultKeyedRateLimiter::keyed(quota),
            clock: DefaultClock::default(),
        }
    }

    /// Counts one request of `token_id`; `Err(retry_after)` over the limit.
    pub fn check(&self, token_id: &str) -> Result<(), Duration> {
        self.limiter
            .check_key(&token_id.to_owned())
            .map_err(|not_until| not_until.wait_time_from(self.clock.now()))
    }
}

impl TokenBytesLimiter {
    pub fn new(config: &Config) -> TokenBytesLimiter {
        let capacity_kib = nonzero(config.token_bytes_per_hour.div_ceil(1024));
        let quota = Quota::with_period(Duration::from_secs(3600) / capacity_kib.get())
            .unwrap_or_else(|| Quota::per_second(capacity_kib))
            .allow_burst(capacity_kib);
        TokenBytesLimiter {
            limiter: DefaultKeyedRateLimiter::keyed(quota),
            clock: DefaultClock::default(),
            capacity_kib: capacity_kib.get(),
        }
    }

    /// Counts `bytes` of request body of `token_id` (rounded up to KiB); `Err(retry_after)` over the budget.
    pub fn check(&self, token_id: &str, bytes: u64) -> Result<(), Duration> {
        let kib = nonzero(bytes.div_ceil(1024)).min(nonzero(self.capacity_kib.into()));
        match self.limiter.check_key_n(&token_id.to_owned(), kib) {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(not_until)) => Err(not_until.wait_time_from(self.clock.now())),
            Err(_) => Err(Duration::from_secs(3600)),
        }
    }
}

impl AuthFailLimiter {
    pub fn new() -> AuthFailLimiter {
        AuthFailLimiter {
            limiter: DefaultKeyedRateLimiter::keyed(Quota::per_minute(nonzero(
                AUTH_FAILURES_PER_MIN.into(),
            ))),
            clock: DefaultClock::default(),
        }
    }

    /// Counts one failed bearer authentication from `ip`; `Err(retry_after)` once over the limit.
    pub fn record_failure(&self, ip: IpAddr) -> Result<(), Duration> {
        self.limiter
            .check_key(&ip)
            .map_err(|not_until| not_until.wait_time_from(self.clock.now()))
    }
}

impl Default for AuthFailLimiter {
    fn default() -> Self {
        AuthFailLimiter::new()
    }
}

/// Drops idle keys of the three limiters (`retain_recent`).
pub fn prune(token: &TokenLimiter, bytes: &TokenBytesLimiter, auth: &AuthFailLimiter) {
    token.limiter.retain_recent();
    bytes.limiter.retain_recent();
    auth.limiter.retain_recent();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_burst_then_limited() {
        let config = Config::for_tests("/nonexistent");
        let limiter = TokenLimiter::new(&config);
        for _ in 0..TOKEN_BURST {
            limiter.check("t1").unwrap();
        }
        let retry = limiter.check("t1").unwrap_err();
        assert!(retry > Duration::ZERO && retry <= Duration::from_secs(1));
        limiter.check("t2").unwrap();
    }

    #[test]
    fn byte_budget() {
        let mut config = Config::for_tests("/nonexistent");
        config.token_bytes_per_hour = 10 * 1024;
        let limiter = TokenBytesLimiter::new(&config);
        limiter.check("t", 8 * 1024).unwrap();
        limiter.check("t", 2 * 1024).unwrap();
        assert!(limiter.check("t", 1).is_err());
        assert!(
            limiter.check("u", 1 << 40).is_ok(),
            "one oversized body fits the whole budget"
        );
    }

    #[test]
    fn auth_failures_per_ip() {
        let limiter = AuthFailLimiter::new();
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        for _ in 0..AUTH_FAILURES_PER_MIN {
            limiter.record_failure(ip).unwrap();
        }
        assert!(limiter.record_failure(ip).is_err());
        assert!(limiter.record_failure("192.0.2.2".parse().unwrap()).is_ok());
    }
}
