//! Invite code v1 (ADR 0006): the URL/QR payload and the join proof that
//! binds the invite token to the joiner's exact identity and KeyPackage bytes.
//!
//! Everything here is pure protocol: no I/O, no clocks (callers pass unix
//! seconds), no server trust. The rendezvous server never sees the invite
//! token and cannot mint a passing proof.

use crate::{
    INVITE_CODE_VERSION_V1, INVITE_JOIN_PROOF_DOMAIN, INVITE_TOKEN_BYTES,
    MAX_INVITE_DISPLAY_NAME_BYTES, MAX_OBJECT_ID_BYTES, MAX_ROOM_ID_BYTES,
};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

pub const INVITE_URL_SCHEME: &str = "finite";
pub const INVITE_URL_PREFIX: &str = "finite://join?";
pub const NOSTR_NPUB_HRP: &str = "npub";
pub const NOSTR_NPROFILE_HRP: &str = "nprofile";
pub const NOSTR_NSEC_HRP: &str = "nsec";
pub const MAX_INVITE_URL_BYTES: usize = 2048;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InviteCodeError {
    #[error("invite code must start with {INVITE_URL_PREFIX}")]
    NotAnInviteUrl,
    #[error("invite code version {0} is not supported")]
    UnsupportedVersion(String),
    #[error("invite code is missing required field {0}")]
    MissingField(&'static str),
    #[error("invite code field {field} is invalid: {reason}")]
    InvalidField { field: &'static str, reason: String },
    #[error("invite code is too long")]
    TooLong,
}

/// The parsed form of a v1 invite URL. The room-server address is the
/// first-class field (ADR 0005): joining a room is discovering where it
/// lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteCodeV1 {
    pub server_url: String,
    pub room_id: String,
    pub invite_id: String,
    pub invite_token: Vec<u8>,
    /// The inviting agent's account id (lowercase hex of the account public
    /// key). Displayed as an npub; verified against the joined group's
    /// member credentials after Welcome activation.
    pub inviter_account_id: String,
    pub display_name: Option<String>,
}

impl InviteCodeV1 {
    pub fn encode(&self) -> Result<String, InviteCodeError> {
        self.validate()?;
        let mut url = String::with_capacity(256);
        url.push_str(INVITE_URL_PREFIX);
        url.push_str("v=1");
        url.push_str("&s=");
        url.push_str(&percent_encode(&self.server_url));
        url.push_str("&r=");
        url.push_str(&percent_encode(&self.room_id));
        url.push_str("&i=");
        url.push_str(&percent_encode(&self.invite_id));
        url.push_str("&t=");
        url.push_str(&hex_lower(&self.invite_token));
        url.push_str("&a=");
        url.push_str(&npub_encode(&self.inviter_account_id).map_err(|reason| {
            InviteCodeError::InvalidField {
                field: "inviter_account_id",
                reason,
            }
        })?);
        if let Some(name) = &self.display_name {
            url.push_str("&n=");
            url.push_str(&percent_encode(name));
        }
        if url.len() > MAX_INVITE_URL_BYTES {
            return Err(InviteCodeError::TooLong);
        }
        Ok(url)
    }

    pub fn parse(input: &str) -> Result<Self, InviteCodeError> {
        if input.len() > MAX_INVITE_URL_BYTES {
            return Err(InviteCodeError::TooLong);
        }
        let trimmed = input.trim();
        let query = trimmed
            .strip_prefix(INVITE_URL_PREFIX)
            .ok_or(InviteCodeError::NotAnInviteUrl)?;
        let mut version = None;
        let mut server_url = None;
        let mut room_id = None;
        let mut invite_id = None;
        let mut invite_token = None;
        let mut inviter_account_id = None;
        let mut display_name = None;
        for pair in query.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            match key {
                "v" => version = Some(value.to_owned()),
                "s" => server_url = Some(percent_decode("s", value)?),
                "r" => room_id = Some(percent_decode("r", value)?),
                "i" => invite_id = Some(percent_decode("i", value)?),
                "t" => {
                    invite_token = Some(
                        decode_hex(value).map_err(|reason| InviteCodeError::InvalidField {
                            field: "t",
                            reason,
                        })?,
                    )
                }
                "a" => {
                    inviter_account_id =
                        Some(npub_decode(value).map_err(|reason| {
                            InviteCodeError::InvalidField { field: "a", reason }
                        })?)
                }
                "n" => display_name = Some(percent_decode("n", value)?),
                // Unknown fields are ignored so v1 parsers tolerate
                // forward-compatible additions; unknown *versions* are not.
                _ => {}
            }
        }
        match version.as_deref() {
            Some("1") => {}
            Some(other) => return Err(InviteCodeError::UnsupportedVersion(other.to_owned())),
            None => return Err(InviteCodeError::MissingField("v")),
        }
        let code = Self {
            server_url: server_url.ok_or(InviteCodeError::MissingField("s"))?,
            room_id: room_id.ok_or(InviteCodeError::MissingField("r"))?,
            invite_id: invite_id.ok_or(InviteCodeError::MissingField("i"))?,
            invite_token: invite_token.ok_or(InviteCodeError::MissingField("t"))?,
            inviter_account_id: inviter_account_id.ok_or(InviteCodeError::MissingField("a"))?,
            display_name,
        };
        code.validate()?;
        Ok(code)
    }

    fn validate(&self) -> Result<(), InviteCodeError> {
        let _ = INVITE_CODE_VERSION_V1;
        check_field("s", &self.server_url, MAX_INVITE_URL_BYTES)?;
        if !self.server_url.starts_with("http://") && !self.server_url.starts_with("https://") {
            return Err(InviteCodeError::InvalidField {
                field: "s",
                reason: "server url must be http(s)".to_owned(),
            });
        }
        check_field("r", &self.room_id, MAX_ROOM_ID_BYTES as usize)?;
        check_field("i", &self.invite_id, MAX_OBJECT_ID_BYTES as usize)?;
        if self.invite_token.len() != INVITE_TOKEN_BYTES as usize {
            return Err(InviteCodeError::InvalidField {
                field: "t",
                reason: format!("invite token must be {INVITE_TOKEN_BYTES} bytes"),
            });
        }
        if self.inviter_account_id.len() != 64
            || !self
                .inviter_account_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(InviteCodeError::InvalidField {
                field: "a",
                reason: "inviter account id must be 64 lowercase hex characters".to_owned(),
            });
        }
        if let Some(name) = &self.display_name {
            check_field("n", name, MAX_INVITE_DISPLAY_NAME_BYTES as usize)?;
        }
        Ok(())
    }
}

