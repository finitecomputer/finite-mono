use nostr::event::{FinalizeEvent, SignEvent};
use nostr::key::GetPublicKey;
use nostr::nips::nip44::Nip44;
use nostr::nips::nip59::{GiftWrapBuilder, GiftWrapSealBuilder};
use nostr::{Event, Kind, Tag, Timestamp, UnsignedEvent};

use crate::{NostrPrimitiveError, NostrPublicKey, verify_event_integrity};

/// NIP-59 seal events are kind 13.
pub const SEAL_KIND: u16 = 13;

/// NIP-59 gift-wrap events are kind 1059.
pub const GIFT_WRAP_KIND: u16 = 1_059;

/// Build a NIP-59 rumor as an unsigned event with a deterministic ID.
pub fn build_rumor<I, S>(
    issuer: NostrPublicKey,
    kind: Kind,
    tags: I,
    content: S,
    created_at_unix_seconds: u64,
) -> UnsignedEvent
where
    I: IntoIterator<Item = Tag>,
    S: Into<String>,
{
    let mut rumor = UnsignedEvent::new(
        issuer.as_protocol(),
        Timestamp::from_secs(created_at_unix_seconds),
        kind,
        tags,
        content,
    );
    rumor.ensure_id();
    rumor
}

/// Validate a NIP-59 rumor's deterministic ID and optional issuer.
pub fn validate_rumor(
    rumor: &UnsignedEvent,
    expected_issuer: Option<NostrPublicKey>,
) -> Result<(), NostrPrimitiveError> {
    if rumor.id.is_none() || rumor.verify_id().is_err() {
        return Err(NostrPrimitiveError::InvalidEventId);
    }

    if let Some(expected_issuer) = expected_issuer {
        let actual = NostrPublicKey::from_protocol(rumor.pubkey);
        if actual != expected_issuer {
            return Err(NostrPrimitiveError::WrongIssuer {
                expected: expected_issuer.to_hex(),
                actual: actual.to_hex(),
            });
        }
    }

    Ok(())
}

/// Seal a NIP-59 rumor for a recipient.
pub fn seal_rumor<S>(
    signer: &S,
    recipient: NostrPublicKey,
    rumor: UnsignedEvent,
) -> Result<Event, NostrPrimitiveError>
where
    S: GetPublicKey + SignEvent + Nip44,
{
    validate_rumor(
        &rumor,
        Some(NostrPublicKey::from_protocol(
            signer
                .get_public_key()
                .map_err(|_| NostrPrimitiveError::MalformedInput {
                    field: "signer_public_key",
                })?,
        )),
    )?;

    GiftWrapSealBuilder::new(rumor, recipient.as_protocol())
        .finalize(signer)
        .map_err(|_| NostrPrimitiveError::FailedEncrypt)
}

/// Wrap a NIP-59 rumor for a recipient.
pub fn wrap_rumor<S>(
    signer: &S,
    recipient: NostrPublicKey,
    rumor: UnsignedEvent,
) -> Result<Event, NostrPrimitiveError>
where
    S: GetPublicKey + SignEvent + Nip44,
{
    validate_rumor(
        &rumor,
        Some(NostrPublicKey::from_protocol(
            signer
                .get_public_key()
                .map_err(|_| NostrPrimitiveError::MalformedInput {
                    field: "signer_public_key",
                })?,
        )),
    )?;

    GiftWrapBuilder::new(recipient.as_protocol(), rumor)
        .finalize(signer)
        .map_err(|_| NostrPrimitiveError::FailedEncrypt)
}

/// Expected identities for validating and opening a NIP-59 gift-wrap event.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GiftWrapValidation {
    recipient: NostrPublicKey,
    expected_issuer: Option<NostrPublicKey>,
}

impl GiftWrapValidation {
    /// Create validation rules for a gift-wrap event addressed to `recipient`.
    pub fn new(recipient: NostrPublicKey) -> Self {
        Self {
            recipient,
            expected_issuer: None,
        }
    }

    /// Require the decrypted seal and rumor to come from this issuer.
    pub fn with_expected_issuer(mut self, issuer: NostrPublicKey) -> Self {
        self.expected_issuer = Some(issuer);
        self
    }
}

