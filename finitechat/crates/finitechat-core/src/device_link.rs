use finite_nostr::{NostrPublicKey, decrypt_nip44, encrypt_nip44};
use nostr::{Keys, SecretKey};
use serde::{Deserialize, Serialize};

use crate::{FiniteChatCoreError, nostr_identity_from_account_secret_hex};

pub const DEVICE_LINK_PAYLOAD_VERSION: u16 = 1;
pub const DEVICE_LINK_PURPOSE: &str = "finitechat.device-link.v1";
pub const DEVICE_LINK_MAX_TTL_SECONDS: u64 = 10 * 60;
const DEVICE_LINK_CLOCK_SKEW_SECONDS: u64 = 60;
const MAX_LINK_SESSION_ID_BYTES: usize = 256;
const MAX_DEVICE_ID_BYTES: usize = 256;
const MAX_SERVER_URL_BYTES: usize = 2_048;
const MAX_ENCRYPTED_LINK_PAYLOAD_BYTES: usize = 16 * 1_024;

/// Ephemeral receiver material for one device-link rendezvous. The secret is
/// held by the native bootstrap process only; it is never an AppState field or
/// browser payload.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceLinkPairingKey {
    pub secret_key_hex: String,
    pub public_key_hex: String,
}

/// Plaintext protected by NIP-44. Every routing field is repeated inside the
/// authenticated ciphertext so a relay, browser, or stale callback cannot
/// substitute the target Device or server.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLinkPayloadV1 {
    pub version: u16,
    pub purpose: String,
    pub link_session_id: String,
    pub pairing_public_key: String,
    pub account_secret_hex: String,
    pub account_id: String,
    pub target_device_id: String,
    pub server_url: String,
    pub issued_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
}

