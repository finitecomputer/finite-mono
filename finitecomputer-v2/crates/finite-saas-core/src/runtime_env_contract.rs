//! Runtime environment key contract shared by finite-saas-core and
//! finite-saas-runner.
//!
//! Core validates the operator-configured runtime environment when it builds a
//! `RuntimeSpecV1`; the runner validates the same map again at launch. The two
//! validations must agree: when the runner reserved a key that Core did not,
//! an operator setting that key passed Core's validation and then bricked
//! every launch with "owned by the Runtime contract". This crate is the single
//! source both crates consume, so the two sides cannot drift apart again.
//!
//! Keep this crate dependency-free: it exists to peel the runtime env key
//! contract out of the runner -> finite-saas-core coupling, not to deepen it.

/// Maximum length of a runtime environment key, in bytes.
pub const MAX_RUNTIME_ENVIRONMENT_KEY_BYTES: usize = 128;

/// Retired AEON specialization worker keys.
///
/// Retired AEON worker keys stay reserved so leftover host or operator env
/// cannot re-inject the clawland specialization bundle. Reserved-alone is
/// not enough on Kata upgrade: that path copies reserved keys forward, so
/// upgrade and replacement also drop them from retained env (which is why
/// this subset is named separately from the full reserved list).
pub const RETIRED_SPECIALIZATION_ENVIRONMENT_KEYS: &[&str] = &[
    "FBRAIN_EMBEDDING_BEARER_TOKEN",
    "FBRAIN_EMBEDDING_ENDPOINT",
    "FINITE_SPECIALIZATION_BUNDLE",
    "FINITE_SPECIALIZATION_WORKER_API_KEY",
];

/// Every environment key owned by the Runtime contract, byte-wise sorted
/// (`str` Ord, so `FINITECHAT_*` precedes `FINITE_*`; kept sorted so the
/// nesting test below can hold the list to one canonical order).
///
/// This is the union of the two lists Core and the runner kept separately
/// before the single source (the runner's list was the safety-superset), so
/// both sides now reject exactly the same keys. Core rejecting a key here is
/// a validation-time error; the runner rejecting a key Core accepted is a
/// bricked launch.
pub const RESERVED_RUNTIME_ENVIRONMENT_KEYS: &[&str] = &[
    "FBRAIN_EMBEDDING_BEARER_TOKEN",
    "FBRAIN_EMBEDDING_ENDPOINT",
    "FINITECHAT_ALLOW_ALL_USERS",
    "FINITECHAT_HERMES_AGENT_DEVICE_ID",
    "FINITECHAT_HERMES_AGENT_NAME",
    "FINITECHAT_HERMES_AGENT_PICTURE_URL",
    "FINITECHAT_HERMES_API_MODE",
    "FINITECHAT_HERMES_BASE_URL",
    "FINITECHAT_HERMES_CONTEXT_LENGTH",
    "FINITECHAT_HERMES_INBOUND_STREAM",
    "FINITECHAT_HERMES_MODEL",
    "FINITECHAT_HERMES_PROVIDER",
    "FINITECHAT_HERMES_ROOM_NAME",
    "FINITECHAT_HOME",
    "FINITECHAT_SERVER_URL",
    "FINITECHAT_WORKSPACE",
    "FINITE_AGENT_BOOT_INTENT_JSON",
    "FINITE_AGENT_HTTP_HOST",
    "FINITE_AGENT_HTTP_PORT",
    "FINITE_AGENT_ID",
    "FINITE_AGENT_NAME",
    "FINITE_AGENT_STATE_ROOT",
    "FINITE_ALLOW_ALL_USERS",
    "FINITE_DEFAULT_INFERENCE_PROFILE",
    "FINITE_HOME",
    "FINITE_PRIVATE_API_KEY",
    "FINITE_PRIVATE_BASE_URL",
    "FINITE_PRIVATE_CONTEXT_LENGTH",
    "FINITE_PRIVATE_MODEL",
    "FINITE_SERVER_URL",
    "FINITE_SPECIALIZATION_BUNDLE",
    "FINITE_SPECIALIZATION_WORKER_API_KEY",
    "GATEWAY_ALLOW_ALL_USERS",
    "HERMES_HOME",
    "OPENAI_API_KEY",
];

/// Whether `key` is owned by the Runtime contract and may not appear in
/// operator-configured runtime environment or secret references.
pub fn reserved_runtime_environment_key(key: &str) -> bool {
    RESERVED_RUNTIME_ENVIRONMENT_KEYS.contains(&key)
}

