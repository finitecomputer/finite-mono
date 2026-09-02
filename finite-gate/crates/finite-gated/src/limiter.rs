//! Hand-rolled fixed-window per-IP rate limiter for the gate's public
//! routes (`/authorize`, `/callback`, `/dev/confirm`), following the
//! finitechat-server public-route limiter pattern: a
//! `Mutex<HashMap<IpAddr, (window_start, count)>>` is enough at the current
//! fleet size and adds no dependency.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Requests per IP per window. The gate's public surface is tiny; a browser
/// needs a handful of authorizes even with several tabs.
pub const RATE_LIMIT_PER_WINDOW: u32 = 120;
pub const RATE_LIMIT_WINDOW_SECONDS: u64 = 60;

/// Cap on tracked client buckets; past it, each check sweeps expired
/// windows so a spray of spoofed/one-shot IPs cannot grow the map without
/// bound.
const MAX_RATE_LIMIT_ENTRIES: usize = 10_000;

#[derive(Debug)]
pub struct PublicRouteLimiter {
    max_requests: u32,
    window: Duration,
    max_entries: usize,
    windows: Mutex<HashMap<IpAddr, (Instant, u32)>>,
}

impl PublicRouteLimiter {
    pub fn new(max_requests: u32, window_seconds: u64) -> Self {
        Self {
            max_requests,
            window: Duration::from_secs(window_seconds),
            max_entries: MAX_RATE_LIMIT_ENTRIES,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Record one request from `ip`; false once the window allowance is spent.
    pub fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if windows.len() >= self.max_entries {
            windows.retain(|_, (started, _)| now.duration_since(*started) < self.window);
        }
        match windows.get_mut(&ip) {
            Some((started, count)) if now.duration_since(*started) < self.window => {
                if *count >= self.max_requests {
                    return false;
                }
                *count += 1;
                true
            }
            _ => {
                windows.insert(ip, (now, 1));
                true
            }
        }
    }
}

impl Default for PublicRouteLimiter {
    fn default() -> Self {
        Self::new(RATE_LIMIT_PER_WINDOW, RATE_LIMIT_WINDOW_SECONDS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_limit_then_denies_until_the_window_passes() {
        let limiter = PublicRouteLimiter::new(2, 60);
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
        assert!(!limiter.check(ip));
        // Simulate window expiry by draining the tracked windows.
        limiter
            .windows
            .lock()
            .unwrap()
            .insert(ip, (Instant::now() - Duration::from_secs(61), 2));
        assert!(limiter.check(ip));
    }

    #[test]
    fn independent_ips_do_not_share_budgets() {
        let limiter = PublicRouteLimiter::new(1, 60);
        let a: IpAddr = "127.0.0.1".parse().unwrap();
        let b: IpAddr = "127.0.0.2".parse().unwrap();
        assert!(limiter.check(a));
        assert!(!limiter.check(a));
        assert!(limiter.check(b));
    }
}