/// Public envelope stored by the opaque link-session rendezvous. The sender
/// account id is needed to derive the NIP-44 receive key; the decrypted payload
/// must prove that the transferred secret derives that same account id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedDeviceLinkPayloadV1 {
    pub version: u16,
    pub sender_account_id: String,
    pub ciphertext: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DeviceLinkEncryptInput {
    pub account_secret_hex: String,
    pub pairing_public_key: String,
    pub link_session_id: String,
    pub target_device_id: String,
    pub server_url: String,
    pub issued_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DeviceLinkDecryptInput {
    pub pairing_secret_key_hex: String,
    pub encrypted_payload: Vec<u8>,
    pub expected_link_session_id: String,
    pub expected_pairing_public_key: String,
    pub expected_target_device_id: String,
    pub expected_server_url: String,
    pub now_unix_seconds: u64,
}

pub fn create_device_link_pairing_key() -> DeviceLinkPairingKey {
    let keys = Keys::generate();
    DeviceLinkPairingKey {
        secret_key_hex: keys.secret_key().to_secret_hex(),
        public_key_hex: NostrPublicKey::from_protocol(keys.public_key()).to_hex(),
    }
}

pub fn encrypt_device_link_payload(
    input: DeviceLinkEncryptInput,
) -> Result<Vec<u8>, FiniteChatCoreError> {
    validate_link_fields(
        &input.link_session_id,
        &input.target_device_id,
        &input.server_url,
    )?;
    validate_validity_window(input.issued_at_unix_seconds, input.expires_at_unix_seconds)?;
    let identity = nostr_identity_from_account_secret_hex(input.account_secret_hex.clone())?;
    let sender_secret = secret_key_from_hex(&identity.account_secret_hex)?;
    let recipient = NostrPublicKey::from_hex(&input.pairing_public_key)
        .map_err(|_| link_error("pairing public key is invalid"))?;
    let pairing_public_key = recipient.to_hex();
    let payload = DeviceLinkPayloadV1 {
        version: DEVICE_LINK_PAYLOAD_VERSION,
        purpose: DEVICE_LINK_PURPOSE.to_owned(),
        link_session_id: input.link_session_id,
        pairing_public_key,
        account_secret_hex: identity.account_secret_hex,
        account_id: identity.account_id.clone(),
        target_device_id: input.target_device_id,
        server_url: normalize_server_url(&input.server_url)?,
        issued_at_unix_seconds: input.issued_at_unix_seconds,
        expires_at_unix_seconds: input.expires_at_unix_seconds,
    };
    let plaintext = serde_json::to_vec(&payload).map_err(link_serialize_error)?;
    let ciphertext = encrypt_nip44(&sender_secret, recipient, plaintext)
        .map_err(|_| link_error("device-link encryption failed"))?;
    let envelope = EncryptedDeviceLinkPayloadV1 {
        version: DEVICE_LINK_PAYLOAD_VERSION,
        sender_account_id: identity.account_id,
        ciphertext,
    };
    let encoded = serde_json::to_vec(&envelope).map_err(link_serialize_error)?;
    if encoded.len() > MAX_ENCRYPTED_LINK_PAYLOAD_BYTES {
        return Err(link_error("encrypted device-link payload is too large"));
    }
    Ok(encoded)
}

pub fn decrypt_device_link_payload(
    input: DeviceLinkDecryptInput,
) -> Result<DeviceLinkPayloadV1, FiniteChatCoreError> {
    validate_link_fields(
        &input.expected_link_session_id,
        &input.expected_target_device_id,
        &input.expected_server_url,
    )?;
    if input.encrypted_payload.is_empty()
        || input.encrypted_payload.len() > MAX_ENCRYPTED_LINK_PAYLOAD_BYTES
    {
        return Err(link_error(
            "encrypted device-link payload has an invalid size",
        ));
    }
    let expected_pairing_public_key = NostrPublicKey::from_hex(&input.expected_pairing_public_key)
        .map_err(|_| link_error("pairing public key is invalid"))?
        .to_hex();
    let pairing_secret = secret_key_from_hex(&input.pairing_secret_key_hex)?;
    let actual_pairing_public_key =
        NostrPublicKey::from_protocol(Keys::new(pairing_secret.clone()).public_key()).to_hex();
    if actual_pairing_public_key != expected_pairing_public_key {
        return Err(link_error(
            "pairing secret does not match this link session",
        ));
    }

    let envelope: EncryptedDeviceLinkPayloadV1 =
        serde_json::from_slice(&input.encrypted_payload)
            .map_err(|_| link_error("encrypted device-link envelope is malformed"))?;
    if envelope.version != DEVICE_LINK_PAYLOAD_VERSION {
        return Err(link_error("device-link envelope version is unsupported"));
    }
    let sender = NostrPublicKey::from_hex(&envelope.sender_account_id)
        .map_err(|_| link_error("device-link sender account is invalid"))?;
    let plaintext = decrypt_nip44(&pairing_secret, sender, &envelope.ciphertext)
        .map_err(|_| link_error("device-link payload could not be decrypted"))?;
    let payload: DeviceLinkPayloadV1 = serde_json::from_str(&plaintext)
        .map_err(|_| link_error("decrypted device-link payload is malformed"))?;
    validate_decrypted_payload(&payload, &envelope, &input, &expected_pairing_public_key)?;
    Ok(payload)
}

fn validate_decrypted_payload(
    payload: &DeviceLinkPayloadV1,
    envelope: &EncryptedDeviceLinkPayloadV1,
    input: &DeviceLinkDecryptInput,
    expected_pairing_public_key: &str,
) -> Result<(), FiniteChatCoreError> {
    if payload.version != DEVICE_LINK_PAYLOAD_VERSION || payload.purpose != DEVICE_LINK_PURPOSE {
        return Err(link_error(
            "device-link payload version or purpose is unsupported",
        ));
    }
    if payload.link_session_id != input.expected_link_session_id
        || payload.target_device_id != input.expected_target_device_id
        || payload.pairing_public_key != expected_pairing_public_key
        || payload.server_url != normalize_server_url(&input.expected_server_url)?
    {
        return Err(link_error(
            "device-link payload is bound to a different request",
        ));
    }
    validate_validity_window(
        payload.issued_at_unix_seconds,
        payload.expires_at_unix_seconds,
    )?;
    if input
        .now_unix_seconds
        .saturating_add(DEVICE_LINK_CLOCK_SKEW_SECONDS)
        < payload.issued_at_unix_seconds
    {
        return Err(link_error("device-link payload is not valid yet"));
    }
    if input.now_unix_seconds > payload.expires_at_unix_seconds {
        return Err(link_error("device-link payload has expired"));
    }
    let identity = nostr_identity_from_account_secret_hex(payload.account_secret_hex.clone())?;
    if identity.account_id != payload.account_id
        || identity.account_id != envelope.sender_account_id
    {
        return Err(link_error("device-link account binding is invalid"));
    }
    Ok(())
}

fn validate_link_fields(
    link_session_id: &str,
    target_device_id: &str,
    server_url: &str,
) -> Result<(), FiniteChatCoreError> {
    validate_token(
        "link session id",
        link_session_id,
        MAX_LINK_SESSION_ID_BYTES,
    )?;
    validate_token("target device id", target_device_id, MAX_DEVICE_ID_BYTES)?;
    if target_device_id == "hosted-web" {
        return Err(link_error(
            "target device must be distinct from the Hosted Web Device",
        ));
    }
    let _ = normalize_server_url(server_url)?;
    Ok(())
}

fn validate_token(field: &str, value: &str, max_bytes: usize) -> Result<(), FiniteChatCoreError> {
    if value.is_empty()
        || value.len() > max_bytes
        || value.chars().any(char::is_control)
        || value.trim() != value
    {
        return Err(link_error(format!("{field} is invalid")));
    }
    Ok(())
}

fn normalize_server_url(value: &str) -> Result<String, FiniteChatCoreError> {
    if value.is_empty() || value.len() > MAX_SERVER_URL_BYTES || value.trim() != value {
        return Err(link_error("device-link server URL is invalid"));
    }
    let parsed =
        reqwest::Url::parse(value).map_err(|_| link_error("device-link server URL is invalid"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(link_error("device-link server URL is invalid"));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

fn validate_validity_window(issued_at: u64, expires_at: u64) -> Result<(), FiniteChatCoreError> {
    if expires_at <= issued_at || expires_at.saturating_sub(issued_at) > DEVICE_LINK_MAX_TTL_SECONDS
    {
        return Err(link_error("device-link validity window is invalid"));
    }
    Ok(())
}

fn secret_key_from_hex(value: &str) -> Result<SecretKey, FiniteChatCoreError> {
    let bytes = hex::decode(value).map_err(|_| link_error("device-link secret is invalid"))?;
    SecretKey::from_slice(&bytes).map_err(|_| link_error("device-link secret is invalid"))
}

fn link_error(reason: impl Into<String>) -> FiniteChatCoreError {
    FiniteChatCoreError::Client {
        reason: reason.into(),
    }
}

fn link_serialize_error(error: serde_json::Error) -> FiniteChatCoreError {
    link_error(format!("device-link serialization failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;
    const ACCOUNT_SECRET: &str = "6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e";

    fn encrypted(pairing: &DeviceLinkPairingKey) -> Vec<u8> {
        encrypt_device_link_payload(DeviceLinkEncryptInput {
            account_secret_hex: ACCOUNT_SECRET.to_owned(),
            pairing_public_key: pairing.public_key_hex.clone(),
            link_session_id: "link-test-1".to_owned(),
            target_device_id: "electron-test-1".to_owned(),
            server_url: "https://chat.finite.test/".to_owned(),
            issued_at_unix_seconds: NOW,
            expires_at_unix_seconds: NOW + 300,
        })
        .unwrap()
    }

    fn decrypt_input(
        pairing: &DeviceLinkPairingKey,
        encrypted_payload: Vec<u8>,
    ) -> DeviceLinkDecryptInput {
        DeviceLinkDecryptInput {
            pairing_secret_key_hex: pairing.secret_key_hex.clone(),
            encrypted_payload,
            expected_link_session_id: "link-test-1".to_owned(),
            expected_pairing_public_key: pairing.public_key_hex.clone(),
            expected_target_device_id: "electron-test-1".to_owned(),
            expected_server_url: "https://chat.finite.test".to_owned(),
            now_unix_seconds: NOW + 1,
        }
    }

    #[test]
    fn device_link_payload_round_trips_and_binds_every_route_field() {
        let pairing = create_device_link_pairing_key();
        let encrypted = encrypted(&pairing);
        let encoded_envelope = String::from_utf8(encrypted.clone()).unwrap();
        assert!(!encoded_envelope.contains(ACCOUNT_SECRET));
        assert!(!encoded_envelope.contains(&pairing.secret_key_hex));
        let payload = decrypt_device_link_payload(decrypt_input(&pairing, encrypted)).unwrap();
        assert_eq!(payload.version, DEVICE_LINK_PAYLOAD_VERSION);
        assert_eq!(payload.purpose, DEVICE_LINK_PURPOSE);
        assert_eq!(payload.link_session_id, "link-test-1");
        assert_eq!(payload.target_device_id, "electron-test-1");
        assert_eq!(payload.server_url, "https://chat.finite.test");
        assert_eq!(payload.account_secret_hex, ACCOUNT_SECRET);
        assert_eq!(payload.account_id.len(), 64);
    }

    #[test]
    fn device_link_rejects_wrong_pairing_key_tamper_and_expiry() {
        let pairing = create_device_link_pairing_key();
        let ciphertext = encrypted(&pairing);

        let wrong = create_device_link_pairing_key();
        let mut wrong_input = decrypt_input(&wrong, ciphertext.clone());
        wrong_input.expected_pairing_public_key = pairing.public_key_hex.clone();
        assert!(decrypt_device_link_payload(wrong_input).is_err());

        let mut tampered = ciphertext.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(decrypt_device_link_payload(decrypt_input(&pairing, tampered)).is_err());

        let mut expired = decrypt_input(&pairing, ciphertext);
        expired.now_unix_seconds = NOW + 301;
        assert!(decrypt_device_link_payload(expired).is_err());
    }

    #[test]
    fn device_link_rejects_substituted_session_device_and_server() {
        let pairing = create_device_link_pairing_key();
        let ciphertext = encrypted(&pairing);
        for mutate in [
            |input: &mut DeviceLinkDecryptInput| {
                input.expected_link_session_id = "link-other".to_owned()
            },
            |input: &mut DeviceLinkDecryptInput| {
                input.expected_target_device_id = "electron-other".to_owned()
            },
            |input: &mut DeviceLinkDecryptInput| {
                input.expected_server_url = "https://other.finite.test".to_owned()
            },
        ] {
            let mut input = decrypt_input(&pairing, ciphertext.clone());
            mutate(&mut input);
            assert!(decrypt_device_link_payload(input).is_err());
        }
    }

    #[test]
    fn device_link_rejects_oversized_ttl_and_hosted_web_target() {
        let pairing = create_device_link_pairing_key();
        let result = encrypt_device_link_payload(DeviceLinkEncryptInput {
            account_secret_hex: ACCOUNT_SECRET.to_owned(),
            pairing_public_key: pairing.public_key_hex.clone(),
            link_session_id: "link-test".to_owned(),
            target_device_id: "electron-test".to_owned(),
            server_url: "https://chat.finite.test".to_owned(),
            issued_at_unix_seconds: NOW,
            expires_at_unix_seconds: NOW + DEVICE_LINK_MAX_TTL_SECONDS + 1,
        });
        assert!(result.is_err());

        let result = encrypt_device_link_payload(DeviceLinkEncryptInput {
            account_secret_hex: ACCOUNT_SECRET.to_owned(),
            pairing_public_key: pairing.public_key_hex,
            link_session_id: "link-test".to_owned(),
            target_device_id: "hosted-web".to_owned(),
            server_url: "https://chat.finite.test".to_owned(),
            issued_at_unix_seconds: NOW,
            expires_at_unix_seconds: NOW + 60,
        });
        assert!(result.is_err());
    }
}
