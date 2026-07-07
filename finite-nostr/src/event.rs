use std::fmt;

use nostr::{Event, EventId, UnsignedEvent};

use crate::NostrPrimitiveError;

/// Hex encoded deterministic event ID.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct EventIdHex(String);

impl EventIdHex {
    /// Parse and validate a hex/bech32/NIP-21 event ID, then normalize to hex.
    pub fn parse(value: &str) -> Result<Self, NostrPrimitiveError> {
        EventId::parse(value)
            .map(|id| Self(id.to_hex()))
            .map_err(|_| NostrPrimitiveError::MalformedInput { field: "event_id" })
    }

    /// Return the lowercase hex string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EventIdHex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Compute the deterministic NIP-01 event ID for an unsigned event.
pub fn compute_event_id(event: &UnsignedEvent) -> EventIdHex {
    EventIdHex(event.compute_id().to_hex())
}

/// Validate deterministic event ID and signature for a signed event.
pub fn verify_event_integrity(event: &Event) -> Result<(), NostrPrimitiveError> {
    if !event.verify_id() {
        return Err(NostrPrimitiveError::InvalidEventId);
    }

    if !event.verify_signature() {
        return Err(NostrPrimitiveError::SignatureFailure);
    }

    Ok(())
}
