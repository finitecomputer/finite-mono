//! Finite Chat MLS integration helpers.
//!
//! This crate starts with the identity boundary because the room server must
//! not become authoritative for account/device identity. OpenMLS stores our
//! signed device binding as opaque `BasicCredential` identity bytes; Finite
//! Chat clients parse and verify those bytes locally against the expected
//! Nostr account, device id, and MLS leaf signing key.

use finitechat_proto::{DeviceId, MAX_DEVICE_ID_BYTES, MAX_OBJECT_ID_BYTES};
use hkdf::Hkdf;
use openmls::prelude::{BasicCredential, Credential, CredentialWithKey};
use secp256k1::{Keypair, Message, Secp256k1, SecretKey, XOnlyPublicKey, schnorr::Signature};
use sha2::{Digest, Sha256};
use std::fmt;
use thiserror::Error;

pub const FINITE_DEVICE_CREDENTIAL_VERSION: u16 = 1;
pub const NOSTR_PUBLIC_KEY_BYTES: usize = 32;
pub const NOSTR_SECRET_KEY_BYTES: usize = 32;
pub const NOSTR_SCHNORR_SIGNATURE_BYTES: usize = 64;
pub const MAX_MLS_LEAF_SIGNING_PUBLIC_KEY_BYTES: u32 = MAX_OBJECT_ID_BYTES;

const FINITE_DEVICE_CREDENTIAL_MAGIC: &[u8] = b"finitechat.device-credential.v1";
const DEVICE_BINDING_SIGNATURE_DOMAIN: &[u8] = b"finitechat.device-binding-signature.v1";
const NOSTR_SECRET_DERIVATION_ROOT_DOMAIN: &[u8] = b"finitechat.nostr-secret-hkdf.v1";
const MAX_SECRET_DERIVATION_DOMAIN_BYTES: usize = 128;
const MAX_SECRET_DERIVATION_CONTEXT_BYTES: usize = 1024;
const U16_BYTES: usize = 2;
const U64_BYTES: usize = 8;
const UNSIGNED_FIXED_BYTES: usize = FINITE_DEVICE_CREDENTIAL_MAGIC.len()
    + U16_BYTES
    + NOSTR_PUBLIC_KEY_BYTES
    + U16_BYTES
    + U16_BYTES
    + U64_BYTES
    + U64_BYTES;
const SIGNED_FIXED_BYTES: usize = UNSIGNED_FIXED_BYTES + NOSTR_SCHNORR_SIGNATURE_BYTES;

const _: () = {
    assert!(FINITE_DEVICE_CREDENTIAL_VERSION == 1);
    assert!(NOSTR_PUBLIC_KEY_BYTES == 32);
    assert!(NOSTR_SECRET_KEY_BYTES == 32);
    assert!(NOSTR_SCHNORR_SIGNATURE_BYTES == 64);
    assert!(MAX_SECRET_DERIVATION_DOMAIN_BYTES <= u16::MAX as usize);
    assert!(MAX_SECRET_DERIVATION_CONTEXT_BYTES <= u16::MAX as usize);
    assert!(MAX_DEVICE_ID_BYTES <= u16::MAX as u32);
    assert!(MAX_MLS_LEAF_SIGNING_PUBLIC_KEY_BYTES <= u16::MAX as u32);
};

#[derive(Clone, PartialEq, Eq)]
pub struct NostrSecretKey([u8; NOSTR_SECRET_KEY_BYTES]);

impl NostrSecretKey {
    pub fn from_bytes(bytes: [u8; NOSTR_SECRET_KEY_BYTES]) -> Result<Self, MlsCredentialError> {
        let secret_key =
            SecretKey::from_slice(&bytes).map_err(|_| MlsCredentialError::InvalidNostrSecretKey)?;
        let key = Self(secret_key.secret_bytes());
        debug_assert_eq!(key.0.len(), NOSTR_SECRET_KEY_BYTES);
        Ok(key)
    }

    pub fn public_key(&self) -> NostrPublicKey {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&self.0)
            .expect("NostrSecretKey is validated before construction");
        let keypair = Keypair::from_secret_key(&secp, &secret_key);
        let (public_key, _) = XOnlyPublicKey::from_keypair(&keypair);
        let public_key = NostrPublicKey(public_key.serialize());
        debug_assert_eq!(public_key.0.len(), NOSTR_PUBLIC_KEY_BYTES);
        public_key
    }

    /// Raw secret bytes for durable storage by trusted callers (e.g. the
    /// agent CLI's 0600 nsec file). Handle with care.
    pub fn as_bytes(&self) -> &[u8; NOSTR_SECRET_KEY_BYTES] {
        &self.0
    }

    pub fn derive_secret_32(
        &self,
        domain: &[u8],
        context: &[u8],
    ) -> Result<[u8; 32], MlsCredentialError> {
        validate_secret_derivation_input(domain, context)?;
        let salt = secret_derivation_salt(domain)?;
        let hkdf = Hkdf::<Sha256>::new(Some(&salt), &self.0);
        let mut output = [0u8; 32];
        hkdf.expand(context, &mut output)
            .map_err(|_| MlsCredentialError::SecretDerivationFailed)?;
        debug_assert_eq!(output.len(), 32);
        Ok(output)
    }

    pub fn sign_schnorr_digest(&self, digest: [u8; 32]) -> [u8; NOSTR_SCHNORR_SIGNATURE_BYTES] {
        let message = Message::from_digest(digest);
        let secp = Secp256k1::new();
        secp.sign_schnorr_no_aux_rand(&message, &self.keypair())
            .serialize()
    }

    fn keypair(&self) -> Keypair {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&self.0)
            .expect("NostrSecretKey is validated before construction");
        Keypair::from_secret_key(&secp, &secret_key)
    }
}

impl fmt::Debug for NostrSecretKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NostrSecretKey(REDACTED)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NostrPublicKey([u8; NOSTR_PUBLIC_KEY_BYTES]);