/// A successfully opened NIP-59 gift-wrap event.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OpenedGiftWrap {
    /// Sender/issuer public key from the seal.
    pub sender: NostrPublicKey,
    /// Recipient public key expected by validation.
    pub recipient: NostrPublicKey,
    /// Decrypted and verified seal event.
    pub seal: Event,
    /// Decrypted and verified rumor.
    pub rumor: UnsignedEvent,
}

/// Validate the visible gift-wrap shell.
pub fn validate_gift_wrap(
    gift_wrap: &Event,
    recipient: NostrPublicKey,
) -> Result<(), NostrPrimitiveError> {
    if gift_wrap.kind != Kind::GiftWrap {
        return Err(NostrPrimitiveError::WrongEventKind {
            expected: GIFT_WRAP_KIND,
            actual: gift_wrap.kind.as_u16(),
        });
    }

    verify_event_integrity(gift_wrap)?;
    validate_gift_wrap_recipient(gift_wrap, recipient)
}

/// Validate a decrypted NIP-59 seal event.
pub fn validate_seal(
    seal: &Event,
    expected_issuer: Option<NostrPublicKey>,
) -> Result<(), NostrPrimitiveError> {
    if seal.kind != Kind::Seal {
        return Err(NostrPrimitiveError::WrongEventKind {
            expected: SEAL_KIND,
            actual: seal.kind.as_u16(),
        });
    }

    verify_event_integrity(seal)?;
    if let Some(expected_issuer) = expected_issuer {
        let actual = NostrPublicKey::from_protocol(seal.pubkey);
        if actual != expected_issuer {
            return Err(NostrPrimitiveError::WrongIssuer {
                expected: expected_issuer.to_hex(),
                actual: actual.to_hex(),
            });
        }
    }

    Ok(())
}

/// Open and validate a NIP-59 gift-wrap event.
pub fn open_gift_wrap<T>(
    recipient_keys: &T,
    gift_wrap: &Event,
    validation: &GiftWrapValidation,
) -> Result<OpenedGiftWrap, NostrPrimitiveError>
where
    T: Nip44,
{
    validate_gift_wrap(gift_wrap, validation.recipient)?;

    let seal_plaintext = recipient_keys
        .nip44_decrypt(&gift_wrap.pubkey, &gift_wrap.content)
        .map_err(|_| NostrPrimitiveError::FailedDecrypt)?;
    let seal: Event = Event::from_json(seal_plaintext)
        .map_err(|_| NostrPrimitiveError::MalformedPlaintext { field: "seal" })?;

    validate_seal(&seal, validation.expected_issuer)?;

    let rumor_plaintext = recipient_keys
        .nip44_decrypt(&seal.pubkey, &seal.content)
        .map_err(|_| NostrPrimitiveError::FailedDecrypt)?;
    let rumor: UnsignedEvent = UnsignedEvent::from_json(rumor_plaintext)
        .map_err(|_| NostrPrimitiveError::MalformedPlaintext { field: "rumor" })?;

    let seal_issuer = NostrPublicKey::from_protocol(seal.pubkey);
    validate_rumor(&rumor, Some(seal_issuer))?;

    Ok(OpenedGiftWrap {
        sender: seal_issuer,
        recipient: validation.recipient,
        seal,
        rumor,
    })
}

fn validate_gift_wrap_recipient(
    gift_wrap: &Event,
    expected_recipient: NostrPublicKey,
) -> Result<(), NostrPrimitiveError> {
    let recipients = gift_wrap
        .tags
        .iter()
        .filter(|tag| tag.kind() == "p")
        .map(|tag| {
            tag.content()
                .ok_or(NostrPrimitiveError::MalformedInput {
                    field: "recipient_tag",
                })
                .and_then(NostrPublicKey::parse)
        })
        .collect::<Result<Vec<_>, _>>()?;

    if recipients.is_empty() {
        return Err(NostrPrimitiveError::MissingRecipient);
    }

    if !recipients.contains(&expected_recipient) {
        return Err(NostrPrimitiveError::WrongRecipient {
            expected: expected_recipient.to_hex(),
            actual: recipients.iter().map(NostrPublicKey::to_hex).collect(),
        });
    }

    Ok(())
}
