//! Transport-independent NIP-AB v1 pairing state machine.
//!
//! The wire messages and derivations follow Block's draft NIP-AB proposal.
//! Finite's WorkOS bootstrap transports the signed events through its bounded
//! one-use rendezvous. A future QR/WebSocket adapter can use the same sessions
//! without changing credential-transfer cryptography.

use std::collections::BTreeSet;

use hkdf::Hkdf;
use nostr::event::FinalizeEvent;
use nostr::nips::nip44;
use nostr::{Event, EventBuilder, Keys, Kind, PublicKey, SecretKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub const NIP_AB_VERSION: u16 = 1;
pub const NIP_AB_EVENT_KIND: u16 = 24_134;
pub const NIP_AB_SESSION_TTL_SECONDS: u64 = 120;
pub const NIP_AB_MAX_PLAINTEXT_BYTES: usize = 65_535;
pub const FINITE_PAIRING_PURPOSE_V1: &str = "finitechat.account-pairing.v1";
const NIP_AB_MIN_CONTENT_CHARS: usize = 132;
const NIP_AB_MAX_CONTENT_CHARS: usize = 87_472;
const NIP_AB_MAX_PROCESSED_EVENTS: usize = 8;

const HKDF_SESSION_ID_INFO: &[u8] = b"nostr-pair-session-id";
const HKDF_SAS_INFO: &[u8] = b"nostr-pair-sas-v1";
const HKDF_TRANSCRIPT_INFO: &[u8] = b"nostr-pair-transcript-v1";

#[derive(Debug, Error)]
pub enum NipAbError {
    #[error("pairing entropy failed")]
    Entropy,
    #[error("pairing session expired")]
    Expired,
    #[error("pairing event is invalid")]
    InvalidEvent,
    #[error("pairing event is from another peer")]
    WrongPeer,
    #[error("pairing message is out of order")]
    OutOfOrder,
    #[error("pairing session id is invalid")]
    InvalidSession,
    #[error("pairing transcript does not match")]
    TranscriptMismatch,
    #[error("pairing payload is invalid")]
    InvalidPayload,
    #[error("pairing cryptography failed")]
    Cryptography,
    #[error("pairing serialization failed")]
    Serialization,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NipAbPayloadType {
    Nsec,
    Bunker,
    Connect,
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NipAbAbortReason {
    SasMismatch,
    UserDenied,
    Timeout,
    ProtocolError,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum NipAbMessage {
    Offer {
        session_id: String,
        #[serde(default = "default_message_version")]
        version: u16,
    },
    SasConfirm {
        transcript_hash: String,
    },
    Payload {
        payload_type: NipAbPayloadType,
        payload: String,
    },
    Complete {
        success: bool,
    },
    Abort {
        reason: NipAbAbortReason,
    },
}

fn default_message_version() -> u16 {
    NIP_AB_VERSION
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct NipAbSourceCheckpointV1 {
    pub version: u16,
    pub source_secret_key_hex: String,
    pub session_secret_hex: String,
    #[zeroize(skip)]
    pub expected_target_public_key: String,
    #[zeroize(skip)]
    pub issued_at_unix_seconds: u64,
    #[zeroize(skip)]
    pub expires_at_unix_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NipAbSourceDescriptorV1 {
    pub version: u16,
    pub source_public_key: String,
    pub session_secret_hex: String,
    pub expires_at_unix_seconds: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct FinitePairingPayloadV1 {
    #[zeroize(skip)]
    pub version: u16,
    pub purpose: String,
    #[zeroize(skip)]
    pub pairing_session_id: String,
    pub account_secret_hex: String,
    #[zeroize(skip)]
    pub account_id: String,
    #[zeroize(skip)]
    pub target_device_id: String,
    #[zeroize(skip)]
    pub enrollment_user_id: String,
    pub enrollment_capability_hex: String,
    #[zeroize(skip)]
    pub server_url: String,
    #[zeroize(skip)]
    pub issued_at_unix_seconds: u64,
    #[zeroize(skip)]
    pub expires_at_unix_seconds: u64,
}

impl FinitePairingPayloadV1 {
    pub fn validate(
        &self,
        expected_pairing_session_id: &str,
        expected_target_device_id: &str,
        expected_server_url: &str,
        now_unix_seconds: u64,
    ) -> Result<(), NipAbError> {
        if self.version != NIP_AB_VERSION
            || self.purpose != FINITE_PAIRING_PURPOSE_V1
            || self.pairing_session_id != expected_pairing_session_id
            || self.target_device_id != expected_target_device_id
            || self.enrollment_user_id.is_empty()
            || self.enrollment_user_id.len() > 512
            || self.enrollment_user_id.trim() != self.enrollment_user_id
            || self.enrollment_user_id.chars().any(char::is_control)
            || parse_hex_32(&self.enrollment_capability_hex).is_err()
            || self.server_url.trim_end_matches('/') != expected_server_url.trim_end_matches('/')
            || self.issued_at_unix_seconds > now_unix_seconds
            || self.expires_at_unix_seconds < now_unix_seconds
            || self
                .expires_at_unix_seconds
                .saturating_sub(self.issued_at_unix_seconds)
                > NIP_AB_SESSION_TTL_SECONDS
        {
            return Err(NipAbError::InvalidPayload);
        }
        let secret =
            parse_secret_key(&self.account_secret_hex).map_err(|_| NipAbError::InvalidPayload)?;
        if Keys::new(secret).public_key().to_hex() != self.account_id {
            return Err(NipAbError::InvalidPayload);
        }
        Ok(())
    }
}

pub struct NipAbTargetBootstrap {
    keys: Keys,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceState {
    WaitingForOffer,
    OfferAccepted,
    Confirmed,
    PayloadSent,
    Completed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetState {
    OfferSent,
    AwaitingTargetConfirmation,
    SourceConfirmed,
    PayloadReceived,
    Completed,
}

pub struct NipAbSourceSession {
    keys: Keys,
    session_secret: [u8; 32],
    session_id: [u8; 32],
    expected_target_public_key: PublicKey,
    peer_public_key: Option<PublicKey>,
    sas_input: Option<[u8; 32]>,
    state: SourceState,
    processed_event_ids: BTreeSet<[u8; 32]>,
    issued_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
}

pub struct NipAbTargetSession {
    keys: Keys,
    session_secret: [u8; 32],
    session_id: [u8; 32],
    source_public_key: PublicKey,
    sas_input: [u8; 32],
    state: TargetState,
    processed_event_ids: BTreeSet<[u8; 32]>,
    expires_at_unix_seconds: u64,
}

impl NipAbSourceSession {
    pub fn create(
        expected_target_public_key: String,
        now_unix_seconds: u64,
    ) -> Result<(Self, NipAbSourceDescriptorV1), NipAbError> {
        let expected_target_public_key = PublicKey::from_hex(&expected_target_public_key)
            .map_err(|_| NipAbError::InvalidSession)?;
        let keys = Keys::generate();
        let mut session_secret = [0_u8; 32];
        getrandom::fill(&mut session_secret).map_err(|_| NipAbError::Entropy)?;
        if session_secret == [0_u8; 32] {
            return Err(NipAbError::Entropy);
        }
        let expires_at_unix_seconds = now_unix_seconds
            .checked_add(NIP_AB_SESSION_TTL_SECONDS)
            .ok_or(NipAbError::Expired)?;
        let checkpoint = NipAbSourceCheckpointV1 {
            version: NIP_AB_VERSION,
            source_secret_key_hex: keys.secret_key().to_secret_hex(),
            session_secret_hex: hex::encode(session_secret),
            expected_target_public_key: expected_target_public_key.to_hex(),
            issued_at_unix_seconds: now_unix_seconds,
            expires_at_unix_seconds,
        };
        let session = Self::restore(&checkpoint, now_unix_seconds)?;
        let descriptor = session.descriptor();
        Ok((session, descriptor))
    }

    pub fn restore(
        checkpoint: &NipAbSourceCheckpointV1,
        now_unix_seconds: u64,
    ) -> Result<Self, NipAbError> {
        if checkpoint.version != NIP_AB_VERSION
            || checkpoint.expires_at_unix_seconds <= checkpoint.issued_at_unix_seconds
            || checkpoint
                .expires_at_unix_seconds
                .saturating_sub(checkpoint.issued_at_unix_seconds)
                > NIP_AB_SESSION_TTL_SECONDS
            || now_unix_seconds > checkpoint.expires_at_unix_seconds
        {
            return Err(NipAbError::Expired);
        }
        let source_secret = parse_secret_key(&checkpoint.source_secret_key_hex)?;
        let expected_target_public_key =
            PublicKey::from_hex(&checkpoint.expected_target_public_key)
                .map_err(|_| NipAbError::InvalidSession)?;
        let session_secret = parse_hex_32(&checkpoint.session_secret_hex)?;
        if session_secret == [0_u8; 32] {
            return Err(NipAbError::InvalidSession);
        }
        Ok(Self {
            keys: Keys::new(source_secret),
            session_secret,
            session_id: derive_session_id(&session_secret)?,
            expected_target_public_key,
            peer_public_key: None,
            sas_input: None,
            state: SourceState::WaitingForOffer,
            processed_event_ids: BTreeSet::new(),
            issued_at_unix_seconds: checkpoint.issued_at_unix_seconds,
            expires_at_unix_seconds: checkpoint.expires_at_unix_seconds,
        })
    }

    pub fn checkpoint(&self) -> NipAbSourceCheckpointV1 {
        NipAbSourceCheckpointV1 {
            version: NIP_AB_VERSION,
            source_secret_key_hex: self.keys.secret_key().to_secret_hex(),
            session_secret_hex: hex::encode(self.session_secret),
            expected_target_public_key: self.expected_target_public_key.to_hex(),
            issued_at_unix_seconds: self.issued_at_unix_seconds,
            expires_at_unix_seconds: self.expires_at_unix_seconds,
        }
    }

    pub fn descriptor(&self) -> NipAbSourceDescriptorV1 {
        NipAbSourceDescriptorV1 {
            version: NIP_AB_VERSION,
            source_public_key: self.keys.public_key().to_hex(),
            session_secret_hex: hex::encode(self.session_secret),
            expires_at_unix_seconds: self.expires_at_unix_seconds,
        }
    }

    pub fn accept_offer(
        &mut self,
        event: &Event,
        now_unix_seconds: u64,
    ) -> Result<String, NipAbError> {
        self.require_source_state(SourceState::WaitingForOffer)?;
        self.check_time(now_unix_seconds)?;
        self.validate_event(
            event,
            Some(self.expected_target_public_key),
            now_unix_seconds,
        )?;
        let message = self.decrypt_message(event)?;
        let NipAbMessage::Offer {
            session_id,
            version,
        } = message
        else {
            return Err(NipAbError::OutOfOrder);
        };
        if version != NIP_AB_VERSION {
            return Err(NipAbError::InvalidSession);
        }
        let received_session_id = parse_hex_32(&session_id)?;
        if !constant_time_eq(&received_session_id, &self.session_id) {
            return Err(NipAbError::InvalidSession);
        }
        let peer_public_key = event.pubkey;
        if peer_public_key != self.expected_target_public_key {
            return Err(NipAbError::WrongPeer);
        }
        let mut shared = nostr::util::generate_shared_key(self.keys.secret_key(), &peer_public_key)
            .map_err(|_| NipAbError::Cryptography)?;
        let (sas_code, sas_input) = derive_sas(&shared, &self.session_secret)?;
        shared.zeroize();
        self.peer_public_key = Some(peer_public_key);
        self.sas_input = Some(sas_input);
        self.state = SourceState::OfferAccepted;
        self.record_event(event)?;
        Ok(format_sas(sas_code))
    }

    pub fn confirm_sas(&mut self, now_unix_seconds: u64) -> Result<Event, NipAbError> {
        self.require_source_state(SourceState::OfferAccepted)?;
        self.check_time(now_unix_seconds)?;
        let peer = self.peer_public_key.ok_or(NipAbError::WrongPeer)?;
        let sas_input = self.sas_input.ok_or(NipAbError::TranscriptMismatch)?;
        let transcript_hash = derive_transcript_hash(
            &self.session_id,
            &self.keys.public_key().to_bytes(),
            &peer.to_bytes(),
            &sas_input,
            &self.session_secret,
        )?;
        let event = self.build_event(
            &NipAbMessage::SasConfirm {
                transcript_hash: hex::encode(transcript_hash),
            },
            peer,
            now_unix_seconds,
        )?;
        self.state = SourceState::Confirmed;
        Ok(event)
    }

    pub fn send_payload(
        &mut self,
        payload_type: NipAbPayloadType,
        payload: Zeroizing<String>,
        now_unix_seconds: u64,
    ) -> Result<Event, NipAbError> {
        self.require_source_state(SourceState::Confirmed)?;
        self.check_time(now_unix_seconds)?;
        let peer = self.peer_public_key.ok_or(NipAbError::WrongPeer)?;
        if payload.is_empty() || payload.len() > NIP_AB_MAX_PLAINTEXT_BYTES {
            return Err(NipAbError::InvalidPayload);
        }
        let mut message = NipAbMessage::Payload {
            payload_type,
            payload: payload.to_string(),
        };
        let result = self.build_event(&message, peer, now_unix_seconds);
        if let NipAbMessage::Payload { payload, .. } = &mut message {
            payload.zeroize();
        }
        let event = result?;
        self.state = SourceState::PayloadSent;
        Ok(event)
    }

    pub fn accept_complete(
        &mut self,
        event: &Event,
        now_unix_seconds: u64,
    ) -> Result<(), NipAbError> {
        self.require_source_state(SourceState::PayloadSent)?;
        self.check_time(now_unix_seconds)?;
        self.validate_event(event, self.peer_public_key, now_unix_seconds)?;
        match self.decrypt_message(event)? {
            NipAbMessage::Complete { success: true } => {
                self.state = SourceState::Completed;
                self.record_event(event)
            }
            _ => Err(NipAbError::OutOfOrder),
        }
    }

    pub fn accept_published_response(
        &mut self,
        source_confirmation_event: &Event,
        payload_event: &Event,
        expected_payload_type: NipAbPayloadType,
        expected_payload: &str,
        now_unix_seconds: u64,
    ) -> Result<(), NipAbError> {
        self.require_source_state(SourceState::OfferAccepted)?;
        self.check_time(now_unix_seconds)?;
        let peer = self.peer_public_key.ok_or(NipAbError::WrongPeer)?;
        self.validate_own_published_event(source_confirmation_event, peer, now_unix_seconds)?;
        let sas_input = self.sas_input.ok_or(NipAbError::TranscriptMismatch)?;
        let expected_transcript = derive_transcript_hash(
            &self.session_id,
            &self.keys.public_key().to_bytes(),
            &peer.to_bytes(),
            &sas_input,
            &self.session_secret,
        )?;
        match self.decrypt_outbound_message(source_confirmation_event, peer)? {
            NipAbMessage::SasConfirm { transcript_hash }
                if parse_hex_32(&transcript_hash)
                    .is_ok_and(|value| constant_time_eq(&value, &expected_transcript)) => {}
            _ => return Err(NipAbError::TranscriptMismatch),
        }
        self.state = SourceState::Confirmed;

        self.validate_own_published_event(payload_event, peer, now_unix_seconds)?;
        match self.decrypt_outbound_message(payload_event, peer)? {
            NipAbMessage::Payload {
                payload_type,
                mut payload,
            } if payload_type == expected_payload_type && payload == expected_payload => {
                payload.zeroize();
            }
            NipAbMessage::Payload { mut payload, .. } => {
                payload.zeroize();
                return Err(NipAbError::InvalidPayload);
            }
            _ => return Err(NipAbError::OutOfOrder),
        }
        self.state = SourceState::PayloadSent;
        Ok(())
    }

    fn validate_own_published_event(
        &self,
        event: &Event,
        peer: PublicKey,
        now_unix_seconds: u64,
    ) -> Result<(), NipAbError> {
        event.verify().map_err(|_| NipAbError::InvalidEvent)?;
        if event.pubkey != self.keys.public_key()
            || event.kind != Kind::Custom(NIP_AB_EVENT_KIND)
            || event.created_at.as_secs().abs_diff(now_unix_seconds) > NIP_AB_SESSION_TTL_SECONDS
        {
            return Err(NipAbError::InvalidEvent);
        }
        let tags: Vec<_> = event.tags.iter().collect();
        if tags.len() != 1 {
            return Err(NipAbError::InvalidEvent);
        }
        let values = tags[0].as_slice();
        if values.len() != 2 || values[0].as_str() != "p" || values[1].as_str() != peer.to_hex() {
            return Err(NipAbError::InvalidEvent);
        }
        Ok(())
    }

    fn decrypt_outbound_message(
        &self,
        event: &Event,
        peer: PublicKey,
    ) -> Result<NipAbMessage, NipAbError> {
        let mut plaintext = nip44::decrypt(self.keys.secret_key(), &peer, event.content.as_str())
            .map_err(|_| NipAbError::Cryptography)?;
        if plaintext.len() > NIP_AB_MAX_PLAINTEXT_BYTES {
            plaintext.zeroize();
            return Err(NipAbError::InvalidPayload);
        }
        let result = serde_json::from_str(&plaintext).map_err(|_| NipAbError::Serialization);
        plaintext.zeroize();
        result
    }

    fn require_source_state(&self, expected: SourceState) -> Result<(), NipAbError> {
        if self.state == expected {
            Ok(())
        } else {
            Err(NipAbError::OutOfOrder)
        }
    }
}

impl NipAbTargetSession {
    pub fn prepare() -> NipAbTargetBootstrap {
        NipAbTargetBootstrap {
            keys: Keys::generate(),
        }
    }

    pub fn create(
        bootstrap: NipAbTargetBootstrap,
        descriptor: &NipAbSourceDescriptorV1,
        now_unix_seconds: u64,
    ) -> Result<(Self, Event), NipAbError> {
        if descriptor.version != NIP_AB_VERSION
            || now_unix_seconds > descriptor.expires_at_unix_seconds
            || descriptor
                .expires_at_unix_seconds
                .saturating_sub(now_unix_seconds)
                > NIP_AB_SESSION_TTL_SECONDS
        {
            return Err(NipAbError::Expired);
        }
        let source_public_key = PublicKey::from_hex(&descriptor.source_public_key)
            .map_err(|_| NipAbError::InvalidSession)?;
        let session_secret = parse_hex_32(&descriptor.session_secret_hex)?;
        if session_secret == [0_u8; 32] {
            return Err(NipAbError::InvalidSession);
        }
        let keys = bootstrap.keys;
        let mut shared = nostr::util::generate_shared_key(keys.secret_key(), &source_public_key)
            .map_err(|_| NipAbError::Cryptography)?;
        let (_, sas_input) = derive_sas(&shared, &session_secret)?;
        shared.zeroize();
        let session_id = derive_session_id(&session_secret)?;
        let session = Self {
            keys,
            session_secret,
            session_id,
            source_public_key,
            sas_input,
            state: TargetState::OfferSent,
            processed_event_ids: BTreeSet::new(),
            expires_at_unix_seconds: descriptor.expires_at_unix_seconds,
        };
        let offer = session.build_event(
            &NipAbMessage::Offer {
                session_id: hex::encode(session_id),
                version: NIP_AB_VERSION,
            },
            source_public_key,
            now_unix_seconds,
        )?;
        debug_assert_eq!(session.state, TargetState::OfferSent);
        Ok((session, offer))
    }

    pub fn accept_source_confirmation(
        &mut self,
        event: &Event,
        now_unix_seconds: u64,
    ) -> Result<String, NipAbError> {
        self.require_target_state(TargetState::OfferSent)?;
        self.check_time(now_unix_seconds)?;
        self.validate_event(event, Some(self.source_public_key), now_unix_seconds)?;
        let NipAbMessage::SasConfirm { transcript_hash } = self.decrypt_message(event)? else {
            return Err(NipAbError::OutOfOrder);
        };
        let expected = derive_transcript_hash(
            &self.session_id,
            &self.source_public_key.to_bytes(),
            &self.keys.public_key().to_bytes(),
            &self.sas_input,
            &self.session_secret,
        )?;
        let received = parse_hex_32(&transcript_hash)?;
        if !constant_time_eq(&received, &expected) {
            return Err(NipAbError::TranscriptMismatch);
        }
        self.state = TargetState::AwaitingTargetConfirmation;
        self.record_event(event)?;
        let (sas_code, _) = derive_sas_from_input(&self.sas_input);
        Ok(format_sas(sas_code))
    }

    pub fn confirm_sas(&mut self, now_unix_seconds: u64) -> Result<(), NipAbError> {
        self.require_target_state(TargetState::AwaitingTargetConfirmation)?;
        self.check_time(now_unix_seconds)?;
        self.state = TargetState::SourceConfirmed;
        Ok(())
    }

    pub fn accept_payload(
        &mut self,
        event: &Event,
        now_unix_seconds: u64,
    ) -> Result<(NipAbPayloadType, Zeroizing<String>), NipAbError> {
        self.require_target_state(TargetState::SourceConfirmed)?;
        self.check_time(now_unix_seconds)?;
        self.validate_event(event, Some(self.source_public_key), now_unix_seconds)?;
        match self.decrypt_message(event)? {
            NipAbMessage::Payload {
                payload_type,
                payload,
            } if !payload.is_empty() && payload.len() <= NIP_AB_MAX_PLAINTEXT_BYTES => {
                self.state = TargetState::PayloadReceived;
                self.record_event(event)?;
                Ok((payload_type, Zeroizing::new(payload)))
            }
            _ => Err(NipAbError::InvalidPayload),
        }
    }

    pub fn complete(&mut self, now_unix_seconds: u64) -> Result<Event, NipAbError> {
        self.require_target_state(TargetState::PayloadReceived)?;
        self.check_time(now_unix_seconds)?;
        let event = self.build_event(
            &NipAbMessage::Complete { success: true },
            self.source_public_key,
            now_unix_seconds,
        )?;
        self.state = TargetState::Completed;
        Ok(event)
    }

    pub fn target_public_key(&self) -> String {
        self.keys.public_key().to_hex()
    }

    fn require_target_state(&self, expected: TargetState) -> Result<(), NipAbError> {
        if self.state == expected {
            Ok(())
        } else {
            Err(NipAbError::OutOfOrder)
        }
    }
}

impl NipAbTargetBootstrap {
    pub fn public_key(&self) -> String {
        self.keys.public_key().to_hex()
    }
}

trait NipAbSession {
    fn keys(&self) -> &Keys;
    fn processed_event_ids(&self) -> &BTreeSet<[u8; 32]>;
    fn processed_event_ids_mut(&mut self) -> &mut BTreeSet<[u8; 32]>;
    fn expires_at_unix_seconds(&self) -> u64;

    fn check_time(&self, now_unix_seconds: u64) -> Result<(), NipAbError> {
        if now_unix_seconds <= self.expires_at_unix_seconds() {
            Ok(())
        } else {
            Err(NipAbError::Expired)
        }
    }

    fn build_event(
        &self,
        message: &NipAbMessage,
        peer: PublicKey,
        now_unix_seconds: u64,
    ) -> Result<Event, NipAbError> {
        let mut plaintext =
            serde_json::to_string(message).map_err(|_| NipAbError::Serialization)?;
        if plaintext.len() > NIP_AB_MAX_PLAINTEXT_BYTES {
            plaintext.zeroize();
            return Err(NipAbError::InvalidPayload);
        }
        let encrypted = nip44::encrypt(
            self.keys().secret_key(),
            &peer,
            &plaintext,
            nip44::Version::V2,
        )
        .map_err(|_| NipAbError::Cryptography);
        plaintext.zeroize();
        let encrypted = encrypted?;
        EventBuilder::new(Kind::Custom(NIP_AB_EVENT_KIND), encrypted)
            .tags([Tag::public_key(peer)])
            .custom_created_at(Timestamp::from_secs(now_unix_seconds))
            .finalize(self.keys())
            .map_err(|_| NipAbError::Cryptography)
    }

    fn decrypt_message(&self, event: &Event) -> Result<NipAbMessage, NipAbError> {
        if !(NIP_AB_MIN_CONTENT_CHARS..=NIP_AB_MAX_CONTENT_CHARS).contains(&event.content.len()) {
            return Err(NipAbError::InvalidEvent);
        }
        let mut plaintext = nip44::decrypt(
            self.keys().secret_key(),
            &event.pubkey,
            event.content.as_str(),
        )
        .map_err(|_| NipAbError::Cryptography)?;
        if plaintext.len() > NIP_AB_MAX_PLAINTEXT_BYTES {
            plaintext.zeroize();
            return Err(NipAbError::InvalidPayload);
        }
        let result = serde_json::from_str(&plaintext).map_err(|_| NipAbError::Serialization);
        plaintext.zeroize();
        result
    }

    fn validate_event(
        &self,
        event: &Event,
        expected_peer: Option<PublicKey>,
        now_unix_seconds: u64,
    ) -> Result<(), NipAbError> {
        event.verify().map_err(|_| NipAbError::InvalidEvent)?;
        if event.kind != Kind::Custom(NIP_AB_EVENT_KIND) {
            return Err(NipAbError::InvalidEvent);
        }
        if let Some(expected_peer) = expected_peer
            && event.pubkey != expected_peer
        {
            return Err(NipAbError::WrongPeer);
        }
        let our_public_key = self.keys().public_key().to_hex();
        let mut matching_tags = 0_u8;
        for tag in event.tags.iter() {
            let values = tag.as_slice();
            if values.first().map(|value| value.as_str()) == Some("p") {
                if values.len() != 2 || values[1].as_str() != our_public_key {
                    return Err(NipAbError::InvalidEvent);
                }
                matching_tags = matching_tags.saturating_add(1);
            } else {
                return Err(NipAbError::InvalidEvent);
            }
        }
        if matching_tags != 1 {
            return Err(NipAbError::InvalidEvent);
        }
        let created_at = event.created_at.as_secs();
        if created_at.abs_diff(now_unix_seconds) > NIP_AB_SESSION_TTL_SECONDS {
            return Err(NipAbError::InvalidEvent);
        }
        if self.processed_event_ids().contains(&event.id.to_bytes()) {
            return Err(NipAbError::OutOfOrder);
        }
        Ok(())
    }

    fn record_event(&mut self, event: &Event) -> Result<(), NipAbError> {
        if self.processed_event_ids().len() >= NIP_AB_MAX_PROCESSED_EVENTS {
            return Err(NipAbError::OutOfOrder);
        }
        let inserted = self.processed_event_ids_mut().insert(event.id.to_bytes());
        if inserted {
            Ok(())
        } else {
            Err(NipAbError::OutOfOrder)
        }
    }
}

impl NipAbSession for NipAbSourceSession {
    fn keys(&self) -> &Keys {
        &self.keys
    }

    fn processed_event_ids(&self) -> &BTreeSet<[u8; 32]> {
        &self.processed_event_ids
    }

    fn processed_event_ids_mut(&mut self) -> &mut BTreeSet<[u8; 32]> {
        &mut self.processed_event_ids
    }

    fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }
}

impl NipAbSession for NipAbTargetSession {
    fn keys(&self) -> &Keys {
        &self.keys
    }

    fn processed_event_ids(&self) -> &BTreeSet<[u8; 32]> {
        &self.processed_event_ids
    }

    fn processed_event_ids_mut(&mut self) -> &mut BTreeSet<[u8; 32]> {
        &mut self.processed_event_ids
    }

    fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }
}

impl Drop for NipAbSourceSession {
    fn drop(&mut self) {
        self.session_secret.zeroize();
        self.session_id.zeroize();
        if let Some(sas_input) = &mut self.sas_input {
            sas_input.zeroize();
        }
    }
}

impl Drop for NipAbTargetSession {
    fn drop(&mut self) {
        self.session_secret.zeroize();
        self.session_id.zeroize();
        self.sas_input.zeroize();
    }
}

fn derive_session_id(session_secret: &[u8; 32]) -> Result<[u8; 32], NipAbError> {
    hkdf32(&[], session_secret, HKDF_SESSION_ID_INFO)
}

fn derive_sas(
    shared_secret: &[u8; 32],
    session_secret: &[u8; 32],
) -> Result<(u32, [u8; 32]), NipAbError> {
    let input = hkdf32(session_secret, shared_secret, HKDF_SAS_INFO)?;
    Ok(derive_sas_from_input(&input))
}

fn derive_sas_from_input(input: &[u8; 32]) -> (u32, [u8; 32]) {
    let code = u32::from_be_bytes([input[0], input[1], input[2], input[3]]) % 1_000_000;
    (code, *input)
}

fn derive_transcript_hash(
    session_id: &[u8; 32],
    source_public_key: &[u8; 32],
    target_public_key: &[u8; 32],
    sas_input: &[u8; 32],
    session_secret: &[u8; 32],
) -> Result<[u8; 32], NipAbError> {
    let mut transcript = [0_u8; 128];
    transcript[0..32].copy_from_slice(session_id);
    transcript[32..64].copy_from_slice(source_public_key);
    transcript[64..96].copy_from_slice(target_public_key);
    transcript[96..128].copy_from_slice(sas_input);
    let result = hkdf32(session_secret, &transcript, HKDF_TRANSCRIPT_INFO);
    transcript.zeroize();
    result
}

fn hkdf32(salt: &[u8], input: &[u8], info: &[u8]) -> Result<[u8; 32], NipAbError> {
    let hkdf = Hkdf::<Sha256>::new(Some(salt), input);
    let mut output = [0_u8; 32];
    hkdf.expand(info, &mut output)
        .map_err(|_| NipAbError::Cryptography)?;
    Ok(output)
}

fn parse_hex_32(value: &str) -> Result<[u8; 32], NipAbError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(NipAbError::InvalidSession);
    }
    let bytes = hex::decode(value).map_err(|_| NipAbError::InvalidSession)?;
    bytes.try_into().map_err(|_| NipAbError::InvalidSession)
}

fn parse_secret_key(value: &str) -> Result<SecretKey, NipAbError> {
    let bytes = parse_hex_32(value)?;
    SecretKey::from_slice(&bytes).map_err(|_| NipAbError::InvalidSession)
}

fn constant_time_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    let mut difference = 0_u8;
    for index in 0..32 {
        difference |= left[index] ^ right[index];
    }
    difference == 0
}

fn format_sas(code: u32) -> String {
    format!("{code:06}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    #[test]
    fn buzz_derivation_vectors_match() {
        let session_secret =
            parse_hex_32("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2")
                .unwrap();
        let source_secret = SecretKey::from_slice(
            &parse_hex_32("7f4c11a9c9d1e3b5a7f2e4d6c8b0a2f4e6d8c0b2a4f6e8d0c2b4a6f8e0d2c4b5")
                .unwrap(),
        )
        .unwrap();
        let target_secret = SecretKey::from_slice(
            &parse_hex_32("3a5b7c9d1e3f5a7b9c1d3e5f7a9b1c3d5e7f9a1b3c5d7e9f1a3b5c7d9e1f3a5b")
                .unwrap(),
        )
        .unwrap();
        let source = Keys::new(source_secret);
        let target = Keys::new(target_secret);
        let shared =
            nostr::util::generate_shared_key(source.secret_key(), &target.public_key()).unwrap();
        assert_eq!(
            hex::encode(derive_session_id(&session_secret).unwrap()),
            "fb357d0f8e8d5a5ba3b2a91cb18c119e1567b07ffa38cdebb73e68df78f5a380"
        );
        let (sas, sas_input) = derive_sas(&shared, &session_secret).unwrap();
        assert_eq!(format_sas(sas), "863346");
        assert_eq!(
            hex::encode(
                derive_transcript_hash(
                    &derive_session_id(&session_secret).unwrap(),
                    &source.public_key().to_bytes(),
                    &target.public_key().to_bytes(),
                    &sas_input,
                    &session_secret,
                )
                .unwrap()
            ),
            "d662818ff8911fc60a2d025f8b8b4756107104e85888dd202d28db5ca2cf28d3"
        );
    }

    #[test]
    fn workos_trusted_pairing_round_trip_requires_transcript_and_storage_completion() {
        let target_bootstrap = NipAbTargetSession::prepare();
        let target_public_key = target_bootstrap.public_key();
        let (mut source, descriptor) = NipAbSourceSession::create(target_public_key, NOW).unwrap();
        let checkpoint = source.checkpoint();
        let (mut target, offer) =
            NipAbTargetSession::create(target_bootstrap, &descriptor, NOW + 1).unwrap();

        let source_sas = source.accept_offer(&offer, NOW + 1).unwrap();
        let confirmation = source.confirm_sas(NOW + 1).unwrap();
        let payload = FinitePairingPayloadV1 {
            version: 1,
            purpose: FINITE_PAIRING_PURPOSE_V1.to_owned(),
            pairing_session_id: "pair-test".to_owned(),
            account_secret_hex: "11".repeat(32),
            account_id: Keys::new(parse_secret_key(&"11".repeat(32)).unwrap())
                .public_key()
                .to_hex(),
            target_device_id: "ios-test".to_owned(),
            enrollment_user_id: "user_test".to_owned(),
            enrollment_capability_hex: "ab".repeat(32),
            server_url: "https://chat.finite.test".to_owned(),
            issued_at_unix_seconds: NOW,
            expires_at_unix_seconds: NOW + NIP_AB_SESSION_TTL_SECONDS,
        };
        let encoded = Zeroizing::new(serde_json::to_string(&payload).unwrap());
        let payload_event = source
            .send_payload(NipAbPayloadType::Custom, encoded, NOW + 1)
            .unwrap();

        let target_sas = target
            .accept_source_confirmation(&confirmation, NOW + 2)
            .unwrap();
        assert_eq!(source_sas, target_sas);
        assert!(matches!(
            target.accept_payload(&payload_event, NOW + 2),
            Err(NipAbError::OutOfOrder)
        ));
        target.confirm_sas(NOW + 2).unwrap();
        let (payload_type, received) = target.accept_payload(&payload_event, NOW + 2).unwrap();
        assert_eq!(payload_type, NipAbPayloadType::Custom);
        let decoded: FinitePairingPayloadV1 = serde_json::from_str(&received).unwrap();
        assert_eq!(decoded.target_device_id, "ios-test");

        let complete = target.complete(NOW + 2).unwrap();
        let mut restarted = NipAbSourceSession::restore(&checkpoint, NOW + 2).unwrap();
        restarted.accept_offer(&offer, NOW + 2).unwrap();
        restarted
            .accept_published_response(
                &confirmation,
                &payload_event,
                NipAbPayloadType::Custom,
                &serde_json::to_string(&payload).unwrap(),
                NOW + 2,
            )
            .unwrap();
        restarted.accept_complete(&complete, NOW + 2).unwrap();
    }

    #[test]
    fn wrong_transcript_replay_order_and_expiry_fail_closed() {
        let target_bootstrap = NipAbTargetSession::prepare();
        let target_public_key = target_bootstrap.public_key();
        let (mut source, descriptor) = NipAbSourceSession::create(target_public_key, NOW).unwrap();
        let (mut target, offer) =
            NipAbTargetSession::create(target_bootstrap, &descriptor, NOW).unwrap();
        source.accept_offer(&offer, NOW).unwrap();
        assert!(matches!(
            source.accept_offer(&offer, NOW),
            Err(NipAbError::OutOfOrder)
        ));

        let wrong_confirmation = source
            .build_event(
                &NipAbMessage::SasConfirm {
                    transcript_hash: "00".repeat(32),
                },
                source.peer_public_key.unwrap(),
                NOW,
            )
            .unwrap();
        assert!(matches!(
            target.accept_source_confirmation(&wrong_confirmation, NOW),
            Err(NipAbError::TranscriptMismatch)
        ));
        assert!(matches!(
            NipAbTargetSession::create(
                NipAbTargetSession::prepare(),
                &descriptor,
                NOW + NIP_AB_SESSION_TTL_SECONDS + 1
            ),
            Err(NipAbError::Expired)
        ));
    }

    #[test]
    fn workos_target_binding_rejects_a_valid_offer_from_an_attacker() {
        let real_target = NipAbTargetSession::prepare();
        let (mut source, descriptor) =
            NipAbSourceSession::create(real_target.public_key(), NOW).unwrap();
        let attacker = NipAbTargetSession::prepare();
        let (_, attacker_offer) = NipAbTargetSession::create(attacker, &descriptor, NOW).unwrap();
        assert!(matches!(
            source.accept_offer(&attacker_offer, NOW),
            Err(NipAbError::WrongPeer)
        ));
    }

    #[test]
    fn finite_payload_route_account_and_expiry_substitutions_fail_closed() {
        let account_secret = "11".repeat(32);
        let payload = FinitePairingPayloadV1 {
            version: NIP_AB_VERSION,
            purpose: FINITE_PAIRING_PURPOSE_V1.to_owned(),
            pairing_session_id: "pair-bound".to_owned(),
            account_secret_hex: account_secret.clone(),
            account_id: Keys::new(parse_secret_key(&account_secret).unwrap())
                .public_key()
                .to_hex(),
            target_device_id: "ios-bound".to_owned(),
            enrollment_user_id: "user_test".to_owned(),
            enrollment_capability_hex: "cd".repeat(32),
            server_url: "https://chat.finite.test".to_owned(),
            issued_at_unix_seconds: NOW,
            expires_at_unix_seconds: NOW + NIP_AB_SESSION_TTL_SECONDS,
        };
        assert!(
            payload
                .validate(
                    "pair-bound",
                    "ios-bound",
                    "https://chat.finite.test",
                    NOW + 1
                )
                .is_ok()
        );
        assert!(matches!(
            payload.validate(
                "pair-other",
                "ios-bound",
                "https://chat.finite.test",
                NOW + 1
            ),
            Err(NipAbError::InvalidPayload)
        ));
        let mut wrong_account = payload.clone();
        wrong_account.account_id = "22".repeat(32);
        assert!(matches!(
            wrong_account.validate(
                "pair-bound",
                "ios-bound",
                "https://chat.finite.test",
                NOW + 1
            ),
            Err(NipAbError::InvalidPayload)
        ));
        assert!(matches!(
            payload.validate(
                "pair-bound",
                "ios-bound",
                "https://chat.finite.test",
                NOW + NIP_AB_SESSION_TTL_SECONDS + 1
            ),
            Err(NipAbError::InvalidPayload)
        ));
    }
}