fn check_field(field: &'static str, value: &str, max_bytes: usize) -> Result<(), InviteCodeError> {
    if value.is_empty() || value.len() > max_bytes {
        return Err(InviteCodeError::InvalidField {
            field,
            reason: format!("must contain between 1 and {max_bytes} bytes"),
        });
    }
    Ok(())
}

/// The proof a joiner submits. Binds the invite token to the joiner's account,
/// device, and exact KeyPackage bytes, so the rendezvous server cannot mint a
/// proof or substitute identity/key material.
pub fn invite_join_proof(
    invite_token: &[u8],
    account_id: &str,
    device_id: &str,
    key_package: &[u8],
) -> String {
    let mut mac = HmacSha256::new_from_slice(invite_token).expect("HMAC accepts any key length");
    mac.update(INVITE_JOIN_PROOF_DOMAIN);
    mac.update(account_id.as_bytes());
    mac.update(&[0]);
    mac.update(device_id.as_bytes());
    mac.update(&[0]);
    mac.update(&Sha256::digest(key_package));
    hex_lower(&mac.finalize().into_bytes())
}

/// Inviter-side verification: recompute the proof from the invite token and
/// the pending request material.
pub fn verify_invite_join_proof(
    invite_token: &[u8],
    account_id: &str,
    device_id: &str,
    key_package: &[u8],
    proof: &str,
) -> bool {
    let expected = invite_join_proof(invite_token, account_id, device_id, key_package);
    // Proofs are one-shot rendezvous artifacts, not an oracle the attacker can
    // query repeatedly, so plain comparison is fine here.
    expected == proof
}

pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("hex value must have even length".to_owned());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16).map_err(|_| "invalid hex".to_owned())
        })
        .collect()
}

/// NIP-19 npub display form of a 64-hex account id.
pub fn npub_encode(account_id_hex: &str) -> Result<String, String> {
    let bytes = decode_hex(account_id_hex)?;
    if bytes.len() != 32 {
        return Err("account id must be 32 bytes of hex".to_owned());
    }
    let hrp = bech32::Hrp::parse(NOSTR_NPUB_HRP).expect("static hrp");
    bech32::encode::<bech32::Bech32>(hrp, &bytes).map_err(|error| error.to_string())
}

