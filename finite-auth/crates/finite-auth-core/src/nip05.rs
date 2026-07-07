use std::collections::BTreeMap;
use std::fmt;

use finite_nostr::NostrPublicKey;
use serde::Deserialize;

use crate::AuthError;

/// Largest NIP-05 well-known document accepted by this scaffold.
pub const MAX_NIP05_DOCUMENT_BYTES: usize = 64 * 1024;
/// Largest number of names accepted in one NIP-05 document.
pub const MAX_NIP05_NAMES: usize = 1_024;
/// Largest number of relay URLs attached to one public key.
pub const MAX_NIP05_RELAY_URLS: usize = 32;

const MAX_NIP05_IDENTIFIER_LEN: usize = 320;
const MAX_NIP05_LOCAL_PART_LEN: usize = 64;
const MAX_NIP05_DOMAIN_LEN: usize = 253;
const MAX_RELAY_URL_LEN: usize = 2_048;

/// Parsed NIP-05 internet identifier.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Nip05Identifier {
    value: String,
    local_part: String,
    domain: String,
}

impl Nip05Identifier {
    /// Parse and normalize a NIP-05 identifier.
    pub fn parse(value: &str) -> Result<Self, AuthError> {
        if value.is_empty() || value.len() > MAX_NIP05_IDENTIFIER_LEN {
            return Err(AuthError::LimitExceeded {
                field: "nip05_identifier",
                limit: MAX_NIP05_IDENTIFIER_LEN,
                actual: value.len(),
            });
        }

        if value.matches('@').count() != 1 {
            return Err(AuthError::InvalidInput {
                field: "nip05_identifier",
                reason: "expected exactly one @".to_string(),
            });
        }

        let (local_part, domain) =
            value
                .split_once('@')
                .ok_or_else(|| AuthError::InvalidInput {
                    field: "nip05_identifier",
                    reason: "expected local-part@domain".to_string(),
                })?;
        validate_local_part(local_part)?;
        let domain = normalize_domain(domain)?;
        let value = format!("{local_part}@{domain}");

        Ok(Self {
            value,
            local_part: local_part.to_string(),
            domain,
        })
    }

    /// Full normalized identifier.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// NIP-05 local part.
    pub fn local_part(&self) -> &str {
        &self.local_part
    }

    /// NIP-05 domain.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// Display form. `_@domain` may be displayed as just the domain.
    pub fn display_name(&self) -> &str {
        if self.local_part == "_" {
            &self.domain
        } else {
            &self.value
        }
    }

    /// Fetch request facts for a NIP-05 well-known lookup.
    pub fn well_known_request(&self) -> Nip05WellKnownRequest {
        Nip05WellKnownRequest {
            url: format!(
                "https://{}/.well-known/nostr.json?name={}",
                self.domain, self.local_part
            ),
            max_response_bytes: MAX_NIP05_DOCUMENT_BYTES,
            follow_redirects: false,
        }
    }
}

impl fmt::Display for Nip05Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Fetcher policy for a NIP-05 well-known document.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Nip05WellKnownRequest {
    /// Absolute HTTPS well-known URL.
    pub url: String,
    /// Maximum accepted response size.
    pub max_response_bytes: usize,
    /// NIP-05 forbids following redirects for this endpoint.
    pub follow_redirects: bool,
}

/// Parsed NIP-05 well-known document.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub struct Nip05WellKnownDocument {
    names: BTreeMap<String, String>,
    #[serde(default)]
    relays: BTreeMap<String, Vec<String>>,
}

impl Nip05WellKnownDocument {
    /// Parse and validate a bounded NIP-05 document.
    pub fn from_json(bytes: &[u8]) -> Result<Self, AuthError> {
        if bytes.len() > MAX_NIP05_DOCUMENT_BYTES {
            return Err(AuthError::LimitExceeded {
                field: "nip05_document",
                limit: MAX_NIP05_DOCUMENT_BYTES,
                actual: bytes.len(),
            });
        }

        let document: Self =
            serde_json::from_slice(bytes).map_err(|error| AuthError::MalformedNip05Document {
                reason: error.to_string(),
            })?;
        document.validate_shape()?;
        Ok(document)
    }

