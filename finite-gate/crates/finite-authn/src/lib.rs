//! `finite-authn` — the shared, pure authentication-statement verifier.
//!
//! Finite has exactly two auth sentences, one per participant kind:
//! **actors sign** and **viewers gate**. This crate speaks both, with one
//! shared policy table:
//!
//! - NIP-98 ("this key makes this HTTP request"): a kind-27235 nostr event
//!   bound to one URL + method (+ body hash) inside a small freshness
//!   window. Used by every control-plane actor (CLI, agents, daemons).
//! - Gate vouch ("the gate authenticated this human for this origin just
//!   now"): a short-lived, origin-bound, single-use signed statement minted
//!   by the Auth Gate after WorkOS verifies a human. finitesitesd verifies
//!   it OFFLINE against a pinned gate public key — it never calls the gate
//!   at runtime.
//!
//! The crate is deliberately small and pure: no storage, no HTTP server, no
//! clock reads, no randomness. Callers supply `now` and nonces so the policy
//! stays testable and embeddable.

pub mod event;
pub mod hex;
pub mod nip98;
pub mod replay;
pub mod vouch;

pub use event::NostrEvent;
pub use replay::ReplayGuard;
pub use vouch::{VouchClaims, gate_pubkey_for_secret, mint_vouch, verify_vouch};

/// The one shared policy table for both statement kinds. Values here are the
/// production contract; they may be tightened per deployment, never loosened
/// without a compatibility review of every verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthPolicy {
    /// NIP-98 events older or newer than this many seconds are rejected.
    /// 60s is the spec-suggested window.
    pub nip98_max_skew_seconds: u64,
    /// A vouch is consumed by one browser redirect, not emailed: ~60s from
    /// mint to redeem is ample and keeps the replay surface tiny.
    pub vouch_ttl_seconds: u64,
    /// Verifier-side clock slack for a vouch's `iat`/`exp` in addition to
    /// the TTL (mint and verify hosts may drift slightly).
    pub vouch_max_skew_seconds: u64,
}

pub const DEFAULT_NIP98_MAX_SKEW_SECONDS: u64 = 60;
pub const DEFAULT_VOUCH_TTL_SECONDS: u64 = 60;
pub const DEFAULT_VOUCH_MAX_SKEW_SECONDS: u64 = 60;

impl Default for AuthPolicy {
    fn default() -> Self {
        AuthPolicy {
            nip98_max_skew_seconds: DEFAULT_NIP98_MAX_SKEW_SECONDS,
            vouch_ttl_seconds: DEFAULT_VOUCH_TTL_SECONDS,
            vouch_max_skew_seconds: DEFAULT_VOUCH_MAX_SKEW_SECONDS,
        }
    }
}

impl AuthPolicy {
    /// The longest a vouch can remain acceptable to a verifier: TTL plus
    /// skew. Replay guards only need to remember nonces this long.
    pub fn vouch_replay_window_seconds(&self) -> u64 {
        self.vouch_ttl_seconds + self.vouch_max_skew_seconds
    }
}

/// Errors for the statement kinds this crate verifies.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthnError {
    #[error("invalid hex: {0}")]
    InvalidHex(&'static str),
    #[error("invalid event: {0}")]
    InvalidEvent(&'static str),
    #[error("invalid signature")]
    InvalidSignature,
    #[error("invalid auth header: {0}")]
    InvalidAuthHeader(&'static str),
    #[error("auth rejected: {0}")]
    AuthRejected(&'static str),
    #[error("invalid vouch: {0}")]
    InvalidVouch(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_window_covers_ttl_plus_skew() {
        let policy = AuthPolicy::default();
        assert_eq!(policy.vouch_ttl_seconds, 60);
        assert_eq!(policy.vouch_replay_window_seconds(), 120);
    }
}