impl NostrPublicKey {
    pub fn from_bytes(bytes: [u8; NOSTR_PUBLIC_KEY_BYTES]) -> Result<Self, MlsCredentialError> {
        XOnlyPublicKey::from_slice(&bytes)
            .map_err(|_| MlsCredentialError::InvalidNostrPublicKey)?;
        let public_key = Self(bytes);
        debug_assert_eq!(public_key.0.len(), NOSTR_PUBLIC_KEY_BYTES);
        Ok(public_key)
    }

    pub fn bytes(&self) -> [u8; NOSTR_PUBLIC_KEY_BYTES] {
        self.0
    }

    pub fn as_bytes(&self) -> &[u8; NOSTR_PUBLIC_KEY_BYTES] {
        &self.0
    }

    fn xonly(&self) -> XOnlyPublicKey {
        XOnlyPublicKey::from_slice(&self.0)
            .expect("NostrPublicKey is validated before construction")
    }

    pub fn verify_schnorr_digest(
        &self,
        digest: [u8; 32],
        signature: &[u8; NOSTR_SCHNORR_SIGNATURE_BYTES],
    ) -> Result<(), MlsCredentialError> {
        let signature =
            Signature::from_slice(signature).map_err(|_| MlsCredentialError::MalformedSignature)?;
        let message = Message::from_digest(digest);
        let secp = Secp256k1::verification_only();
        secp.verify_schnorr(&signature, &message, &self.xonly())
            .map_err(|_| MlsCredentialError::InvalidAccountSignature)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiniteDeviceCredentialV1 {
    account_public_key: NostrPublicKey,
    device_id: DeviceId,
    mls_leaf_signing_public_key: Vec<u8>,
    not_before_unix_seconds: u64,
    not_after_unix_seconds: u64,
    account_signature: [u8; NOSTR_SCHNORR_SIGNATURE_BYTES],
}

#[derive(Debug, Clone, Copy)]
pub struct ExpectedDeviceCredential<'a> {
    pub account_public_key: NostrPublicKey,
    pub device_id: &'a str,
    pub mls_leaf_signing_public_key: &'a [u8],
    pub now_unix_seconds: u64,
}

impl FiniteDeviceCredentialV1 {
    pub fn sign(
        account_secret_key: &NostrSecretKey,
        device_id: impl Into<DeviceId>,
        mls_leaf_signing_public_key: Vec<u8>,
        not_before_unix_seconds: u64,
        not_after_unix_seconds: u64,
    ) -> Result<Self, MlsCredentialError> {
        let device_id = device_id.into();
        validate_binding_fields(
            &device_id,
            &mls_leaf_signing_public_key,
            not_before_unix_seconds,
            not_after_unix_seconds,
        )?;

        let account_public_key = account_secret_key.public_key();
        let digest = binding_digest(
            account_public_key,
            &device_id,
            &mls_leaf_signing_public_key,
            not_before_unix_seconds,
            not_after_unix_seconds,
        )?;
        let message = Message::from_digest(digest);
        let secp = Secp256k1::new();
        let signature = secp
            .sign_schnorr_no_aux_rand(&message, &account_secret_key.keypair())
            .serialize();

        let credential = Self {
            account_public_key,
            device_id,
            mls_leaf_signing_public_key,
            not_before_unix_seconds,
            not_after_unix_seconds,
            account_signature: signature,
        };
        debug_assert!(
            credential
                .verify_signature_at(not_before_unix_seconds)
                .is_ok()
        );
        Ok(credential)
    }

    pub fn from_basic_credential(credential: &BasicCredential) -> Result<Self, MlsCredentialError> {
        Self::from_identity_bytes(credential.identity())
    }

    pub fn from_credential(credential: Credential) -> Result<Self, MlsCredentialError> {
        let basic = BasicCredential::try_from(credential)
            .map_err(|_| MlsCredentialError::WrongOpenMlsCredentialType)?;
        Self::from_basic_credential(&basic)
    }

    pub fn from_identity_bytes(bytes: &[u8]) -> Result<Self, MlsCredentialError> {
        validate_identity_size(bytes)?;
        let mut cursor = CredentialCursor::new(bytes);
        cursor.take_magic()?;
        let version = cursor.take_u16()?;
        if version != FINITE_DEVICE_CREDENTIAL_VERSION {
            return Err(MlsCredentialError::UnsupportedCredentialVersion(version));
        }

        let account_public_key = cursor.take_nostr_public_key()?;
        let device_id = cursor.take_device_id()?;
        let mls_leaf_signing_public_key = cursor.take_mls_leaf_signing_public_key()?;
        let not_before_unix_seconds = cursor.take_u64()?;
        let not_after_unix_seconds = cursor.take_u64()?;
        let account_signature = cursor.take_signature()?;
        cursor.finish()?;

        validate_binding_fields(
            &device_id,
            &mls_leaf_signing_public_key,
            not_before_unix_seconds,
            not_after_unix_seconds,
        )?;

        let credential = Self {
            account_public_key,
            device_id,
            mls_leaf_signing_public_key,
            not_before_unix_seconds,
            not_after_unix_seconds,
            account_signature,
        };
        debug_assert_eq!(credential.identity_bytes().len(), bytes.len());
        Ok(credential)
    }

    pub fn to_basic_credential(&self) -> BasicCredential {
        BasicCredential::new(self.identity_bytes())
    }

    pub fn to_openmls_credential_with_key(&self) -> CredentialWithKey {
        validate_binding_fields(
            &self.device_id,
            &self.mls_leaf_signing_public_key,
            self.not_before_unix_seconds,
            self.not_after_unix_seconds,
        )
        .expect("FiniteDeviceCredentialV1 fields are validated before construction");
        let credential_with_key = CredentialWithKey {
            credential: self.to_basic_credential().into(),
            signature_key: self.mls_leaf_signing_public_key.as_slice().into(),
        };
        debug_assert_eq!(
            credential_with_key.signature_key.as_slice(),
            self.mls_leaf_signing_public_key
        );
        credential_with_key
    }