    /// Verify the document maps an identifier to the expected public key.
    pub fn verify(
        &self,
        identifier: &Nip05Identifier,
        expected_public_key: NostrPublicKey,
    ) -> Result<VerifiedNip05, AuthError> {
        let actual_hex =
            self.names
                .get(identifier.local_part())
                .ok_or_else(|| AuthError::Nip05NameMissing {
                    identifier: identifier.as_str().to_string(),
                })?;
        let actual_public_key = parse_lowercase_public_key(actual_hex)?;
        if actual_public_key != expected_public_key {
            return Err(AuthError::Nip05PublicKeyMismatch {
                identifier: identifier.as_str().to_string(),
                expected: expected_public_key.to_hex(),
                actual: actual_public_key.to_hex(),
            });
        }

        let relays = self
            .relays
            .get(&expected_public_key.to_hex())
            .cloned()
            .unwrap_or_default();
        validate_relay_urls(&relays)?;

        Ok(VerifiedNip05 {
            identifier: identifier.clone(),
            public_key: expected_public_key,
            relays,
        })
    }

    fn validate_shape(&self) -> Result<(), AuthError> {
        if self.names.len() > MAX_NIP05_NAMES {
            return Err(AuthError::LimitExceeded {
                field: "nip05_names",
                limit: MAX_NIP05_NAMES,
                actual: self.names.len(),
            });
        }

        for (name, public_key) in &self.names {
            validate_local_part(name)?;
            parse_lowercase_public_key(public_key)?;
        }

        for (public_key, relays) in &self.relays {
            parse_lowercase_public_key(public_key)?;
            validate_relay_urls(relays)?;
        }

        Ok(())
    }
}

/// Verified current NIP-05 mapping for a Nostr public key.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct VerifiedNip05 {
    identifier: Nip05Identifier,
    public_key: NostrPublicKey,
    relays: Vec<String>,
}

impl VerifiedNip05 {
    /// Verified identifier.
    pub fn identifier(&self) -> &Nip05Identifier {
        &self.identifier
    }

    /// Public key the identifier mapped to.
    pub fn public_key(&self) -> NostrPublicKey {
        self.public_key
    }

    /// Optional relay hints attached by the NIP-05 document.
    pub fn relays(&self) -> &[String] {
        &self.relays
    }
}

fn validate_local_part(value: &str) -> Result<(), AuthError> {
    if value.is_empty() || value.len() > MAX_NIP05_LOCAL_PART_LEN {
        return Err(AuthError::LimitExceeded {
            field: "nip05_local_part",
            limit: MAX_NIP05_LOCAL_PART_LEN,
            actual: value.len(),
        });
    }

    if !value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
    }) {
        return Err(AuthError::InvalidInput {
            field: "nip05_local_part",
            reason: "expected a-z, 0-9, dash, underscore, or dot".to_string(),
        });
    }

    Ok(())
}

fn normalize_domain(value: &str) -> Result<String, AuthError> {
    if value.is_empty() || value.len() > MAX_NIP05_DOMAIN_LEN {
        return Err(AuthError::LimitExceeded {
            field: "nip05_domain",
            limit: MAX_NIP05_DOMAIN_LEN,
            actual: value.len(),
        });
    }

    let domain = value.to_ascii_lowercase();
    let labels = domain.split('.').collect::<Vec<_>>();
    if labels
        .iter()
        .any(|label| label.is_empty() || label.starts_with('-') || label.ends_with('-'))
    {
        return Err(AuthError::InvalidInput {
            field: "nip05_domain",
            reason: "expected non-empty DNS labels without leading or trailing dash".to_string(),
        });
    }

    if !domain.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
    }) {
        return Err(AuthError::InvalidInput {
            field: "nip05_domain",
            reason: "expected ASCII DNS name".to_string(),
        });
    }

    Ok(domain)
}

fn parse_lowercase_public_key(value: &str) -> Result<NostrPublicKey, AuthError> {
    let public_key = NostrPublicKey::from_hex(value)?;
    if public_key.to_hex() != value {
        return Err(AuthError::Nip05KeyNotLowercaseHex {
            value: value.to_string(),
        });
    }

    Ok(public_key)
}

fn validate_relay_urls(relays: &[String]) -> Result<(), AuthError> {
    if relays.len() > MAX_NIP05_RELAY_URLS {
        return Err(AuthError::LimitExceeded {
            field: "nip05_relays",
            limit: MAX_NIP05_RELAY_URLS,
            actual: relays.len(),
        });
    }

    for relay in relays {
        if relay.is_empty() || relay.len() > MAX_RELAY_URL_LEN {
            return Err(AuthError::LimitExceeded {
                field: "nip05_relay_url",
                limit: MAX_RELAY_URL_LEN,
                actual: relay.len(),
            });
        }

        let supported_scheme = relay.starts_with("wss://") || relay.starts_with("ws://");
        let unsafe_bytes = relay.bytes().any(|byte| byte <= 0x20 || byte == 0x7f);
        if !supported_scheme || unsafe_bytes {
            return Err(AuthError::InvalidInput {
                field: "nip05_relay_url",
                reason: "expected ws or wss URL without whitespace".to_string(),
            });
        }
    }

    Ok(())
}