/// Decode an npub back to the 64-hex account id.
pub fn npub_decode(npub: &str) -> Result<String, String> {
    let (hrp, bytes) = bech32::decode(npub).map_err(|error| error.to_string())?;
    if hrp.as_str() != NOSTR_NPUB_HRP {
        return Err(format!("expected {NOSTR_NPUB_HRP}, got {hrp}"));
    }
    if bytes.len() != 32 {
        return Err("npub must decode to 32 bytes".to_owned());
    }
    Ok(hex_lower(&bytes))
}

/// Decode a NIP-19 nprofile back to the embedded 64-hex account id.
pub fn nprofile_decode(nprofile: &str) -> Result<String, String> {
    let (hrp, bytes) = bech32::decode(nprofile).map_err(|error| error.to_string())?;
    if hrp.as_str() != NOSTR_NPROFILE_HRP {
        return Err(format!("expected {NOSTR_NPROFILE_HRP}, got {hrp}"));
    }

    let mut offset = 0;
    while offset < bytes.len() {
        if offset + 2 > bytes.len() {
            return Err("nprofile contains truncated TLV header".to_owned());
        }
        let tag = bytes[offset];
        let length = bytes[offset + 1] as usize;
        offset += 2;
        if offset + length > bytes.len() {
            return Err("nprofile contains truncated TLV value".to_owned());
        }
        let value = &bytes[offset..offset + length];
        offset += length;
        if tag == 0 {
            if value.len() != 32 {
                return Err("nprofile pubkey TLV must be 32 bytes".to_owned());
            }
            return Ok(hex_lower(value));
        }
    }

    Err("nprofile is missing pubkey TLV".to_owned())
}

/// NIP-19 nsec display form of a 32-byte account secret.
pub fn nsec_encode(secret_hex: &str) -> Result<String, String> {
    let bytes = decode_hex(secret_hex)?;
    if bytes.len() != 32 {
        return Err("account secret must be 32 bytes of hex".to_owned());
    }
    let hrp = bech32::Hrp::parse(NOSTR_NSEC_HRP).expect("static hrp");
    bech32::encode::<bech32::Bech32>(hrp, &bytes).map_err(|error| error.to_string())
}