/// Whether `key` is one of the retired AEON specialization keys that upgrade
/// and replacement paths must also drop from retained container env.
pub fn retired_specialization_environment_key(key: &str) -> bool {
    RETIRED_SPECIALIZATION_ENVIRONMENT_KEYS.contains(&key)
}

/// Whether `key` looks secret-bearing and must be configured as a secret
/// reference, not as inline runtime environment.
pub fn secret_runtime_environment_key(key: &str) -> bool {
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"]
        .iter()
        .any(|part| key.split('_').any(|segment| segment == *part))
}

/// Whether `key` is a bounded uppercase environment name: non-empty, at most
/// [`MAX_RUNTIME_ENVIRONMENT_KEY_BYTES`] bytes, ASCII uppercase, underscore,
/// and digits anywhere but the first byte.
pub fn valid_runtime_environment_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_RUNTIME_ENVIRONMENT_KEY_BYTES
        && key.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_uppercase() || (index > 0 && byte.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lists must stay sorted and deduplicated, and the retired subset
    /// must remain part of the reserved union, so the named subset can never
    /// drift away from the contract it belongs to.
    #[test]
    fn reserved_lists_are_sorted_deduplicated_and_nested() {
        for list in [
            RESERVED_RUNTIME_ENVIRONMENT_KEYS,
            RETIRED_SPECIALIZATION_ENVIRONMENT_KEYS,
        ] {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(list, sorted.as_slice(), "reserved lists must be sorted");
        }
        for key in RETIRED_SPECIALIZATION_ENVIRONMENT_KEYS {
            assert!(
                reserved_runtime_environment_key(key),
                "retired key {key} fell out of the reserved list"
            );
        }
        assert_eq!(RESERVED_RUNTIME_ENVIRONMENT_KEYS.len(), 35);
        assert_eq!(RETIRED_SPECIALIZATION_ENVIRONMENT_KEYS.len(), 4);
    }

    /// These six keys were runner-reserved but NOT core-reserved before the
    /// single source: Core's validation accepted them and every runner launch
    /// then failed with "owned by the Runtime contract". They must stay
    /// reserved, and the ones that do not match the secret shape would sail
    /// through any validation that only rejected secret-looking keys.
    #[test]
    fn historically_drifted_keys_stay_reserved() {
        let drifted_plain_shaped = [
            "FBRAIN_EMBEDDING_ENDPOINT",
            "FINITE_PRIVATE_CONTEXT_LENGTH",
            "FINITE_SPECIALIZATION_BUNDLE",
            "FINITECHAT_HERMES_CONTEXT_LENGTH",
        ];
        for key in drifted_plain_shaped {
            assert!(reserved_runtime_environment_key(key));
            assert!(!secret_runtime_environment_key(key));
        }
        assert!(reserved_runtime_environment_key(
            "FBRAIN_EMBEDDING_BEARER_TOKEN"
        ));
        assert!(reserved_runtime_environment_key(
            "FINITE_SPECIALIZATION_WORKER_API_KEY"
        ));
    }

    /// The heuristic matches whole underscore-separated segments, so
    /// "MONKEY" is a key about monkeys, not a secret about keys.
    #[test]
    fn secret_shape_requires_a_whole_segment_match() {
        assert!(secret_runtime_environment_key("FINITE_AGENT_LAUNCH_TOKEN"));
        assert!(secret_runtime_environment_key("MY_SERVICE_PASSWORD"));
        assert!(secret_runtime_environment_key("OPENAI_API_KEY"));
        assert!(!secret_runtime_environment_key("MONKEY"));
        assert!(!secret_runtime_environment_key("FINITE_SERVER_URL"));
        assert!(!secret_runtime_environment_key(
            "FINITE_SPECIALIZATION_BUNDLE"
        ));
    }

    #[test]
    fn environment_key_shape_is_bounded_uppercase() {
        assert!(valid_runtime_environment_key("FINITE_AGENT_2_URL"));
        assert!(!valid_runtime_environment_key(""));
        assert!(!valid_runtime_environment_key("2FINITE"));
        assert!(!valid_runtime_environment_key("finite"));
        assert!(!valid_runtime_environment_key("FINITE AGENT"));
        assert!(!valid_runtime_environment_key(
            &"F".repeat(MAX_RUNTIME_ENVIRONMENT_KEY_BYTES + 1)
        ));
    }
}
