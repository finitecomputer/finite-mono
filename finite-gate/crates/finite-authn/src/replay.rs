//! In-memory single-use enforcement for short-lived statements.
//!
//! Vouches carry a `jti` nonce exactly so the consumer can reject replay
//! without durable storage: the guard remembers nonces only as long as the
//! statement could still verify (TTL + skew from the shared
//! [`crate::AuthPolicy`]), then forgets them. A restart empties the table,
//! which is acceptable because a replayed vouch after a restart is also
//! expired (the replay window equals the verification window).

use std::collections::HashMap;
use std::sync::Mutex;

/// Bound on tracked nonces; past it the guard prunes expired entries and,
/// if everything is fresh, denies new keys (an abuse brake, not a meter).
const MAX_TRACKED_NONCES: usize = 100_000;

#[derive(Debug)]
pub struct ReplayGuard {
    replay_window_seconds: u64,
    seen: Mutex<HashMap<String, u64>>,
}

impl ReplayGuard {
    pub fn new(policy: &crate::AuthPolicy) -> ReplayGuard {
        ReplayGuard {
            replay_window_seconds: policy.vouch_replay_window_seconds(),
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Record `nonce` and report whether it is fresh. `false` means the
    /// nonce was already seen (replay) or the table is under abuse pressure.
    pub fn check_and_record(&self, nonce: &str, now_unix: u64) -> bool {
        let mut seen = self.seen.lock().expect("replay guard mutex never poisoned");
        // Entries carry their own expiry (insertion + replay window); forget
        // them once that passes — any statement still carrying the nonce
        // could no longer verify by then either.
        seen.retain(|_, expires_at| *expires_at > now_unix);
        if seen.len() >= MAX_TRACKED_NONCES {
            // Everything tracked is still inside the replay window: deny
            // rather than grow without bound.
            return false;
        }
        seen.insert(nonce.to_string(), now_unix + self.replay_window_seconds)
            .is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuthPolicy;

    const NOW: u64 = 1_750_000_000;

    #[test]
    fn rejects_replay_within_window() {
        let guard = ReplayGuard::new(&AuthPolicy::default());
        assert!(guard.check_and_record("n1", NOW));
        assert!(!guard.check_and_record("n1", NOW + 1));
        assert!(!guard.check_and_record("n1", NOW + 30));
    }

    #[test]
    fn nonces_age_out_after_the_replay_window() {
        let guard = ReplayGuard::new(&AuthPolicy::default());
        assert!(guard.check_and_record("n1", NOW));
        // Window is TTL (60) + skew (60); after it the nonce is forgotten —
        // and any statement carrying it could no longer verify anyway.
        assert!(guard.check_and_record("n1", NOW + 121));
    }

    #[test]
    fn nonces_are_independent() {
        let guard = ReplayGuard::new(&AuthPolicy::default());
        assert!(guard.check_and_record("a", NOW));
        assert!(guard.check_and_record("b", NOW));
        assert!(!guard.check_and_record("a", NOW));
    }
}