/// Decode an nsec back to the 32-byte account secret hex.
pub fn nsec_decode(nsec: &str) -> Result<String, String> {
    let (hrp, bytes) = bech32::decode(nsec).map_err(|error| error.to_string())?;
    if hrp.as_str() != NOSTR_NSEC_HRP {
        return Err(format!("expected {NOSTR_NSEC_HRP}, got {hrp}"));
    }
    if bytes.len() != 32 {
        return Err("nsec must decode to 32 bytes".to_owned());
    }
    Ok(hex_lower(&bytes))
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn percent_decode(field: &'static str, value: &str) -> Result<String, InviteCodeError> {
    let mut out = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                if index + 2 >= bytes.len() + 1 {
                    return Err(InviteCodeError::InvalidField {
                        field,
                        reason: "truncated percent escape".to_owned(),
                    });
                }
                let hex = value.get(index + 1..index + 3).ok_or_else(|| {
                    InviteCodeError::InvalidField {
                        field,
                        reason: "truncated percent escape".to_owned(),
                    }
                })?;
                let byte =
                    u8::from_str_radix(hex, 16).map_err(|_| InviteCodeError::InvalidField {
                        field,
                        reason: "invalid percent escape".to_owned(),
                    })?;
                out.push(byte);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            other => {
                out.push(other);
                index += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| InviteCodeError::InvalidField {
        field,
        reason: "value is not valid UTF-8".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: [u8; 16] = [7; 16];

    fn account_hex() -> String {
        hex_lower(&[0xab; 32])
    }

    fn sample() -> InviteCodeV1 {
        InviteCodeV1 {
            server_url: "http://127.0.0.1:8787".to_owned(),
            room_id: "room-agent-dm".to_owned(),
            invite_id: "invite-1".to_owned(),
            invite_token: TOKEN.to_vec(),
            inviter_account_id: account_hex(),
            display_name: Some("Hermes Agent".to_owned()),
        }
    }

    #[test]
    fn invite_code_round_trips_with_escaping_and_npub() {
        let code = sample();
        let url = code.encode().expect("encode");
        assert!(url.starts_with("finite://join?v=1&s=http%3A%2F%2F127.0.0.1%3A8787&r="));
        assert!(url.contains("&a=npub1"));
        assert!(url.contains("&n=Hermes%20Agent"));
        let parsed = InviteCodeV1::parse(&url).expect("parse");
        assert_eq!(parsed, code);
    }

    #[test]
    fn invite_code_rejects_unknown_version_and_missing_fields() {
        let url = sample().encode().expect("encode");
        let v2 = url.replace("v=1", "v=2");
        assert_eq!(
            InviteCodeV1::parse(&v2),
            Err(InviteCodeError::UnsupportedVersion("2".to_owned()))
        );
        let missing_token = url.replace("&t=", "&x=");
        assert!(matches!(
            InviteCodeV1::parse(&missing_token),
            Err(InviteCodeError::MissingField("t") | InviteCodeError::InvalidField { .. })
        ));
        assert_eq!(
            InviteCodeV1::parse("https://example.com/join?v=1"),
            Err(InviteCodeError::NotAnInviteUrl)
        );
    }

    #[test]
    fn invite_code_ignores_unknown_fields_for_forward_compat() {
        let url = format!("{}&future=value", sample().encode().expect("encode"));
        assert_eq!(InviteCodeV1::parse(&url).expect("parse"), sample());
    }

    #[test]
    fn join_proof_verifies_and_binds_token_identity_and_bytes() {
        let proof = invite_join_proof(&TOKEN, "acct", "device", b"kp-bytes");
        assert!(verify_invite_join_proof(
            &TOKEN,
            "acct",
            "device",
            b"kp-bytes",
            &proof
        ));
        // Any tampering kills it: token, identity, device, or key package bytes.
        assert!(!verify_invite_join_proof(
            &[8; 16],
            "acct",
            "device",
            b"kp-bytes",
            &proof
        ));
        assert!(!verify_invite_join_proof(
            &TOKEN,
            "other",
            "device",
            b"kp-bytes",
            &proof
        ));
        assert!(!verify_invite_join_proof(
            &TOKEN,
            "acct",
            "other",
            b"kp-bytes",
            &proof
        ));
        assert!(!verify_invite_join_proof(
            &TOKEN,
            "acct",
            "device",
            b"tampered",
            &proof
        ));
    }

    #[test]
    fn npub_round_trips() {
        let hex = account_hex();
        let npub = npub_encode(&hex).expect("encode");
        assert!(npub.starts_with("npub1"));
        assert_eq!(npub_decode(&npub).expect("decode"), hex);
        assert!(npub_decode("nsec1qqqq").is_err());
    }

    #[test]
    fn nprofile_decodes_pubkey_tlv() {
        let hex = account_hex();
        let mut payload = vec![0, 32];
        payload.extend(decode_hex(&hex).expect("hex"));
        let nprofile = bech32::encode::<bech32::Bech32>(
            bech32::Hrp::parse(NOSTR_NPROFILE_HRP).expect("hrp"),
            &payload,
        )
        .expect("encode");

        assert!(nprofile.starts_with("nprofile1"));
        assert_eq!(nprofile_decode(&nprofile).expect("decode"), hex);
    }

    #[test]
    fn nprofile_skips_relay_tlv_before_pubkey() {
        let hex = account_hex();
        let relay = b"wss://relay.example";
        let mut payload = vec![1, relay.len() as u8];
        payload.extend(relay);
        payload.extend([0, 32]);
        payload.extend(decode_hex(&hex).expect("hex"));
        let nprofile = bech32::encode::<bech32::Bech32>(
            bech32::Hrp::parse(NOSTR_NPROFILE_HRP).expect("hrp"),
            &payload,
        )
        .expect("encode");

        assert_eq!(nprofile_decode(&nprofile).expect("decode"), hex);
    }

    #[test]
    fn nsec_round_trips() {
        let hex = "1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100";
        let nsec = nsec_encode(hex).expect("encode");
        assert!(nsec.starts_with("nsec1"));
        assert_eq!(nsec_decode(&nsec).expect("decode"), hex);
        assert!(nsec_decode("npub1qqqq").is_err());
    }
}
