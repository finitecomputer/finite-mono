use crate::types::{MemberId, MessageId};
use serde::{Deserialize, Serialize};

/// Unix-seconds timestamp. Used as an ordering hint only.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Timestamp(pub u64);

/// Source label for a [`TransportMessage`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransportSource(pub String);

/// Raw transport-layer message. The payload is opaque to the delivery service.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportMessage {
    pub id: MessageId,
    pub payload: Vec<u8>,
    pub timestamp: Timestamp,
    pub causal_deps: Vec<MessageId>,
    pub source: TransportSource,
    pub envelope: TransportEnvelope,
}

/// Routing envelope for an opaque transport payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportEnvelope {
    /// Group message scoped by the transport-visible group id.
    GroupMessage { transport_group_id: Vec<u8> },
    /// Welcome addressed to a specific member.
    Welcome { recipient: MemberId },
}
