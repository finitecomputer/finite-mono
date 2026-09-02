//! Gate daemon state: the WorkOS client (in production), the pending
//! AuthKit round trips, the public-route limiter, and the config.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::config::GateConfig;
use crate::limiter::PublicRouteLimiter;

/// Cap on tracked pending AuthKit states; past it the table is swept, and
/// if everything is fresh the flow denies rather than grows without bound.
const MAX_PENDING_STATES: usize = 10_000;

pub struct GateState {
    pub config: GateConfig,
    pub workos: Option<workos::Client>,
    pub limiter: PublicRouteLimiter,
    pending: Mutex<HashMap<String, PendingRequest>>,
}

#[derive(Debug, Clone)]
struct PendingRequest {
    audience: String,
    return_to: String,
    expires_at: u64,
}

impl GateState {
    pub fn new(config: GateConfig) -> GateState {
        let workos = config.workos_client_id.as_ref().map(|client_id| {
            workos::Client::builder()
                .client_id(client_id.clone())
                .api_key(
                    config
                        .workos_api_key
                        .clone()
                        .expect("validated alongside the client id"),
                )
                .build()
        });
        GateState {
            config,
            workos,
            limiter: PublicRouteLimiter::default(),
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Store one pending AuthKit round trip under a fresh random state
    /// nonce and return the nonce for the authorization URL.
    pub fn remember_pending(&self, audience: &str, return_to: &str, now: u64) -> String {
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce).expect("operating system randomness must be available");
        let state = finite_authn::hex::encode(&nonce);
        let mut pending = self.pending.lock().expect("pending mutex never poisoned");
        pending.retain(|_, request| request.expires_at > now);
        if pending.len() < MAX_PENDING_STATES {
            pending.insert(
                state.clone(),
                PendingRequest {
                    audience: audience.to_string(),
                    return_to: return_to.to_string(),
                    expires_at: now + crate::PENDING_STATE_TTL_SECONDS,
                },
            );
        }
        state
    }

    /// Consume the state nonce (single use). `None` when unknown, expired,
    /// or already consumed.
    pub fn take_pending(&self, state_nonce: &str, now: u64) -> Option<(String, String)> {
        if state_nonce.len() != 64 || !finite_authn::hex::is_hex32(state_nonce) {
            return None;
        }
        let mut pending = self.pending.lock().expect("pending mutex never poisoned");
        let request = pending.remove(state_nonce)?;
        if request.expires_at <= now {
            return None;
        }
        Some((request.audience, request.return_to))
    }
}

pub type SharedGateState = Arc<GateState>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_DEV_EMAIL;

    fn state() -> GateState {
        GateState::new(GateConfig {
            listen: "127.0.0.1:8792".parse().unwrap(),
            public_url: "http://127.0.0.1:8792".to_string(),
            signing_key: [7; 32],
            workos_client_id: None,
            workos_api_key: None,
            dev_mode: true,
            dev_email: DEFAULT_DEV_EMAIL.to_string(),
        })
    }

    #[test]
    fn pending_states_are_single_use_and_expire() {
        let gate = state();
        let nonce = gate.remember_pending("https://hello.finite.chat", "/", 1_000);
        assert_eq!(
            gate.take_pending(&nonce, 1_000),
            Some(("https://hello.finite.chat".to_string(), "/".to_string()))
        );
        // Consumed.
        assert_eq!(gate.take_pending(&nonce, 1_000), None);
        // Unknown/malformed states never resolve.
        assert_eq!(gate.take_pending("deadbeef", 1_000), None);

        let nonce = gate.remember_pending("https://hello.finite.chat", "/", 1_000);
        assert_eq!(gate.take_pending(&nonce, 1_000 + 601), None);
    }
}
