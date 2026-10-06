//! Personal Agent Consent: the Agent's half of Personal Brain setup.
//!
//! A Personal Brain needs two signatures. The owner signs the creation
//! request, and the Agent signs this consent naming that owner. The Brain
//! server checks both against its own records, so one person cannot claim
//! another person's Agent: `personal_agents.agent_npub` is unique.

use aes_gcm::aead::OsRng;
use aes_gcm::aead::rand_core::RngCore;
use finite_nostr::{NostrPublicKey, verify_event_integrity};
use nostr::Event;
use serde::{Deserialize, Serialize};

use crate::{
    APP_SPECIFIC_KIND, BrainEventTemplate, BrainId, CryptoRecordError, absolute_http_url_parts,
    brain_identity_provider_error, hex_encode, json_string,
};

/// Versioned consent an Agent signs to become one owner's Personal Agent.
pub const PERSONAL_AGENT_CONSENT_VERSION: &str = "finite-brain-personal-agent-consent-v1";
/// Longest a consent may stay valid after the Agent signs it.
pub const MAX_PERSONAL_AGENT_CONSENT_SECONDS: u64 = 15 * 60;

/// Signed payload bound into a `finite-brain-personal-agent-consent-v1`
/// Nostr event. The Agent's key signs it; the Brain server only validates it.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAgentConsentPayload {
    pub version: String,
    pub agent_npub: String,
    pub owner_npub: String,
    pub brain_id: String,
    pub brain_server: String,
    pub nonce: String,
    pub expires_at: u64,
}

impl PersonalAgentConsentPayload {
    /// Canonical JSON carried as the signed event content.
    pub fn canonical_json(&self) -> String {
        format!(
            "{{\"version\":{},\"agentNpub\":{},\"ownerNpub\":{},\"brainId\":{},\"brainServer\":{},\"nonce\":{},\"expiresAt\":{}}}",
            json_string(&self.version),
            json_string(&self.agent_npub),
            json_string(&self.owner_npub),
            json_string(&self.brain_id),
            json_string(&self.brain_server),
            json_string(&self.nonce),
            self.expires_at,
        )
    }
}

/// A fresh random consent nonce: 32 lowercase hex characters.
pub fn personal_agent_consent_nonce() -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex_encode(&bytes)
}

/// The one Personal Brain ID for an owner: `personal-` and the first 16 hex
/// characters of the owner's public key. Retries of a lost setup response
/// therefore reach the same Brain.
pub fn personal_brain_id_for_owner(owner_npub: &str) -> Result<BrainId, CryptoRecordError> {
    let owner = canonical_npub(owner_npub, "owner")?;
    let hex = NostrPublicKey::parse(&owner)
        .map_err(|error| CryptoRecordError::EventMismatch {
            reason: format!("Personal Agent Consent owner npub is invalid: {error}"),
        })?
        .to_hex();
    Ok(BrainId::new(format!("personal-{}", &hex[..16]))?)
}