    pub fn identity_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.identity_len());
        append_unsigned_binding(
            &mut bytes,
            self.account_public_key,
            &self.device_id,
            &self.mls_leaf_signing_public_key,
            self.not_before_unix_seconds,
            self.not_after_unix_seconds,
        )
        .expect("FiniteDeviceCredentialV1 fields are validated before construction");
        bytes.extend_from_slice(&self.account_signature);
        debug_assert_eq!(bytes.len(), self.identity_len());
        bytes
    }

    pub fn verify_signature_at(&self, now_unix_seconds: u64) -> Result<(), MlsCredentialError> {
        self.verify_signature_and_time(now_unix_seconds)
    }

    pub fn verify_expected(
        &self,
        expected: ExpectedDeviceCredential<'_>,
    ) -> Result<(), MlsCredentialError> {
        validate_binding_fields(
            &self.device_id,
            &self.mls_leaf_signing_public_key,
            self.not_before_unix_seconds,
            self.not_after_unix_seconds,
        )?;
        validate_expected_fields(&expected)?;
        if expected.account_public_key != self.account_public_key {
            return Err(MlsCredentialError::AccountPublicKeyMismatch);
        }
        if expected.device_id != self.device_id {
            return Err(MlsCredentialError::DeviceIdMismatch);
        }
        if expected.mls_leaf_signing_public_key != self.mls_leaf_signing_public_key {
            return Err(MlsCredentialError::MlsLeafSigningKeyMismatch);
        }

        self.verify_signature_and_time(expected.now_unix_seconds)?;
        debug_assert_eq!(expected.account_public_key, self.account_public_key);
        debug_assert_eq!(expected.device_id, self.device_id);
        Ok(())
    }

    fn verify_signature_and_time(&self, now_unix_seconds: u64) -> Result<(), MlsCredentialError> {
        validate_binding_fields(
            &self.device_id,
            &self.mls_leaf_signing_public_key,
            self.not_before_unix_seconds,
            self.not_after_unix_seconds,
        )?;
        if now_unix_seconds < self.not_before_unix_seconds {
            return Err(MlsCredentialError::CredentialNotYetValid);
        }
        if now_unix_seconds > self.not_after_unix_seconds {
            return Err(MlsCredentialError::CredentialExpired);
        }

        let signature = Signature::from_slice(&self.account_signature)
            .map_err(|_| MlsCredentialError::MalformedSignature)?;
        let digest = binding_digest(
            self.account_public_key,
            &self.device_id,
            &self.mls_leaf_signing_public_key,
            self.not_before_unix_seconds,
            self.not_after_unix_seconds,
        )?;
        let message = Message::from_digest(digest);
        let secp = Secp256k1::verification_only();
        secp.verify_schnorr(&signature, &message, &self.account_public_key.xonly())
            .map_err(|_| MlsCredentialError::InvalidAccountSignature)?;

        debug_assert!(now_unix_seconds >= self.not_before_unix_seconds);
        debug_assert!(now_unix_seconds <= self.not_after_unix_seconds);
        Ok(())
    }

    pub fn account_public_key(&self) -> NostrPublicKey {
        self.account_public_key
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn mls_leaf_signing_public_key(&self) -> &[u8] {
        &self.mls_leaf_signing_public_key
    }

    pub fn not_before_unix_seconds(&self) -> u64 {
        self.not_before_unix_seconds
    }

    pub fn not_after_unix_seconds(&self) -> u64 {
        self.not_after_unix_seconds
    }

    pub fn account_signature(&self) -> &[u8; NOSTR_SCHNORR_SIGNATURE_BYTES] {
        &self.account_signature
    }

    fn identity_len(&self) -> usize {
        SIGNED_FIXED_BYTES + self.device_id.len() + self.mls_leaf_signing_public_key.len()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MlsCredentialError {
    #[error("Nostr secret key is malformed or out of range")]
    InvalidNostrSecretKey,
    #[error("Nostr public key is malformed")]
    InvalidNostrPublicKey,
    #[error("device id must not be empty")]
    EmptyDeviceId,
    #[error("device id has {actual_bytes} bytes, max {max_bytes}")]
    DeviceIdTooLong { max_bytes: u32, actual_bytes: u32 },
    #[error("MLS leaf signing public key must not be empty")]
    EmptyMlsLeafSigningKey,
    #[error("MLS leaf signing public key has {actual_bytes} bytes, max {max_bytes}")]
    MlsLeafSigningKeyTooLong { max_bytes: u32, actual_bytes: u32 },
    #[error("credential validity window is invalid")]
    InvalidValidityWindow,
    #[error("credential bytes are malformed")]
    MalformedCredential,
    #[error("credential version {0} is not supported")]
    UnsupportedCredentialVersion(u16),
    #[error("credential signature bytes are malformed")]
    MalformedSignature,
    #[error("credential Nostr account signature is invalid")]
    InvalidAccountSignature,
    #[error("credential Nostr account public key does not match the expected account")]
    AccountPublicKeyMismatch,
    #[error("credential device id does not match the expected device")]
    DeviceIdMismatch,
    #[error("credential MLS leaf signing key does not match the expected leaf key")]
    MlsLeafSigningKeyMismatch,
    #[error("credential is not yet valid")]
    CredentialNotYetValid,
    #[error("credential is expired")]
    CredentialExpired,
    #[error("OpenMLS credential is not a BasicCredential")]
    WrongOpenMlsCredentialType,
    #[error("secret derivation domain must not be empty")]
    EmptySecretDerivationDomain,
    #[error("secret derivation domain has {actual_bytes} bytes, max {max_bytes}")]
    SecretDerivationDomainTooLong { max_bytes: u32, actual_bytes: u32 },
    #[error("secret derivation context has {actual_bytes} bytes, max {max_bytes}")]
    SecretDerivationContextTooLong { max_bytes: u32, actual_bytes: u32 },
    #[error("secret derivation failed")]
    SecretDerivationFailed,
}

fn validate_secret_derivation_input(
    domain: &[u8],
    context: &[u8],
) -> Result<(), MlsCredentialError> {
    if domain.is_empty() {
        return Err(MlsCredentialError::EmptySecretDerivationDomain);
    }
    if domain.len() > MAX_SECRET_DERIVATION_DOMAIN_BYTES {
        return Err(MlsCredentialError::SecretDerivationDomainTooLong {
            max_bytes: MAX_SECRET_DERIVATION_DOMAIN_BYTES as u32,
            actual_bytes: len_as_u32(domain.len()),
        });
    }
    if context.len() > MAX_SECRET_DERIVATION_CONTEXT_BYTES {
        return Err(MlsCredentialError::SecretDerivationContextTooLong {
            max_bytes: MAX_SECRET_DERIVATION_CONTEXT_BYTES as u32,
            actual_bytes: len_as_u32(context.len()),
        });
    }
    debug_assert!(!domain.is_empty());
    debug_assert!(domain.len() <= MAX_SECRET_DERIVATION_DOMAIN_BYTES);
    debug_assert!(context.len() <= MAX_SECRET_DERIVATION_CONTEXT_BYTES);
    Ok(())
}

fn secret_derivation_salt(domain: &[u8]) -> Result<Vec<u8>, MlsCredentialError> {
    validate_secret_derivation_input(domain, &[])?;
    let domain_len = u16::try_from(domain.len()).map_err(|_| {
        MlsCredentialError::SecretDerivationDomainTooLong {
            max_bytes: MAX_SECRET_DERIVATION_DOMAIN_BYTES as u32,
            actual_bytes: len_as_u32(domain.len()),
        }
    })?;
    let mut salt =
        Vec::with_capacity(NOSTR_SECRET_DERIVATION_ROOT_DOMAIN.len() + U16_BYTES + domain.len());
    salt.extend_from_slice(NOSTR_SECRET_DERIVATION_ROOT_DOMAIN);
    salt.extend_from_slice(&domain_len.to_be_bytes());
    salt.extend_from_slice(domain);
    debug_assert_eq!(
        salt.len(),
        NOSTR_SECRET_DERIVATION_ROOT_DOMAIN.len() + U16_BYTES + domain.len()
    );
    Ok(salt)
}

fn len_as_u32(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

fn validate_binding_fields(
    device_id: &str,
    mls_leaf_signing_public_key: &[u8],
    not_before_unix_seconds: u64,
    not_after_unix_seconds: u64,
) -> Result<(), MlsCredentialError> {
    let device_id_len =
        u32::try_from(device_id.len()).map_err(|_| MlsCredentialError::DeviceIdTooLong {
            max_bytes: MAX_DEVICE_ID_BYTES,
            actual_bytes: u32::MAX,
        })?;
    let mls_key_len = u32::try_from(mls_leaf_signing_public_key.len()).map_err(|_| {
        MlsCredentialError::MlsLeafSigningKeyTooLong {
            max_bytes: MAX_MLS_LEAF_SIGNING_PUBLIC_KEY_BYTES,
            actual_bytes: u32::MAX,
        }
    })?;

    if device_id_len == 0 {
        return Err(MlsCredentialError::EmptyDeviceId);
    }
    if device_id_len > MAX_DEVICE_ID_BYTES {
        return Err(MlsCredentialError::DeviceIdTooLong {
            max_bytes: MAX_DEVICE_ID_BYTES,
            actual_bytes: device_id_len,
        });
    }
    if mls_key_len == 0 {
        return Err(MlsCredentialError::EmptyMlsLeafSigningKey);
    }
    if mls_key_len > MAX_MLS_LEAF_SIGNING_PUBLIC_KEY_BYTES {
        return Err(MlsCredentialError::MlsLeafSigningKeyTooLong {
            max_bytes: MAX_MLS_LEAF_SIGNING_PUBLIC_KEY_BYTES,
            actual_bytes: mls_key_len,
        });
    }
    if not_before_unix_seconds > not_after_unix_seconds {
        return Err(MlsCredentialError::InvalidValidityWindow);
    }

    debug_assert!(device_id_len <= MAX_DEVICE_ID_BYTES);
    debug_assert!(mls_key_len <= MAX_MLS_LEAF_SIGNING_PUBLIC_KEY_BYTES);
    Ok(())
}

fn validate_expected_fields(
    expected: &ExpectedDeviceCredential<'_>,
) -> Result<(), MlsCredentialError> {
    validate_binding_fields(
        expected.device_id,
        expected.mls_leaf_signing_public_key,
        expected.now_unix_seconds,
        expected.now_unix_seconds,
    )?;
    debug_assert!(!expected.device_id.is_empty());
    debug_assert!(!expected.mls_leaf_signing_public_key.is_empty());
    Ok(())
}

fn validate_identity_size(bytes: &[u8]) -> Result<(), MlsCredentialError> {
    let max_len = SIGNED_FIXED_BYTES
        + MAX_DEVICE_ID_BYTES as usize
        + MAX_MLS_LEAF_SIGNING_PUBLIC_KEY_BYTES as usize;
    if bytes.len() < SIGNED_FIXED_BYTES {
        return Err(MlsCredentialError::MalformedCredential);
    }
    if bytes.len() > max_len {
        return Err(MlsCredentialError::MalformedCredential);
    }
    debug_assert!(SIGNED_FIXED_BYTES <= max_len);
    Ok(())
}

fn binding_digest(
    account_public_key: NostrPublicKey,
    device_id: &str,
    mls_leaf_signing_public_key: &[u8],
    not_before_unix_seconds: u64,
    not_after_unix_seconds: u64,
) -> Result<[u8; 32], MlsCredentialError> {
    let mut bytes = Vec::with_capacity(
        DEVICE_BINDING_SIGNATURE_DOMAIN.len()
            + UNSIGNED_FIXED_BYTES
            + device_id.len()
            + mls_leaf_signing_public_key.len(),
    );
    bytes.extend_from_slice(DEVICE_BINDING_SIGNATURE_DOMAIN);
    append_unsigned_binding(
        &mut bytes,
        account_public_key,
        device_id,
        mls_leaf_signing_public_key,
        not_before_unix_seconds,
        not_after_unix_seconds,
    )?;
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    debug_assert_eq!(digest.len(), 32);
    Ok(digest)
}

fn append_unsigned_binding(
    out: &mut Vec<u8>,
    account_public_key: NostrPublicKey,
    device_id: &str,
    mls_leaf_signing_public_key: &[u8],
    not_before_unix_seconds: u64,
    not_after_unix_seconds: u64,
) -> Result<(), MlsCredentialError> {
    validate_binding_fields(
        device_id,
        mls_leaf_signing_public_key,
        not_before_unix_seconds,
        not_after_unix_seconds,
    )?;
    let device_id_len =
        u16::try_from(device_id.len()).map_err(|_| MlsCredentialError::DeviceIdTooLong {
            max_bytes: MAX_DEVICE_ID_BYTES,
            actual_bytes: u32::MAX,
        })?;
    let mls_key_len = u16::try_from(mls_leaf_signing_public_key.len()).map_err(|_| {
        MlsCredentialError::MlsLeafSigningKeyTooLong {
            max_bytes: MAX_MLS_LEAF_SIGNING_PUBLIC_KEY_BYTES,
            actual_bytes: u32::MAX,
        }
    })?;

    out.extend_from_slice(FINITE_DEVICE_CREDENTIAL_MAGIC);
    out.extend_from_slice(&FINITE_DEVICE_CREDENTIAL_VERSION.to_be_bytes());
    out.extend_from_slice(account_public_key.as_bytes());
    out.extend_from_slice(&device_id_len.to_be_bytes());
    out.extend_from_slice(device_id.as_bytes());
    out.extend_from_slice(&mls_key_len.to_be_bytes());
    out.extend_from_slice(mls_leaf_signing_public_key);
    out.extend_from_slice(&not_before_unix_seconds.to_be_bytes());
    out.extend_from_slice(&not_after_unix_seconds.to_be_bytes());
    debug_assert!(out.len() >= UNSIGNED_FIXED_BYTES);
    Ok(())
}

struct CredentialCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> CredentialCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        debug_assert!(!bytes.is_empty());
        Self { bytes, offset: 0 }
    }

    fn take_magic(&mut self) -> Result<(), MlsCredentialError> {
        let magic = self.take_bytes(FINITE_DEVICE_CREDENTIAL_MAGIC.len())?;
        if magic == FINITE_DEVICE_CREDENTIAL_MAGIC {
            Ok(())
        } else {
            Err(MlsCredentialError::MalformedCredential)
        }
    }

    fn take_u16(&mut self) -> Result<u16, MlsCredentialError> {
        let bytes = self.take_bytes(U16_BYTES)?;
        Ok(u16::from_be_bytes(
            bytes
                .try_into()
                .map_err(|_| MlsCredentialError::MalformedCredential)?,
        ))
    }

    fn take_u64(&mut self) -> Result<u64, MlsCredentialError> {
        let bytes = self.take_bytes(U64_BYTES)?;
        Ok(u64::from_be_bytes(
            bytes
                .try_into()
                .map_err(|_| MlsCredentialError::MalformedCredential)?,
        ))
    }

    fn take_nostr_public_key(&mut self) -> Result<NostrPublicKey, MlsCredentialError> {
        let bytes = self.take_bytes(NOSTR_PUBLIC_KEY_BYTES)?;
        let bytes: [u8; NOSTR_PUBLIC_KEY_BYTES] = bytes
            .try_into()
            .map_err(|_| MlsCredentialError::MalformedCredential)?;
        NostrPublicKey::from_bytes(bytes)
    }

    fn take_device_id(&mut self) -> Result<DeviceId, MlsCredentialError> {
        let len = usize::from(self.take_u16()?);
        let bytes = self.take_bytes(len)?;
        let device_id = std::str::from_utf8(bytes)
            .map_err(|_| MlsCredentialError::MalformedCredential)?
            .to_string();
        debug_assert_eq!(device_id.len(), len);
        Ok(device_id)
    }

    fn take_mls_leaf_signing_public_key(&mut self) -> Result<Vec<u8>, MlsCredentialError> {
        let len = usize::from(self.take_u16()?);
        let key = self.take_bytes(len)?.to_vec();
        debug_assert_eq!(key.len(), len);
        Ok(key)
    }

    fn take_signature(
        &mut self,
    ) -> Result<[u8; NOSTR_SCHNORR_SIGNATURE_BYTES], MlsCredentialError> {
        let bytes = self.take_bytes(NOSTR_SCHNORR_SIGNATURE_BYTES)?;
        bytes
            .try_into()
            .map_err(|_| MlsCredentialError::MalformedCredential)
    }

    fn finish(&self) -> Result<(), MlsCredentialError> {
        if self.offset == self.bytes.len() {
            debug_assert_eq!(self.bytes.len() - self.offset, 0);
            Ok(())
        } else {
            Err(MlsCredentialError::MalformedCredential)
        }
    }

    fn take_bytes(&mut self, len: usize) -> Result<&'a [u8], MlsCredentialError> {
        let Some(end) = self.offset.checked_add(len) else {
            return Err(MlsCredentialError::MalformedCredential);
        };
        let Some(bytes) = self.bytes.get(self.offset..end) else {
            return Err(MlsCredentialError::MalformedCredential);
        };
        self.offset = end;
        debug_assert!(self.offset <= self.bytes.len());
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmls::prelude::tls_codec::Deserialize as _;
    use openmls::prelude::{
        Ciphersuite, GroupId, KeyPackage, KeyPackageBundle, MlsGroup, MlsGroupCreateConfig,
        MlsMessageBodyIn, MlsMessageIn, MlsMessageOut, OpenMlsProvider, ProcessedMessageContent,
        ProtocolMessage, StagedWelcome, Welcome, WelcomeError,
    };
    use openmls_basic_credential::SignatureKeyPair;
    use openmls_rust_crypto::OpenMlsRustCrypto;

    const ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [7; NOSTR_SECRET_KEY_BYTES];
    const OTHER_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [9; NOSTR_SECRET_KEY_BYTES];
    const BOB_ACCOUNT_SECRET_BYTES: [u8; NOSTR_SECRET_KEY_BYTES] = [11; NOSTR_SECRET_KEY_BYTES];
    const MLS_LEAF_KEY: &[u8] = b"openmls-leaf-signing-public-key";
    const NOW: u64 = 1_800_000_000;
    const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;

    fn account_secret() -> NostrSecretKey {
        NostrSecretKey::from_bytes(ACCOUNT_SECRET_BYTES).unwrap()
    }

    fn other_account_public_key() -> NostrPublicKey {
        NostrSecretKey::from_bytes(OTHER_ACCOUNT_SECRET_BYTES)
            .unwrap()
            .public_key()
    }

    fn signed_credential() -> FiniteDeviceCredentialV1 {
        FiniteDeviceCredentialV1::sign(
            &account_secret(),
            "phone",
            MLS_LEAF_KEY.to_vec(),
            NOW - 60,
            NOW + 60,
        )
        .unwrap()
    }

    struct TestMlsDevice {
        provider: OpenMlsRustCrypto,
        account_secret: NostrSecretKey,
        credential: FiniteDeviceCredentialV1,
        credential_with_key: CredentialWithKey,
        signer: SignatureKeyPair,
    }

    impl TestMlsDevice {
        fn new(
            account_secret_bytes: [u8; NOSTR_SECRET_KEY_BYTES],
            device_id: &str,
        ) -> TestMlsDevice {
            let provider = OpenMlsRustCrypto::default();
            let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm()).unwrap();
            signer.store(provider.storage()).unwrap();
            let account_secret = NostrSecretKey::from_bytes(account_secret_bytes).unwrap();
            let credential = FiniteDeviceCredentialV1::sign(
                &account_secret,
                device_id,
                signer.to_public_vec(),
                NOW - 60,
                NOW + 60,
            )
            .unwrap();
            credential
                .verify_expected(ExpectedDeviceCredential {
                    account_public_key: account_secret.public_key(),
                    device_id,
                    mls_leaf_signing_public_key: signer.public(),
                    now_unix_seconds: NOW,
                })
                .unwrap();
            let credential_with_key = credential.to_openmls_credential_with_key();
            assert_eq!(
                credential_with_key.signature_key.as_slice(),
                signer.public()
            );

            TestMlsDevice {
                provider,
                account_secret,
                credential,
                credential_with_key,
                signer,
            }
        }

        fn key_package_bundle(&self) -> KeyPackageBundle {
            KeyPackage::builder()
                .build(
                    CIPHERSUITE,
                    &self.provider,
                    &self.signer,
                    self.credential_with_key.clone(),
                )
                .unwrap()
        }

        fn expected<'a>(&'a self, device_id: &'a str) -> ExpectedDeviceCredential<'a> {
            ExpectedDeviceCredential {
                account_public_key: self.account_secret.public_key(),
                device_id,
                mls_leaf_signing_public_key: self.signer.public(),
                now_unix_seconds: NOW,
            }
        }
    }

    fn group_config_requires_explicit_ratchet_tree() -> MlsGroupCreateConfig {
        MlsGroupCreateConfig::builder()
            .ciphersuite(CIPHERSUITE)
            .use_ratchet_tree_extension(false)
            .build()
    }

    #[test]
    fn nostr_signed_device_credential_verifies() {
        let credential = signed_credential();

        credential.verify_expected(expected_credential()).unwrap();
    }

    #[test]
    fn nostr_secret_derivation_matches_pinned_vector() {
        // Pinned HKDF vector: the account secret now arrives via the shared
        // Finite identity file, and existing device stores keyed off the
        // same secret must keep working — the derivation math for a given
        // secret must never change.
        let derived = account_secret()
            .derive_secret_32(b"finitechat.test.local-key.v1", b"phone")
            .unwrap();
        let mut derived_hex = String::with_capacity(64);
        for byte in derived {
            derived_hex.push_str(&format!("{byte:02x}"));
        }
        assert_eq!(
            derived_hex,
            "822a09dcf2789f0a6107d94acef335792f856d3f7b8cd40f755c06c4ca05a5e4"
        );
    }

    #[test]
    fn nostr_secret_derivation_is_stable_and_domain_separated() {
        let secret = account_secret();
        let first = secret
            .derive_secret_32(b"finitechat.test.local-key.v1", b"phone")
            .unwrap();
        let repeated = secret
            .derive_secret_32(b"finitechat.test.local-key.v1", b"phone")
            .unwrap();
        let other_domain = secret
            .derive_secret_32(b"finitechat.test.other-key.v1", b"phone")
            .unwrap();
        let other_context = secret
            .derive_secret_32(b"finitechat.test.local-key.v1", b"laptop")
            .unwrap();
        let other_secret = NostrSecretKey::from_bytes(OTHER_ACCOUNT_SECRET_BYTES)
            .unwrap()
            .derive_secret_32(b"finitechat.test.local-key.v1", b"phone")
            .unwrap();

        assert_eq!(first, repeated);
        assert_ne!(first, other_domain);
        assert_ne!(first, other_context);
        assert_ne!(first, other_secret);
    }

    #[test]
    fn nostr_secret_derivation_rejects_unbounded_input() {
        let secret = account_secret();
        assert_eq!(
            secret.derive_secret_32(b"", b"phone").unwrap_err(),
            MlsCredentialError::EmptySecretDerivationDomain
        );
        assert!(matches!(
            secret.derive_secret_32(&[b'a'; MAX_SECRET_DERIVATION_DOMAIN_BYTES + 1], b"phone"),
            Err(MlsCredentialError::SecretDerivationDomainTooLong { .. })
        ));
        assert!(matches!(
            secret.derive_secret_32(
                b"finitechat.test.local-key.v1",
                &[b'a'; MAX_SECRET_DERIVATION_CONTEXT_BYTES + 1],
            ),
            Err(MlsCredentialError::SecretDerivationContextTooLong { .. })
        ));
    }

    #[test]
    fn wrong_account_key_rejects() {
        let credential = signed_credential();

        assert_eq!(
            credential
                .verify_expected(ExpectedDeviceCredential {
                    account_public_key: other_account_public_key(),
                    ..expected_credential()
                })
                .unwrap_err(),
            MlsCredentialError::AccountPublicKeyMismatch
        );
    }

    #[test]
    fn wrong_device_id_rejects() {
        let credential = signed_credential();

        assert_eq!(
            credential
                .verify_expected(ExpectedDeviceCredential {
                    device_id: "laptop",
                    ..expected_credential()
                })
                .unwrap_err(),
            MlsCredentialError::DeviceIdMismatch
        );
    }

    #[test]
    fn wrong_mls_leaf_key_rejects() {
        let credential = signed_credential();

        assert_eq!(
            credential
                .verify_expected(ExpectedDeviceCredential {
                    mls_leaf_signing_public_key: b"wrong-leaf-key",
                    ..expected_credential()
                })
                .unwrap_err(),
            MlsCredentialError::MlsLeafSigningKeyMismatch
        );
    }

    #[test]
    fn tampered_signature_payload_rejects() {
        let credential = signed_credential();
        let mut bytes = credential.identity_bytes();
        let last_key_byte = bytes.len() - NOSTR_SCHNORR_SIGNATURE_BYTES - U64_BYTES - U64_BYTES - 1;
        bytes[last_key_byte] ^= 0x01;
        let tampered = FiniteDeviceCredentialV1::from_identity_bytes(&bytes).unwrap();

        assert_eq!(
            tampered.verify_signature_at(NOW).unwrap_err(),
            MlsCredentialError::InvalidAccountSignature
        );
    }

    #[test]
    fn expired_credential_rejects() {
        let credential = signed_credential();

        assert_eq!(
            credential.verify_signature_at(NOW + 61).unwrap_err(),
            MlsCredentialError::CredentialExpired
        );
    }

    #[test]
    fn not_yet_valid_credential_rejects() {
        let credential = signed_credential();

        assert_eq!(
            credential.verify_signature_at(NOW - 61).unwrap_err(),
            MlsCredentialError::CredentialNotYetValid
        );
    }

    #[test]
    fn invalid_sizes_reject_before_signing() {
        assert_eq!(
            FiniteDeviceCredentialV1::sign(&account_secret(), "", MLS_LEAF_KEY.to_vec(), NOW, NOW)
                .unwrap_err(),
            MlsCredentialError::EmptyDeviceId
        );
        assert_eq!(
            FiniteDeviceCredentialV1::sign(&account_secret(), "phone", Vec::new(), NOW, NOW)
                .unwrap_err(),
            MlsCredentialError::EmptyMlsLeafSigningKey
        );
        assert_eq!(
            FiniteDeviceCredentialV1::sign(
                &account_secret(),
                "phone",
                vec![1; MAX_MLS_LEAF_SIGNING_PUBLIC_KEY_BYTES as usize + 1],
                NOW,
                NOW,
            )
            .unwrap_err(),
            MlsCredentialError::MlsLeafSigningKeyTooLong {
                max_bytes: MAX_MLS_LEAF_SIGNING_PUBLIC_KEY_BYTES,
                actual_bytes: MAX_MLS_LEAF_SIGNING_PUBLIC_KEY_BYTES + 1,
            }
        );
    }

    #[test]
    fn openmls_basic_credential_round_trips_finite_identity_bytes() {
        let credential = signed_credential();
        let basic = credential.to_basic_credential();
        let openmls_credential: Credential = basic.clone().into();
        let parsed = FiniteDeviceCredentialV1::from_credential(openmls_credential).unwrap();

        assert_eq!(parsed, credential);
        parsed.verify_expected(expected_credential()).unwrap();
    }

    #[test]
    fn openmls_key_package_carries_nostr_rooted_device_credential() {
        let bob = TestMlsDevice::new(BOB_ACCOUNT_SECRET_BYTES, "bob-phone");
        let bob_key_package_bundle = bob.key_package_bundle();
        let leaf_node = bob_key_package_bundle.key_package().leaf_node();
        let parsed = FiniteDeviceCredentialV1::from_credential(leaf_node.credential().clone())
            .expect("key package must carry a finite device credential");

        assert_eq!(parsed, bob.credential);
        parsed
            .verify_expected(ExpectedDeviceCredential {
                mls_leaf_signing_public_key: leaf_node.signature_key().as_slice(),
                ..bob.expected("bob-phone")
            })
            .unwrap();
    }

    #[test]
    fn openmls_welcome_adds_device_after_server_ordered_commit_merge() {
        let alice = TestMlsDevice::new(ACCOUNT_SECRET_BYTES, "alice-laptop");
        let bob = TestMlsDevice::new(BOB_ACCOUNT_SECRET_BYTES, "bob-phone");
        let bob_key_package_bundle = bob.key_package_bundle();
        let group_config = group_config_requires_explicit_ratchet_tree();
        let group_id = GroupId::from_slice(b"finite-room-openmls-proof");
        let mut alice_group = MlsGroup::new_with_group_id(
            &alice.provider,
            &alice.signer,
            &group_config,
            group_id,
            alice.credential_with_key.clone(),
        )
        .unwrap();

        assert_eq!(alice_group.epoch().as_u64(), 0);
        assert_eq!(alice_group.members().count(), 1);
        assert!(alice_group.pending_commit().is_none());

        let (_commit_message, welcome_message, _group_info) = alice_group
            .add_members(
                &alice.provider,
                &alice.signer,
                &[bob_key_package_bundle.key_package().clone()],
            )
            .unwrap();

        assert_eq!(alice_group.epoch().as_u64(), 0);
        assert_eq!(alice_group.members().count(), 1);
        assert!(alice_group.pending_commit().is_some());

        let server_log_observed = false;
        if server_log_observed {
            alice_group.merge_pending_commit(&alice.provider).unwrap();
        }
        assert_eq!(alice_group.epoch().as_u64(), 0);
        assert_eq!(alice_group.members().count(), 1);
        assert!(alice_group.pending_commit().is_some());

        let server_log_observed = true;
        if server_log_observed {
            alice_group.merge_pending_commit(&alice.provider).unwrap();
        }
        assert_eq!(alice_group.epoch().as_u64(), 1);
        assert_eq!(alice_group.members().count(), 2);
        assert!(alice_group.pending_commit().is_none());

        let welcome = welcome_from_out(welcome_message);
        let mut bob_group = StagedWelcome::new_from_welcome(
            &bob.provider,
            group_config.join_config(),
            welcome,
            Some(alice_group.export_ratchet_tree().into()),
        )
        .expect("Welcome with explicit ratchet tree should stage")
        .into_group(&bob.provider)
        .expect("staged Welcome should create Bob's group");

        assert_eq!(bob_group.epoch().as_u64(), 1);
        assert_eq!(bob_group.group_id(), alice_group.group_id());
        assert_eq!(bob_group.members().count(), 2);
        assert_verified_member(&bob_group, &alice, "alice-laptop");
        assert_verified_member(&bob_group, &bob, "bob-phone");

        let plaintext = b"finitecomputer.command.v1 payload";
        let encrypted = alice_group
            .create_message(&alice.provider, &alice.signer, plaintext)
            .unwrap();
        let processed = bob_group
            .process_message(&bob.provider, protocol_message_from_out(encrypted))
            .unwrap();

        let ProcessedMessageContent::ApplicationMessage(message) = processed.into_content() else {
            panic!("expected decrypted application message");
        };
        assert_eq!(message.into_bytes(), plaintext);
    }

    #[test]
    fn openmls_welcome_without_ratchet_tree_material_rejects() {
        let alice = TestMlsDevice::new(ACCOUNT_SECRET_BYTES, "alice-laptop");
        let bob = TestMlsDevice::new(BOB_ACCOUNT_SECRET_BYTES, "bob-phone");
        let bob_key_package_bundle = bob.key_package_bundle();
        let group_config = group_config_requires_explicit_ratchet_tree();
        let group_id = GroupId::from_slice(b"finite-room-missing-tree-proof");
        let mut alice_group = MlsGroup::new_with_group_id(
            &alice.provider,
            &alice.signer,
            &group_config,
            group_id,
            alice.credential_with_key.clone(),
        )
        .unwrap();

        let (_commit_message, welcome_message, _group_info) = alice_group
            .add_members(
                &alice.provider,
                &alice.signer,
                &[bob_key_package_bundle.key_package().clone()],
            )
            .unwrap();
        let welcome = welcome_from_out(welcome_message);
        let missing_tree_error = StagedWelcome::new_from_welcome(
            &bob.provider,
            group_config.join_config(),
            welcome,
            None,
        )
        .expect_err("Welcome activation must require ratchet tree material");

        assert!(matches!(
            missing_tree_error,
            WelcomeError::MissingRatchetTree
        ));
    }

    fn welcome_from_out(message: MlsMessageOut) -> Welcome {
        let message = mls_message_in_from_out(message);
        let MlsMessageBodyIn::Welcome(welcome) = message.extract() else {
            panic!("expected a Welcome message");
        };
        welcome
    }

    fn protocol_message_from_out(message: MlsMessageOut) -> ProtocolMessage {
        mls_message_in_from_out(message)
            .try_into_protocol_message()
            .expect("expected a protocol message")
    }

    fn mls_message_in_from_out(message: MlsMessageOut) -> MlsMessageIn {
        let bytes = message.to_bytes().expect("MlsMessageOut should serialize");
        MlsMessageIn::tls_deserialize(&mut bytes.as_slice())
            .expect("serialized MlsMessageOut should parse as MlsMessageIn")
    }

    fn assert_verified_member(group: &MlsGroup, device: &TestMlsDevice, device_id: &str) {
        let matching_members: Vec<_> = group
            .members()
            .filter(|member| {
                FiniteDeviceCredentialV1::from_credential(member.credential.clone())
                    .map(|credential| credential.device_id() == device_id)
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(matching_members.len(), 1);

        let member = &matching_members[0];
        let credential =
            FiniteDeviceCredentialV1::from_credential(member.credential.clone()).unwrap();
        credential
            .verify_expected(ExpectedDeviceCredential {
                mls_leaf_signing_public_key: &member.signature_key,
                ..device.expected(device_id)
            })
            .unwrap();
    }

    fn expected_credential<'a>() -> ExpectedDeviceCredential<'a> {
        ExpectedDeviceCredential {
            account_public_key: account_secret().public_key(),
            device_id: "phone",
            mls_leaf_signing_public_key: MLS_LEAF_KEY,
            now_unix_seconds: NOW,
        }
    }
}