/// Whether `brain_id` has the Personal Brain shape (`personal-` and 16 lowercase
/// hex characters). Only its owner's Personal Brain may use such an ID, so no
/// Organization Brain can take the ID another person's setup needs.
pub fn is_reserved_personal_brain_id(brain_id: &str) -> bool {
    brain_id.strip_prefix("personal-").is_some_and(|suffix| {
        suffix.len() == 16
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// Validate the typed payload of one Personal Agent Consent.
pub fn validate_personal_agent_consent_payload(
    payload: &PersonalAgentConsentPayload,
) -> Result<(), CryptoRecordError> {
    if payload.version != PERSONAL_AGENT_CONSENT_VERSION {
        return brain_identity_provider_error("Personal Agent Consent version is unsupported");
    }
    canonical_npub(&payload.agent_npub, "agent")?;
    canonical_npub(&payload.owner_npub, "owner")?;
    if payload.agent_npub == payload.owner_npub {
        return brain_identity_provider_error(
            "Personal Agent Consent must name an owner other than the Agent",
        );
    }
    if personal_brain_id_for_owner(&payload.owner_npub)?.as_str() != payload.brain_id {
        return brain_identity_provider_error(
            "Personal Agent Consent names a Brain other than the owner's Personal Brain",
        );
    }
    match absolute_http_url_parts(&payload.brain_server) {
        Some((origin, "/")) if origin == payload.brain_server => {}
        _ => {
            return brain_identity_provider_error(
                "Personal Agent Consent Brain server must be an http(s) origin",
            );
        }
    }
    if payload.nonce.len() != 32
        || !payload
            .nonce
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return brain_identity_provider_error("Personal Agent Consent nonce is invalid");
    }
    if payload.expires_at == 0 {
        return brain_identity_provider_error("Personal Agent Consent expiry is invalid");
    }
    Ok(())
}

fn personal_agent_consent_tags(payload: &PersonalAgentConsentPayload) -> Vec<Vec<String>> {
    vec![
        vec![
            "d".to_owned(),
            format!(
                "finite-brain-personal-agent-consent:{}:{}",
                payload.brain_id, payload.nonce
            ),
        ],
        vec!["brain".to_owned(), payload.brain_id.clone()],
        vec!["owner".to_owned(), payload.owner_npub.clone()],
        vec!["nonce".to_owned(), payload.nonce.clone()],
    ]
}

fn validate_consent_window(created_at: u64, expires_at: u64) -> Result<(), CryptoRecordError> {
    if expires_at <= created_at || expires_at - created_at > MAX_PERSONAL_AGENT_CONSENT_SECONDS {
        return brain_identity_provider_error(
            "Personal Agent Consent must expire within 15 minutes of signing",
        );
    }
    Ok(())
}

/// Build the unsigned event template the Agent signs for one consent.
pub fn personal_agent_consent_event_template(
    payload: &PersonalAgentConsentPayload,
    created_at: u64,
) -> Result<BrainEventTemplate, CryptoRecordError> {
    validate_personal_agent_consent_payload(payload)?;
    validate_consent_window(created_at, payload.expires_at)?;
    Ok(BrainEventTemplate {
        kind: APP_SPECIFIC_KIND,
        created_at,
        tags: personal_agent_consent_tags(payload),
        content: payload.canonical_json(),
    })
}

/// Verify a signed Personal Agent Consent and return its bound payload.
/// Checks the signature, kind, canonical content, tags, signing window, and
/// that the Agent named in the payload is the signer. The caller still checks
/// expiry against its own clock and the bindings to its request.
pub fn verify_personal_agent_consent_event(
    event: &Event,
) -> Result<PersonalAgentConsentPayload, CryptoRecordError> {
    verify_event_integrity(event).map_err(|error| CryptoRecordError::EventMismatch {
        reason: format!("Personal Agent Consent signature is invalid: {error}"),
    })?;
    if event.kind.as_u16() != APP_SPECIFIC_KIND || event.content.is_empty() {
        return brain_identity_provider_error(
            "Personal Agent Consent event kind or content is invalid",
        );
    }
    let payload: PersonalAgentConsentPayload =
        serde_json::from_str(&event.content).map_err(|_| CryptoRecordError::EventMismatch {
            reason: "Personal Agent Consent payload did not parse".to_owned(),
        })?;
    validate_personal_agent_consent_payload(&payload)?;
    let signer = NostrPublicKey::from_protocol(event.pubkey)
        .to_npub()
        .map_err(|error| CryptoRecordError::EventMismatch {
            reason: format!("Personal Agent Consent signer is invalid: {error}"),
        })?;
    if signer != payload.agent_npub {
        return brain_identity_provider_error(
            "Personal Agent Consent signer does not match its Agent",
        );
    }
    if payload.canonical_json() != event.content {
        return brain_identity_provider_error("Personal Agent Consent payload is not canonical");
    }
    let actual_tags = event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect::<Vec<_>>();
    if actual_tags != personal_agent_consent_tags(&payload) {
        return brain_identity_provider_error(
            "Personal Agent Consent tags differ from its payload",
        );
    }
    validate_consent_window(event.created_at.as_secs(), payload.expires_at)?;
    Ok(payload)
}

fn canonical_npub(value: &str, role: &str) -> Result<String, CryptoRecordError> {
    let invalid = |detail: String| CryptoRecordError::EventMismatch {
        reason: format!("Personal Agent Consent {role} npub is invalid: {detail}"),
    };
    let canonical = NostrPublicKey::parse(value)
        .map_err(|error| invalid(error.to_string()))?
        .to_npub()
        .map_err(|error| invalid(error.to_string()))?;
    if canonical != value {
        return Err(invalid("not a canonical npub".to_owned()));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use nostr::{Keys, SecretKey};

    use super::*;
    use crate::sign_brain_event_template;

    const SERVER: &str = "https://brain.example";

    fn npub(keys: &Keys) -> String {
        NostrPublicKey::from_protocol(keys.public_key())
            .to_npub()
            .unwrap()
    }

    fn payload(agent: &Keys, owner: &Keys) -> PersonalAgentConsentPayload {
        let owner_npub = npub(owner);
        PersonalAgentConsentPayload {
            version: PERSONAL_AGENT_CONSENT_VERSION.to_owned(),
            agent_npub: npub(agent),
            brain_id: personal_brain_id_for_owner(&owner_npub)
                .unwrap()
                .as_str()
                .to_owned(),
            owner_npub,
            brain_server: SERVER.to_owned(),
            nonce: "ab".repeat(16),
            expires_at: 1_780_000_600,
        }
    }

    fn sign(keys: &Keys, payload: &PersonalAgentConsentPayload) -> Event {
        let template = personal_agent_consent_event_template(payload, 1_780_000_000).unwrap();
        sign_brain_event_template(keys, &template).unwrap()
    }

    /// FROZEN WIRE CONTRACT: `finite-brain-personal-agent-consent-v1`. These
    /// literals pin the canonical JSON, tag profile, kind and event-id
    /// derivation for a fixed key, payload and created_at. A deliberate
    /// protocol change gets a new version instead of reordered fields. The
    /// test key is a committed fixture, never a secret.
    #[test]
    fn personal_agent_consent_wire_format_is_frozen() {
        const AGENT_SECRET_HEX: &str =
            "d1f1a2b3c4d5e6f708192a3b4c5d6e7f8192a3b4c5d6e7f8192a3b4c5d6e7f81";
        const OWNER_SECRET_HEX: &str =
            "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";
        const AGENT_NPUB: &str = "npub12ht6ulk5xgnw7v58g8vpfvnclnvfy39rcvkshgy6kk733675q85qdxlf8h";
        const OWNER_NPUB: &str = "npub1tge6gyl4ys4mfh5s4lcqdy69w7fuxeq36dtrk82q93m8qpscw3ksm0lym6";
        const BRAIN_ID: &str = "personal-5a33a413f5242bb4";
        const NONCE: &str = "0123456789abcdef0123456789abcdef";
        const CANONICAL_JSON: &str = "{\"version\":\"finite-brain-personal-agent-consent-v1\",\"agentNpub\":\"npub12ht6ulk5xgnw7v58g8vpfvnclnvfy39rcvkshgy6kk733675q85qdxlf8h\",\"ownerNpub\":\"npub1tge6gyl4ys4mfh5s4lcqdy69w7fuxeq36dtrk82q93m8qpscw3ksm0lym6\",\"brainId\":\"personal-5a33a413f5242bb4\",\"brainServer\":\"https://brain.example\",\"nonce\":\"0123456789abcdef0123456789abcdef\",\"expiresAt\":1780000600}";
        // BIP-340 signatures use auxiliary randomness, so only the event id is
        // deterministic; it hashes the canonical serialization.
        const EVENT_ID: &str = "1563574a586fc056e0fa48e884da43e1acc2f00ceba7931a557ceb46be339073";

        let agent = Keys::new(SecretKey::parse(AGENT_SECRET_HEX).unwrap());
        let owner = Keys::new(SecretKey::parse(OWNER_SECRET_HEX).unwrap());
        assert_eq!(npub(&agent), AGENT_NPUB);
        assert_eq!(npub(&owner), OWNER_NPUB);
        assert_eq!(
            personal_brain_id_for_owner(OWNER_NPUB).unwrap().as_str(),
            BRAIN_ID
        );

        let payload = PersonalAgentConsentPayload {
            version: PERSONAL_AGENT_CONSENT_VERSION.to_owned(),
            agent_npub: AGENT_NPUB.to_owned(),
            owner_npub: OWNER_NPUB.to_owned(),
            brain_id: BRAIN_ID.to_owned(),
            brain_server: SERVER.to_owned(),
            nonce: NONCE.to_owned(),
            expires_at: 1_780_000_600,
        };
        assert_eq!(payload.canonical_json(), CANONICAL_JSON);
        let template = personal_agent_consent_event_template(&payload, 1_780_000_000).unwrap();
        assert_eq!(template.kind, 30_078);
        assert_eq!(
            template.tags,
            vec![
                vec![
                    "d".to_owned(),
                    format!("finite-brain-personal-agent-consent:{BRAIN_ID}:{NONCE}"),
                ],
                vec!["brain".to_owned(), BRAIN_ID.to_owned()],
                vec!["owner".to_owned(), OWNER_NPUB.to_owned()],
                vec!["nonce".to_owned(), NONCE.to_owned()],
            ]
        );

        let event = sign_brain_event_template(&agent, &template).unwrap();
        assert_eq!(event.id.to_hex(), EVENT_ID);
        let parsed = Event::from_json(event.as_json()).unwrap();
        assert_eq!(parsed.id.to_hex(), EVENT_ID);
        assert_eq!(
            verify_personal_agent_consent_event(&parsed).unwrap(),
            payload
        );
    }

    #[test]
    fn consent_nonces_are_fresh_lowercase_hex() {
        let first = personal_agent_consent_nonce();
        let agent = Keys::generate();
        let owner = Keys::generate();
        let payload = PersonalAgentConsentPayload {
            nonce: first.clone(),
            ..payload(&agent, &owner)
        };
        assert!(validate_personal_agent_consent_payload(&payload).is_ok());
        assert_ne!(first, personal_agent_consent_nonce());
    }

    #[test]
    fn only_the_exact_personal_brain_id_shape_is_reserved() {
        let owner = npub(&Keys::generate());
        assert!(is_reserved_personal_brain_id(
            personal_brain_id_for_owner(&owner).unwrap().as_str()
        ));
        for allowed in [
            "personal",
            "personal-notes",
            "personal-0123456789abcde",
            "personal-0123456789abcdef0",
            "personal-0123456789ABCDEF",
            "acme-0123456789abcdef",
        ] {
            assert!(!is_reserved_personal_brain_id(allowed), "{allowed}");
        }
    }

    #[test]
    fn personal_brain_id_uses_the_owner_key_prefix() {
        let owner = Keys::generate();
        let hex = owner.public_key().to_hex();
        assert_eq!(
            personal_brain_id_for_owner(&npub(&owner)).unwrap().as_str(),
            format!("personal-{}", &hex[..16])
        );
        assert!(personal_brain_id_for_owner(&hex).is_err());
    }

    #[test]
    fn consent_round_trips_and_binds_its_signer() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let payload = payload(&agent, &owner);
        let event = sign(&agent, &payload);
        assert_eq!(
            verify_personal_agent_consent_event(&event).unwrap(),
            payload
        );

        let signed_by_owner = sign(&owner, &payload);
        assert!(verify_personal_agent_consent_event(&signed_by_owner).is_err());
    }

    #[test]
    fn consent_payload_rejects_unbound_or_malformed_fields() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let valid = payload(&agent, &owner);
        let other_owner = npub(&Keys::generate());
        let cases = [
            PersonalAgentConsentPayload {
                version: "finite-brain-personal-agent-consent-v0".to_owned(),
                ..valid.clone()
            },
            PersonalAgentConsentPayload {
                owner_npub: valid.agent_npub.clone(),
                ..valid.clone()
            },
            PersonalAgentConsentPayload {
                owner_npub: other_owner,
                ..valid.clone()
            },
            PersonalAgentConsentPayload {
                brain_id: "personal-0000000000000000".to_owned(),
                ..valid.clone()
            },
            PersonalAgentConsentPayload {
                agent_npub: valid.agent_npub.to_uppercase(),
                ..valid.clone()
            },
            PersonalAgentConsentPayload {
                brain_server: format!("{SERVER}/"),
                ..valid.clone()
            },
            PersonalAgentConsentPayload {
                brain_server: "ftp://brain.example".to_owned(),
                ..valid.clone()
            },
            PersonalAgentConsentPayload {
                nonce: "AB".repeat(16),
                ..valid.clone()
            },
            PersonalAgentConsentPayload {
                expires_at: 0,
                ..valid.clone()
            },
        ];
        for case in cases {
            assert!(
                validate_personal_agent_consent_payload(&case).is_err(),
                "accepted {case:?}"
            );
        }
        assert!(validate_personal_agent_consent_payload(&valid).is_ok());
    }

    #[test]
    fn consent_window_is_short_and_forward() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let valid = payload(&agent, &owner);
        for expires_at in [
            1_780_000_000,
            1_780_000_000 + MAX_PERSONAL_AGENT_CONSENT_SECONDS + 1,
        ] {
            let payload = PersonalAgentConsentPayload {
                expires_at,
                ..valid.clone()
            };
            assert!(personal_agent_consent_event_template(&payload, 1_780_000_000).is_err());
        }
    }

    #[test]
    fn verification_enforces_the_signing_window() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let valid = payload(&agent, &owner);
        // Sign templates the builder would refuse: verification must refuse them too.
        for expires_at in [
            1_780_000_000 + MAX_PERSONAL_AGENT_CONSENT_SECONDS + 1,
            1_780_000_000 - 1,
        ] {
            let mut template =
                personal_agent_consent_event_template(&valid, 1_780_000_000).unwrap();
            template.content = PersonalAgentConsentPayload {
                expires_at,
                ..valid.clone()
            }
            .canonical_json();
            let event = sign_brain_event_template(&agent, &template).unwrap();
            assert!(
                verify_personal_agent_consent_event(&event).is_err(),
                "accepted expiry {expires_at}"
            );
        }
    }

    #[test]
    fn consent_event_rejects_tampering_and_tag_drift() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let payload = payload(&agent, &owner);

        let mut template = personal_agent_consent_event_template(&payload, 1_780_000_000).unwrap();
        template.tags.pop();
        let drifted = sign_brain_event_template(&agent, &template).unwrap();
        assert!(verify_personal_agent_consent_event(&drifted).is_err());

        let mut template = personal_agent_consent_event_template(&payload, 1_780_000_000).unwrap();
        template.content = template.content.replace("\"nonce\"", " \"nonce\"");
        let noncanonical = sign_brain_event_template(&agent, &template).unwrap();
        assert!(verify_personal_agent_consent_event(&noncanonical).is_err());

        let event = sign(&agent, &payload);
        let mut forged: serde_json::Value = serde_json::from_str(&event.as_json()).unwrap();
        forged["content"] = serde_json::Value::String(
            event
                .content
                .replace(&payload.owner_npub, &npub(&Keys::generate())),
        );
        // Either parsing or verification must refuse content changed after signing.
        assert!(Event::from_json(forged.to_string()).map_or(true, |forged| {
            verify_personal_agent_consent_event(&forged).is_err()
        }));
    }
}
