//! Single-server HTTP delivery-service state.
//!
//! This crate is intentionally below the Finite Chat MLS layer. It stores and sequences
//! already-wrapped [`finitechat_transport::transport::TransportMessage`] values, leases
//! public KeyPackages, and offers a bounded commit-admission hint. It does not
//! peel MLS bytes, inspect plaintext, or choose Finite Chat's canonical convergence
//! branch. Transport delivery stays evidence for the engine, never consensus.
//!
//! The crate has three parts:
//!
//! - [`HttpDelivery`] is the contract: the operations and invariants any
//!   HTTP delivery implementation must provide.
//! - [`HttpDeliveryService`] is the in-memory reference implementation.
//! - [`conformance`] is an executable check suite. Alternative
//!   implementations (durable, remote, clustered) can run the same checks
//!   against their own [`conformance::HttpDeliveryHarness`] to prove they
//!   match the reference behavior, including across restarts.

use std::collections::{HashMap, HashSet};

use finitechat_transport::engine::KeyPackage;
use finitechat_transport::transport::{Timestamp, TransportEnvelope, TransportMessage};
use finitechat_transport::{EpochId, GroupId, MemberId, MessageId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const HTTP_SERVER_SOURCE: &str = "http-single-server";
pub const MAX_HTTP_ID_BYTES: usize = 128;
pub const MAX_HTTP_SOURCE_BYTES: usize = 64;
pub const MAX_HTTP_TRANSPORT_GROUP_ID_BYTES: usize = 128;
pub const MAX_HTTP_MESSAGE_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const MAX_HTTP_MESSAGE_CAUSAL_DEPS: usize = 64;
pub const MAX_HTTP_KEY_PACKAGE_BYTES: usize = 64 * 1024;
pub const MAX_HTTP_KEY_PACKAGES_PER_ACCOUNT: usize = 64;
pub const MAX_HTTP_GROUPS: usize = 1024;
pub const MAX_HTTP_RECIPIENT_INBOXES: usize = 4096;
pub const MAX_HTTP_QUEUE_ENTRIES_PER_ROUTE: usize = 4096;
pub const MAX_HTTP_SYNC_PAGE_ENTRIES: usize = 100;

pub type HttpSequence = u64;

/// Capacity limits for one [`HttpDeliveryService`].
///
/// `Default` matches the crate constants, which are sized for tests and small
/// deployments. Production wrappers can raise them via
/// [`HttpDeliveryService::with_limits`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpDeliveryLimits {
    pub max_groups: usize,
    pub max_recipient_inboxes: usize,
    pub max_queue_entries_per_route: usize,
    pub max_key_packages_per_account: usize,
}

impl Default for HttpDeliveryLimits {
    fn default() -> Self {
        Self {
            max_groups: MAX_HTTP_GROUPS,
            max_recipient_inboxes: MAX_HTTP_RECIPIENT_INBOXES,
            max_queue_entries_per_route: MAX_HTTP_QUEUE_ENTRIES_PER_ROUTE,
            max_key_packages_per_account: MAX_HTTP_KEY_PACKAGES_PER_ACCOUNT,
        }
    }
}

/// Outcome of a dry-run publish admission check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpPublishCheck {
    /// The publish is admissible; the matching publish would append a new
    /// entry and return exactly this receipt.
    Fresh(HttpPublishReceipt),
    /// An identical message was already accepted; publishing it again replays
    /// this receipt without appending.
    DuplicateReplay(HttpPublishReceipt),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HttpKeyPackageId(pub Vec<u8>);

impl HttpKeyPackageId {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpCommitAdmission {
    pub source_epoch: EpochId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpPublishTarget {
    Group {
        group_id: GroupId,
        transport_group_id: Vec<u8>,
        commit_admission: Option<HttpCommitAdmission>,
    },
    Inbox {
        recipient: MemberId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpDeliveryPlane {
    Group,
    Inbox,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpPublishReceipt {
    pub message_id: MessageId,
    pub plane: HttpDeliveryPlane,
    pub seq: HttpSequence,
    pub duplicate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpQueuedDelivery {
    pub seq: HttpSequence,
    pub message: TransportMessage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpSyncPage {
    pub entries: Vec<HttpQueuedDelivery>,
    pub next_after_seq: HttpSequence,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpKeyPackagePublication {
    pub key_package_id: HttpKeyPackageId,
    pub owner: MemberId,
    pub key_package: KeyPackage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpClaimedKeyPackage {
    pub key_package_id: HttpKeyPackageId,
    pub owner: MemberId,
    pub key_package: KeyPackage,
}

pub fn prove_http_delivery_core_orders_commit_then_message()
-> Result<Vec<MessageId>, HttpServerError> {
    let mut service = HttpDeliveryService::default();
    let group_id = GroupId::new(b"finitechat-smoke-room".to_vec());
    let transport_group_id = b"finitechat-smoke-transport".to_vec();

    service.publish(
        HttpPublishTarget::Group {
            group_id: group_id.clone(),
            transport_group_id: transport_group_id.clone(),
            commit_admission: Some(HttpCommitAdmission {
                source_epoch: EpochId(1),
            }),
        },
        smoke_group_message("commit-epoch-1", transport_group_id.clone(), b"commit"),
    )?;
    service.publish(
        HttpPublishTarget::Group {
            group_id: group_id.clone(),
            transport_group_id,
            commit_admission: None,
        },
        smoke_group_message(
            "app-message-epoch-2",
            b"finitechat-smoke-transport".to_vec(),
            b"app",
        ),
    )?;

    let page = service.sync_group(&group_id, 0, 10)?;
    Ok(page
        .entries
        .into_iter()
        .map(|entry| entry.message.id)
        .collect())
}

fn smoke_group_message(id: &str, transport_group_id: Vec<u8>, payload: &[u8]) -> TransportMessage {
    TransportMessage {
        id: MessageId::new(id.as_bytes().to_vec()),
        payload: payload.to_vec(),
        timestamp: Timestamp(1),
        causal_deps: Vec::new(),
        source: finitechat_transport::TransportSource(HTTP_SERVER_SOURCE.to_owned()),
        envelope: TransportEnvelope::GroupMessage { transport_group_id },
    }
}

/// Serialize maps with non-string keys as sequences of pairs so snapshots can
/// use JSON.
mod map_as_pairs {
    use std::collections::HashMap;
    use std::hash::Hash;

    use serde::de::{Deserialize, Deserializer};
    use serde::ser::{Serialize, Serializer};

    pub fn serialize<K, V, S>(map: &HashMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize,
        V: Serialize,
        S: Serializer,
    {
        serializer.collect_seq(map.iter())
    }

    pub fn deserialize<'de, K, V, D>(deserializer: D) -> Result<HashMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Eq + Hash,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        Ok(Vec::<(K, V)>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}

/// The HTTP delivery-service contract.
///
/// Implementations sequence opaque [`TransportMessage`] bytes per group,
/// queue Welcome messages per recipient inbox, and lease one-time
/// KeyPackages. They must uphold the invariants checked by [`conformance`]:
/// dense per-queue sequences, digest-exact duplicate replay, one admitted
/// commit per source epoch, envelope/target agreement, and consume-once
/// KeyPackage claims.
pub trait HttpDelivery {
    fn publish(
        &mut self,
        target: HttpPublishTarget,
        message: TransportMessage,
    ) -> Result<HttpPublishReceipt, HttpServerError>;

    fn sync_group(
        &self,
        group_id: &GroupId,
        after_seq: HttpSequence,
        limit: usize,
    ) -> Result<HttpSyncPage, HttpServerError>;

    fn sync_inbox(
        &self,
        recipient: &MemberId,
        after_seq: HttpSequence,
        limit: usize,
    ) -> Result<HttpSyncPage, HttpServerError>;

    fn publish_key_package(
        &mut self,
        publication: HttpKeyPackagePublication,
    ) -> Result<(), HttpServerError>;

    fn claim_key_package(
        &mut self,
        owner: &MemberId,
    ) -> Result<Option<HttpClaimedKeyPackage>, HttpServerError>;
}

/// In-memory reference implementation of [`HttpDelivery`].
///
/// Serializable so durable wrappers can snapshot the whole state and boot
/// from snapshot + tail replay instead of replaying full history.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HttpDeliveryService {
    limits: HttpDeliveryLimits,
    #[serde(with = "map_as_pairs")]
    groups: HashMap<GroupId, GroupQueue>,
    #[serde(with = "map_as_pairs")]
    inboxes: HashMap<MemberId, InboxQueue>,
    #[serde(with = "map_as_pairs")]
    key_packages: HashMap<HttpKeyPackageId, KeyPackageRecord>,
}

impl HttpDelivery for HttpDeliveryService {
    fn publish(
        &mut self,
        target: HttpPublishTarget,
        message: TransportMessage,
    ) -> Result<HttpPublishReceipt, HttpServerError> {
        HttpDeliveryService::publish(self, target, message)
    }

    fn sync_group(
        &self,
        group_id: &GroupId,
        after_seq: HttpSequence,
        limit: usize,
    ) -> Result<HttpSyncPage, HttpServerError> {
        HttpDeliveryService::sync_group(self, group_id, after_seq, limit)
    }

    fn sync_inbox(
        &self,
        recipient: &MemberId,
        after_seq: HttpSequence,
        limit: usize,
    ) -> Result<HttpSyncPage, HttpServerError> {
        HttpDeliveryService::sync_inbox(self, recipient, after_seq, limit)
    }

    fn publish_key_package(
        &mut self,
        publication: HttpKeyPackagePublication,
    ) -> Result<(), HttpServerError> {
        HttpDeliveryService::publish_key_package(self, publication)
    }

    fn claim_key_package(
        &mut self,
        owner: &MemberId,
    ) -> Result<Option<HttpClaimedKeyPackage>, HttpServerError> {
        HttpDeliveryService::claim_key_package(self, owner)
    }
}

impl HttpDeliveryService {
    /// Build a service with non-default capacity limits.
    pub fn with_limits(limits: HttpDeliveryLimits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    pub fn limits(&self) -> HttpDeliveryLimits {
        self.limits
    }

    /// Validate a publish without mutating any state.
    ///
    /// Durable wrappers can persist the operation first and then apply it:
    /// if this returns [`HttpPublishCheck::Fresh`] and no other mutation
    /// intervenes, the matching [`HttpDeliveryService::publish`] cannot
    /// fail. [`HttpPublishCheck::DuplicateReplay`] carries the receipt the
    /// matching publish would return, so wrappers can skip persisting
    /// duplicates entirely.
    pub fn check_publish(
        &self,
        target: &HttpPublishTarget,
        message: &TransportMessage,
    ) -> Result<HttpPublishCheck, HttpServerError> {
        validate_transport_message(message)?;
        validate_target_matches_message(target, message)?;
        let message_digest = digest_transport_message(message);
        match target {
            HttpPublishTarget::Group {
                group_id,
                transport_group_id,
                commit_admission,
            } => {
                validate_group_id(group_id)?;
                validate_transport_group_id(transport_group_id)?;
                match self.groups.get(group_id) {
                    Some(group) => group.check_append(
                        &message.id,
                        message_digest,
                        *commit_admission,
                        self.limits.max_queue_entries_per_route,
                    ),
                    None if self.groups.len() >= self.limits.max_groups => {
                        Err(HttpServerError::GroupLimitExceeded {
                            max: self.limits.max_groups,
                        })
                    }
                    None => Ok(HttpPublishCheck::Fresh(HttpPublishReceipt {
                        message_id: message.id.clone(),
                        plane: HttpDeliveryPlane::Group,
                        seq: 1,
                        duplicate: false,
                    })),
                }
            }
            HttpPublishTarget::Inbox { recipient } => {
                validate_member_id("recipient", recipient)?;
                match self.inboxes.get(recipient) {
                    Some(inbox) => inbox.check_append(
                        &message.id,
                        message_digest,
                        self.limits.max_queue_entries_per_route,
                    ),
                    None if self.inboxes.len() >= self.limits.max_recipient_inboxes => {
                        Err(HttpServerError::InboxLimitExceeded {
                            max: self.limits.max_recipient_inboxes,
                        })
                    }
                    None => Ok(HttpPublishCheck::Fresh(HttpPublishReceipt {
                        message_id: message.id.clone(),
                        plane: HttpDeliveryPlane::Inbox,
                        seq: 1,
                        duplicate: false,
                    })),
                }
            }
        }
    }

    pub fn publish(
        &mut self,
        target: HttpPublishTarget,
        message: TransportMessage,
    ) -> Result<HttpPublishReceipt, HttpServerError> {
        validate_transport_message(&message)?;
        validate_target_matches_message(&target, &message)?;
        let message_digest = digest_transport_message(&message);
        match target {
            HttpPublishTarget::Group {
                group_id,
                transport_group_id,
                commit_admission,
            } => {
                validate_group_id(&group_id)?;
                validate_transport_group_id(&transport_group_id)?;
                self.publish_group(group_id, commit_admission, message, message_digest)
            }
            HttpPublishTarget::Inbox { recipient } => {
                validate_member_id("recipient", &recipient)?;
                self.publish_inbox(recipient, message, message_digest)
            }
        }
    }

    pub fn sync_group(
        &self,
        group_id: &GroupId,
        after_seq: HttpSequence,
        limit: usize,
    ) -> Result<HttpSyncPage, HttpServerError> {
        validate_group_id(group_id)?;
        validate_page_limit(limit)?;
        let Some(group) = self.groups.get(group_id) else {
            return Ok(empty_sync_page(after_seq));
        };
        Ok(sync_page(&group.entries, after_seq, limit))
    }

    pub fn sync_inbox(
        &self,
        recipient: &MemberId,
        after_seq: HttpSequence,
        limit: usize,
    ) -> Result<HttpSyncPage, HttpServerError> {
        validate_member_id("recipient", recipient)?;
        validate_page_limit(limit)?;
        let Some(inbox) = self.inboxes.get(recipient) else {
            return Ok(empty_sync_page(after_seq));
        };
        Ok(sync_page(&inbox.entries, after_seq, limit))
    }

    pub fn publish_key_package(
        &mut self,
        publication: HttpKeyPackagePublication,
    ) -> Result<(), HttpServerError> {
        validate_key_package_publication(&publication)?;
        if let Some(existing) = self.key_packages.get(&publication.key_package_id) {
            if key_package_record_matches(existing, &publication) {
                return Ok(());
            }
            return Err(HttpServerError::ConflictingKeyPackage {
                key_package_id: publication.key_package_id,
            });
        }
        let unconsumed = self
            .key_packages
            .values()
            .filter(|record| record.owner == publication.owner && record.state.is_unconsumed())
            .count();
        if unconsumed >= self.limits.max_key_packages_per_account {
            return Err(HttpServerError::KeyPackageInventoryFull {
                owner: publication.owner,
                max: self.limits.max_key_packages_per_account,
            });
        }
        self.key_packages.insert(
            publication.key_package_id.clone(),
            KeyPackageRecord {
                owner: publication.owner,
                key_package: publication.key_package,
                state: KeyPackageState::Available,
            },
        );
        Ok(())
    }

    pub fn claim_key_package(
        &mut self,
        owner: &MemberId,
    ) -> Result<Option<HttpClaimedKeyPackage>, HttpServerError> {
        validate_member_id("owner", owner)?;
        let selected = self
            .key_packages
            .iter()
            .filter(|(_, record)| {
                record.owner == *owner && record.state == KeyPackageState::Available
            })
            .map(|(key_package_id, _)| key_package_id.clone())
            .min_by(|left, right| left.0.cmp(&right.0));
        let Some(key_package_id) = selected else {
            return Ok(None);
        };
        let record = self
            .key_packages
            .get_mut(&key_package_id)
            .expect("selected available KeyPackage must exist before mutation");
        record.state = KeyPackageState::Consumed;
        Ok(Some(HttpClaimedKeyPackage {
            key_package_id,
            owner: record.owner.clone(),
            key_package: record.key_package.clone(),
        }))
    }

    fn publish_group(
        &mut self,
        group_id: GroupId,
        commit_admission: Option<HttpCommitAdmission>,
        message: TransportMessage,
        message_digest: [u8; 32],
    ) -> Result<HttpPublishReceipt, HttpServerError> {
        let max_entries = self.limits.max_queue_entries_per_route;
        if let Some(group) = self.groups.get_mut(&group_id) {
            return group.append(message, message_digest, commit_admission, max_entries);
        }
        if self.groups.len() >= self.limits.max_groups {
            return Err(HttpServerError::GroupLimitExceeded {
                max: self.limits.max_groups,
            });
        }
        let mut group = GroupQueue::default();
        let receipt = group.append(message, message_digest, commit_admission, max_entries)?;
        self.groups.insert(group_id, group);
        Ok(receipt)
    }

    fn publish_inbox(
        &mut self,
        recipient: MemberId,
        message: TransportMessage,
        message_digest: [u8; 32],
    ) -> Result<HttpPublishReceipt, HttpServerError> {
        let max_entries = self.limits.max_queue_entries_per_route;
        if let Some(inbox) = self.inboxes.get_mut(&recipient) {
            return inbox.append(message, message_digest, max_entries);
        }
        if self.inboxes.len() >= self.limits.max_recipient_inboxes {
            return Err(HttpServerError::InboxLimitExceeded {
                max: self.limits.max_recipient_inboxes,
            });
        }
        let mut inbox = InboxQueue::default();
        let receipt = inbox.append(message, message_digest, max_entries)?;
        self.inboxes.insert(recipient, inbox);
        Ok(receipt)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct GroupQueue {
    entries: Vec<HttpQueuedDelivery>,
    #[serde(with = "map_as_pairs")]
    messages: HashMap<MessageId, MessageIndex>,
    accepted_commit_epochs: HashSet<EpochId>,
}

impl GroupQueue {
    fn check_append(
        &self,
        message_id: &MessageId,
        message_digest: [u8; 32],
        commit_admission: Option<HttpCommitAdmission>,
        max_entries: usize,
    ) -> Result<HttpPublishCheck, HttpServerError> {
        if let Some(existing) = self.messages.get(message_id) {
            return replay_or_reject_duplicate(
                message_id,
                message_digest,
                existing,
                HttpDeliveryPlane::Group,
            )
            .map(HttpPublishCheck::DuplicateReplay);
        }
        if let Some(admission) = commit_admission
            && self
                .accepted_commit_epochs
                .contains(&admission.source_epoch)
        {
            return Err(HttpServerError::StaleEpoch {
                source_epoch: admission.source_epoch,
            });
        }
        ensure_queue_has_space(HttpDeliveryPlane::Group, self.entries.len(), max_entries)?;
        Ok(HttpPublishCheck::Fresh(HttpPublishReceipt {
            message_id: message_id.clone(),
            plane: HttpDeliveryPlane::Group,
            seq: (self.entries.len() as HttpSequence) + 1,
            duplicate: false,
        }))
    }

    fn append(
        &mut self,
        message: TransportMessage,
        message_digest: [u8; 32],
        commit_admission: Option<HttpCommitAdmission>,
        max_entries: usize,
    ) -> Result<HttpPublishReceipt, HttpServerError> {
        match self.check_append(&message.id, message_digest, commit_admission, max_entries)? {
            HttpPublishCheck::DuplicateReplay(receipt) => return Ok(receipt),
            HttpPublishCheck::Fresh(_) => {}
        }
        let seq = (self.entries.len() as HttpSequence) + 1;
        self.messages.insert(
            message.id.clone(),
            MessageIndex {
                seq,
                digest: message_digest,
            },
        );
        self.entries.push(HttpQueuedDelivery {
            seq,
            message: message.clone(),
        });
        if let Some(admission) = commit_admission {
            self.accepted_commit_epochs.insert(admission.source_epoch);
        }
        debug_assert_eq!(self.entries.last().map(|entry| entry.seq), Some(seq));
        Ok(HttpPublishReceipt {
            message_id: message.id,
            plane: HttpDeliveryPlane::Group,
            seq,
            duplicate: false,
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct InboxQueue {
    entries: Vec<HttpQueuedDelivery>,
    #[serde(with = "map_as_pairs")]
    messages: HashMap<MessageId, MessageIndex>,
}

impl InboxQueue {
    fn check_append(
        &self,
        message_id: &MessageId,
        message_digest: [u8; 32],
        max_entries: usize,
    ) -> Result<HttpPublishCheck, HttpServerError> {
        if let Some(existing) = self.messages.get(message_id) {
            return replay_or_reject_duplicate(
                message_id,
                message_digest,
                existing,
                HttpDeliveryPlane::Inbox,
            )
            .map(HttpPublishCheck::DuplicateReplay);
        }
        ensure_queue_has_space(HttpDeliveryPlane::Inbox, self.entries.len(), max_entries)?;
        Ok(HttpPublishCheck::Fresh(HttpPublishReceipt {
            message_id: message_id.clone(),
            plane: HttpDeliveryPlane::Inbox,
            seq: (self.entries.len() as HttpSequence) + 1,
            duplicate: false,
        }))
    }

    fn append(
        &mut self,
        message: TransportMessage,
        message_digest: [u8; 32],
        max_entries: usize,
    ) -> Result<HttpPublishReceipt, HttpServerError> {
        match self.check_append(&message.id, message_digest, max_entries)? {
            HttpPublishCheck::DuplicateReplay(receipt) => return Ok(receipt),
            HttpPublishCheck::Fresh(_) => {}
        }
        let seq = (self.entries.len() as HttpSequence) + 1;
        self.messages.insert(
            message.id.clone(),
            MessageIndex {
                seq,
                digest: message_digest,
            },
        );
        self.entries.push(HttpQueuedDelivery {
            seq,
            message: message.clone(),
        });
        debug_assert_eq!(self.entries.last().map(|entry| entry.seq), Some(seq));
        Ok(HttpPublishReceipt {
            message_id: message.id,
            plane: HttpDeliveryPlane::Inbox,
            seq,
            duplicate: false,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MessageIndex {
    seq: HttpSequence,
    digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum KeyPackageState {
    Available,
    Consumed,
}

impl KeyPackageState {
    fn is_unconsumed(self) -> bool {
        match self {
            Self::Available => true,
            Self::Consumed => false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct KeyPackageRecord {
    owner: MemberId,
    key_package: KeyPackage,
    state: KeyPackageState,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HttpServerError {
    #[error("{field} must not be empty")]
    Empty { field: &'static str },
    #[error("{field} length {actual} exceeds max {max}")]
    TooLarge {
        field: &'static str,
        actual: usize,
        max: usize,
    },
    #[error("publish target does not match message envelope")]
    PublishTargetMismatch,
    #[error("conflicting message id: {message_id}")]
    ConflictingMessageId { message_id: MessageId },
    #[error("commit for source epoch {source_epoch} is stale")]
    StaleEpoch { source_epoch: EpochId },
    #[error("queue for {plane:?} is full: max {max}")]
    QueueFull {
        plane: HttpDeliveryPlane,
        max: usize,
    },
    #[error("group limit exceeded: max {max}")]
    GroupLimitExceeded { max: usize },
    #[error("inbox limit exceeded: max {max}")]
    InboxLimitExceeded { max: usize },
    #[error("sync page limit must be between 1 and {max}, got {actual}")]
    InvalidPageLimit { actual: usize, max: usize },
    #[error("conflicting KeyPackage id: {}", hex::encode(key_package_id.as_slice()))]
    ConflictingKeyPackage { key_package_id: HttpKeyPackageId },
    #[error("KeyPackage inventory full for owner {owner}: max {max}")]
    KeyPackageInventoryFull { owner: MemberId, max: usize },
}

fn replay_or_reject_duplicate(
    message_id: &MessageId,
    message_digest: [u8; 32],
    existing: &MessageIndex,
    plane: HttpDeliveryPlane,
) -> Result<HttpPublishReceipt, HttpServerError> {
    if existing.digest == message_digest {
        Ok(HttpPublishReceipt {
            message_id: message_id.clone(),
            plane,
            seq: existing.seq,
            duplicate: true,
        })
    } else {
        Err(HttpServerError::ConflictingMessageId {
            message_id: message_id.clone(),
        })
    }
}

fn empty_sync_page(after_seq: HttpSequence) -> HttpSyncPage {
    HttpSyncPage {
        entries: Vec::new(),
        next_after_seq: after_seq,
        has_more: false,
    }
}

fn sync_page(
    entries: &[HttpQueuedDelivery],
    after_seq: HttpSequence,
    limit: usize,
) -> HttpSyncPage {
    // Entries are appended in strictly increasing seq order, so the page
    // start can be found by binary search instead of a scan from seq 0.
    let start = entries.partition_point(|entry| entry.seq <= after_seq);
    let mut page_entries = Vec::new();
    let mut next_after_seq = after_seq;
    let mut has_more = false;
    for entry in &entries[start..] {
        if page_entries.len() == limit {
            has_more = true;
            break;
        }
        next_after_seq = entry.seq;
        page_entries.push(entry.clone());
    }
    HttpSyncPage {
        entries: page_entries,
        next_after_seq,
        has_more,
    }
}

fn ensure_queue_has_space(
    plane: HttpDeliveryPlane,
    current_len: usize,
    max_entries: usize,
) -> Result<(), HttpServerError> {
    if current_len < max_entries {
        Ok(())
    } else {
        Err(HttpServerError::QueueFull {
            plane,
            max: max_entries,
        })
    }
}

fn validate_target_matches_message(
    target: &HttpPublishTarget,
    message: &TransportMessage,
) -> Result<(), HttpServerError> {
    match (target, &message.envelope) {
        (
            HttpPublishTarget::Group {
                transport_group_id, ..
            },
            TransportEnvelope::GroupMessage {
                transport_group_id: message_group_id,
            },
        ) if transport_group_id == message_group_id => Ok(()),
        (
            HttpPublishTarget::Inbox { recipient },
            TransportEnvelope::Welcome {
                recipient: message_recipient,
            },
        ) if recipient == message_recipient => Ok(()),
        _ => Err(HttpServerError::PublishTargetMismatch),
    }
}

fn validate_key_package_publication(
    publication: &HttpKeyPackagePublication,
) -> Result<(), HttpServerError> {
    validate_key_package_id(&publication.key_package_id)?;
    validate_member_id("owner", &publication.owner)?;
    validate_non_empty_len(
        "key_package.bytes",
        publication.key_package.bytes.len(),
        MAX_HTTP_KEY_PACKAGE_BYTES,
    )
}

fn key_package_record_matches(
    record: &KeyPackageRecord,
    publication: &HttpKeyPackagePublication,
) -> bool {
    record.owner == publication.owner && record.key_package == publication.key_package
}

fn validate_transport_message(message: &TransportMessage) -> Result<(), HttpServerError> {
    validate_message_id("message.id", &message.id)?;
    validate_non_empty_len(
        "message.payload",
        message.payload.len(),
        MAX_HTTP_MESSAGE_PAYLOAD_BYTES,
    )?;
    validate_string_len("message.source", &message.source.0, MAX_HTTP_SOURCE_BYTES)?;
    validate_item_count(
        "message.causal_deps",
        message.causal_deps.len(),
        MAX_HTTP_MESSAGE_CAUSAL_DEPS,
    )?;
    for dep in &message.causal_deps {
        validate_message_id("message.causal_deps", dep)?;
    }
    match &message.envelope {
        TransportEnvelope::GroupMessage { transport_group_id } => {
            validate_transport_group_id(transport_group_id)
        }
        TransportEnvelope::Welcome { recipient } => {
            validate_member_id("welcome.recipient", recipient)
        }
    }
}

fn validate_group_id(group_id: &GroupId) -> Result<(), HttpServerError> {
    validate_bytes("group_id", group_id.as_slice(), MAX_HTTP_ID_BYTES)
}

fn validate_member_id(field: &'static str, member_id: &MemberId) -> Result<(), HttpServerError> {
    validate_bytes(field, member_id.as_slice(), MAX_HTTP_ID_BYTES)
}

fn validate_message_id(field: &'static str, message_id: &MessageId) -> Result<(), HttpServerError> {
    validate_bytes(field, message_id.as_slice(), MAX_HTTP_ID_BYTES)
}

fn validate_key_package_id(key_package_id: &HttpKeyPackageId) -> Result<(), HttpServerError> {
    validate_bytes(
        "key_package_id",
        key_package_id.as_slice(),
        MAX_HTTP_ID_BYTES,
    )
}

fn validate_transport_group_id(transport_group_id: &[u8]) -> Result<(), HttpServerError> {
    validate_bytes(
        "transport_group_id",
        transport_group_id,
        MAX_HTTP_TRANSPORT_GROUP_ID_BYTES,
    )
}

fn validate_bytes(field: &'static str, bytes: &[u8], max: usize) -> Result<(), HttpServerError> {
    validate_non_empty_len(field, bytes.len(), max)
}

fn validate_string_len(
    field: &'static str,
    value: &str,
    max: usize,
) -> Result<(), HttpServerError> {
    validate_non_empty_len(field, value.len(), max)
}

fn validate_non_empty_len(
    field: &'static str,
    actual: usize,
    max: usize,
) -> Result<(), HttpServerError> {
    if actual == 0 {
        return Err(HttpServerError::Empty { field });
    }
    if actual > max {
        return Err(HttpServerError::TooLarge { field, actual, max });
    }
    Ok(())
}

fn validate_item_count(
    field: &'static str,
    actual: usize,
    max: usize,
) -> Result<(), HttpServerError> {
    if actual <= max {
        Ok(())
    } else {
        Err(HttpServerError::TooLarge { field, actual, max })
    }
}

fn validate_page_limit(limit: usize) -> Result<(), HttpServerError> {
    if (1..=MAX_HTTP_SYNC_PAGE_ENTRIES).contains(&limit) {
        Ok(())
    } else {
        Err(HttpServerError::InvalidPageLimit {
            actual: limit,
            max: MAX_HTTP_SYNC_PAGE_ENTRIES,
        })
    }
}

fn digest_transport_message(message: &TransportMessage) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hash_len_prefixed(&mut hasher, message.id.as_slice());
    hash_len_prefixed(&mut hasher, &message.payload);
    hasher.update(message.timestamp.0.to_be_bytes());
    hasher.update((message.causal_deps.len() as u64).to_be_bytes());
    for dep in &message.causal_deps {
        hash_len_prefixed(&mut hasher, dep.as_slice());
    }
    hash_len_prefixed(&mut hasher, message.source.0.as_bytes());
    match &message.envelope {
        TransportEnvelope::GroupMessage { transport_group_id } => {
            hasher.update([0_u8]);
            hash_len_prefixed(&mut hasher, transport_group_id);
        }
        TransportEnvelope::Welcome { recipient } => {
            hasher.update([1_u8]);
            hash_len_prefixed(&mut hasher, recipient.as_slice());
        }
    }
    hasher.finalize().into()
}

fn hash_len_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

pub mod conformance {
    //! Executable checks for the [`HttpDelivery`](super::HttpDelivery)
    //! contract.
    //!
    //! Each check panics with a descriptive message on contract violation, so
    //! a downstream implementation can expose them as ordinary `#[test]`
    //! functions:
    //!
    //! ```
    //! use finitechat_delivery::{HttpDeliveryService, conformance};
    //!
    //! let mut service = HttpDeliveryService::default();
    //! conformance::check_all(&mut service);
    //! ```
    //!
    //! Checks namespace their group, member, and message ids, so one harness
    //! can run every check in sequence.

    use finitechat_transport::engine::KeyPackage;
    use finitechat_transport::transport::{
        Timestamp, TransportEnvelope, TransportMessage, TransportSource,
    };
    use finitechat_transport::{EpochId, GroupId, MemberId, MessageId};

    use super::{
        HTTP_SERVER_SOURCE, HttpClaimedKeyPackage, HttpCommitAdmission, HttpDelivery,
        HttpDeliveryPlane, HttpKeyPackageId, HttpKeyPackagePublication, HttpPublishTarget,
        HttpServerError,
    };

    /// Provides the implementation under test to the conformance checks.
    ///
    /// `restart` lets durable implementations simulate a process restart
    /// (drop volatile state, reload from storage) between operations. The
    /// in-memory reference is not durable, so its blanket harness returns
    /// `false` and [`check_state_survives_restart`] skips its post-restart
    /// assertions.
    pub trait HttpDeliveryHarness {
        type Delivery: HttpDelivery;

        fn delivery(&mut self) -> &mut Self::Delivery;

        /// Simulate a process restart. Return `false` if this implementation
        /// keeps no durable state; durability assertions are then skipped.
        fn restart(&mut self) -> bool {
            false
        }
    }

    impl HttpDeliveryHarness for super::HttpDeliveryService {
        type Delivery = Self;

        fn delivery(&mut self) -> &mut Self {
            self
        }
    }

    /// Run every conformance check against one harness.
    pub fn check_all<H: HttpDeliveryHarness>(harness: &mut H) {
        check_group_ordering_and_duplicate_replay(harness);
        check_conflicting_duplicate_rejection(harness);
        check_commit_admission_per_source_epoch(harness);
        check_inbox_sequencing(harness);
        check_publish_target_envelope_match(harness);
        check_key_package_lifecycle(harness);
        check_state_survives_restart(harness);
    }

    /// Group publishes receive dense sequences starting at 1, exact
    /// duplicates replay the original receipt, and sync pages are bounded
    /// with `has_more`/cursor continuation.
    pub fn check_group_ordering_and_duplicate_replay<H: HttpDeliveryHarness>(harness: &mut H) {
        let scope = "conformance/group-ordering";
        let service = harness.delivery();
        let first = group_message(scope, "msg-1", b"first");

        let first_receipt = service
            .publish(group_target(scope, None), first.clone())
            .expect("conformance: first group publish is accepted");
        assert_eq!(first_receipt.seq, 1, "first group publish must take seq 1");
        assert_eq!(first_receipt.plane, HttpDeliveryPlane::Group);
        assert!(!first_receipt.duplicate);

        let replay = service
            .publish(group_target(scope, None), first)
            .expect("conformance: exact duplicate publish replays the receipt");
        assert_eq!(replay.seq, 1, "duplicate replay must keep the original seq");
        assert!(replay.duplicate, "duplicate replay must be flagged");

        let second_receipt = service
            .publish(
                group_target(scope, None),
                group_message(scope, "msg-2", b"second"),
            )
            .expect("conformance: second group publish is accepted");
        assert_eq!(second_receipt.seq, 2, "group sequence must be dense");

        let page = service
            .sync_group(&group_id(scope), 0, 1)
            .expect("conformance: bounded group sync succeeds");
        assert_eq!(page.entries.len(), 1, "page limit must bound entries");
        assert_eq!(page.entries[0].seq, 1);
        assert_eq!(page.next_after_seq, 1, "cursor must point at last entry");
        assert!(
            page.has_more,
            "a full page with a successor must set has_more"
        );

        let page = service
            .sync_group(&group_id(scope), page.next_after_seq, 10)
            .expect("conformance: cursor continuation succeeds");
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].seq, 2);
        assert_eq!(page.next_after_seq, 2);
        assert!(!page.has_more, "the final page must clear has_more");
    }

    /// Reusing a message id with different bytes is a conflict, not a replay.
    pub fn check_conflicting_duplicate_rejection<H: HttpDeliveryHarness>(harness: &mut H) {
        let scope = "conformance/conflicting-duplicate";
        let service = harness.delivery();
        service
            .publish(
                group_target(scope, None),
                group_message(scope, "msg-1", b"first"),
            )
            .expect("conformance: original publish is accepted");

        let err = service
            .publish(
                group_target(scope, None),
                group_message(scope, "msg-1", b"changed"),
            )
            .expect_err("conformance: conflicting duplicate must be rejected");
        assert_eq!(
            err,
            HttpServerError::ConflictingMessageId {
                message_id: message_id(scope, "msg-1")
            }
        );
    }

    /// At most one commit-admitted message is accepted per source epoch.
    pub fn check_commit_admission_per_source_epoch<H: HttpDeliveryHarness>(harness: &mut H) {
        let scope = "conformance/commit-admission";
        let service = harness.delivery();
        let admission = HttpCommitAdmission {
            source_epoch: EpochId(7),
        };
        let winner = service
            .publish(
                group_target(scope, Some(admission)),
                group_message(scope, "commit-1", b"one"),
            )
            .expect("conformance: first commit for the source epoch is accepted");
        assert_eq!(winner.seq, 1);

        let err = service
            .publish(
                group_target(scope, Some(admission)),
                group_message(scope, "commit-2", b"two"),
            )
            .expect_err("conformance: second commit for the same source epoch must be rejected");
        assert_eq!(
            err,
            HttpServerError::StaleEpoch {
                source_epoch: EpochId(7)
            }
        );
    }

    /// Welcome publishes sequence per recipient inbox and sync back out.
    pub fn check_inbox_sequencing<H: HttpDeliveryHarness>(harness: &mut H) {
        let scope = "conformance/inbox";
        let service = harness.delivery();
        let recipient = member(scope, "bob");
        let receipt = service
            .publish(
                HttpPublishTarget::Inbox {
                    recipient: recipient.clone(),
                },
                welcome_message(scope, "welcome-1", recipient.clone()),
            )
            .expect("conformance: welcome publish is accepted");
        assert_eq!(receipt.plane, HttpDeliveryPlane::Inbox);
        assert_eq!(receipt.seq, 1, "inbox sequence must start at 1");

        let page = service
            .sync_inbox(&recipient, 0, 10)
            .expect("conformance: inbox sync succeeds");
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].message.id, message_id(scope, "welcome-1"));
    }

    /// The publish target and the message envelope must agree.
    pub fn check_publish_target_envelope_match<H: HttpDeliveryHarness>(harness: &mut H) {
        let scope = "conformance/target-mismatch";
        let service = harness.delivery();
        let err = service
            .publish(
                HttpPublishTarget::Inbox {
                    recipient: member(scope, "bob"),
                },
                group_message(scope, "msg-1", b"not-a-welcome"),
            )
            .expect_err("conformance: target/envelope mismatch must be rejected");
        assert_eq!(err, HttpServerError::PublishTargetMismatch);
    }

    /// KeyPackage publication is idempotent for identical bytes, conflicting
    /// bytes are rejected, and a claim consumes the package exactly once.
    pub fn check_key_package_lifecycle<H: HttpDeliveryHarness>(harness: &mut H) {
        let scope = "conformance/key-package";
        let service = harness.delivery();
        let owner = member(scope, "alice");
        let publication = HttpKeyPackagePublication {
            key_package_id: key_package_id(scope, "kp-1"),
            owner: owner.clone(),
            key_package: KeyPackage::new(b"conformance-key-package-1".to_vec()),
        };

        service
            .publish_key_package(publication.clone())
            .expect("conformance: KeyPackage publication is accepted");
        service
            .publish_key_package(publication.clone())
            .expect("conformance: exact duplicate KeyPackage publication is idempotent");

        let conflicting = HttpKeyPackagePublication {
            key_package: KeyPackage::new(b"changed".to_vec()),
            ..publication.clone()
        };
        let err = service
            .publish_key_package(conflicting)
            .expect_err("conformance: conflicting KeyPackage bytes must be rejected");
        assert_eq!(
            err,
            HttpServerError::ConflictingKeyPackage {
                key_package_id: key_package_id(scope, "kp-1")
            }
        );

        let claimed = service
            .claim_key_package(&owner)
            .expect("conformance: claim request succeeds")
            .expect("conformance: one package must be claimable");
        assert_eq!(
            claimed,
            HttpClaimedKeyPackage {
                key_package_id: key_package_id(scope, "kp-1"),
                owner: owner.clone(),
                key_package: KeyPackage::new(b"conformance-key-package-1".to_vec()),
            }
        );

        let exhausted = service
            .claim_key_package(&owner)
            .expect("conformance: second claim request succeeds");
        assert!(
            exhausted.is_none(),
            "a consumed KeyPackage must not be claimable again"
        );
    }

    /// Durable implementations must preserve sequences, duplicate indexes,
    /// commit admission, and KeyPackage consumption across a restart.
    ///
    /// Skipped (after seeding state) when the harness reports it keeps no
    /// durable state.
    pub fn check_state_survives_restart<H: HttpDeliveryHarness>(harness: &mut H) {
        let scope = "conformance/restart";
        let owner = member(scope, "claimed-owner");
        let spare_owner = member(scope, "available-owner");
        let admission = HttpCommitAdmission {
            source_epoch: EpochId(9),
        };

        let service = harness.delivery();
        let first = group_message(scope, "msg-1", b"durable-first");
        service
            .publish(group_target(scope, None), first.clone())
            .expect("conformance: pre-restart publish is accepted");
        service
            .publish(
                group_target(scope, Some(admission)),
                group_message(scope, "commit-1", b"durable-commit"),
            )
            .expect("conformance: pre-restart commit is accepted");
        service
            .publish_key_package(HttpKeyPackagePublication {
                key_package_id: key_package_id(scope, "kp-claimed"),
                owner: owner.clone(),
                key_package: KeyPackage::new(b"durable-claimed".to_vec()),
            })
            .expect("conformance: pre-restart KeyPackage publication is accepted");
        service
            .claim_key_package(&owner)
            .expect("conformance: pre-restart claim succeeds")
            .expect("conformance: pre-restart package must be claimable");
        service
            .publish_key_package(HttpKeyPackagePublication {
                key_package_id: key_package_id(scope, "kp-available"),
                owner: spare_owner.clone(),
                key_package: KeyPackage::new(b"durable-available".to_vec()),
            })
            .expect("conformance: pre-restart spare KeyPackage publication is accepted");

        if !harness.restart() {
            return;
        }

        let service = harness.delivery();
        let page = service
            .sync_group(&group_id(scope), 0, 10)
            .expect("conformance: post-restart sync succeeds");
        assert_eq!(
            page.entries
                .iter()
                .map(|entry| (entry.seq, entry.message.id.clone()))
                .collect::<Vec<_>>(),
            vec![
                (1, message_id(scope, "msg-1")),
                (2, message_id(scope, "commit-1"))
            ],
            "ordered group log must survive restart"
        );

        let replay = service
            .publish(group_target(scope, None), first)
            .expect("conformance: post-restart exact duplicate replays");
        assert_eq!(replay.seq, 1, "duplicate index must survive restart");
        assert!(replay.duplicate);

        let err = service
            .publish(
                group_target(scope, Some(admission)),
                group_message(scope, "commit-2", b"durable-loser"),
            )
            .expect_err("conformance: commit admission must survive restart");
        assert_eq!(
            err,
            HttpServerError::StaleEpoch {
                source_epoch: EpochId(9)
            }
        );

        let consumed = service
            .claim_key_package(&owner)
            .expect("conformance: post-restart claim request succeeds");
        assert!(
            consumed.is_none(),
            "consumed KeyPackage state must survive restart"
        );
        let available = service
            .claim_key_package(&spare_owner)
            .expect("conformance: post-restart spare claim succeeds")
            .expect("conformance: available KeyPackage must survive restart");
        assert_eq!(
            available.key_package_id,
            key_package_id(scope, "kp-available")
        );
    }

    fn scoped(scope: &str, label: &str) -> Vec<u8> {
        format!("{scope}/{label}").into_bytes()
    }

    fn group_id(scope: &str) -> GroupId {
        GroupId::new(scoped(scope, "group"))
    }

    fn member(scope: &str, label: &str) -> MemberId {
        MemberId::new(scoped(scope, label))
    }

    fn message_id(scope: &str, label: &str) -> MessageId {
        MessageId::new(scoped(scope, label))
    }

    fn key_package_id(scope: &str, label: &str) -> HttpKeyPackageId {
        HttpKeyPackageId::new(scoped(scope, label))
    }

    fn group_target(
        scope: &str,
        commit_admission: Option<HttpCommitAdmission>,
    ) -> HttpPublishTarget {
        HttpPublishTarget::Group {
            group_id: group_id(scope),
            transport_group_id: scoped(scope, "transport-group"),
            commit_admission,
        }
    }

    fn group_message(scope: &str, label: &str, payload: &[u8]) -> TransportMessage {
        TransportMessage {
            id: message_id(scope, label),
            payload: payload.to_vec(),
            timestamp: Timestamp(42),
            causal_deps: Vec::new(),
            source: TransportSource(HTTP_SERVER_SOURCE.to_owned()),
            envelope: TransportEnvelope::GroupMessage {
                transport_group_id: scoped(scope, "transport-group"),
            },
        }
    }

    fn welcome_message(scope: &str, label: &str, recipient: MemberId) -> TransportMessage {
        TransportMessage {
            id: message_id(scope, label),
            payload: b"conformance-welcome-bytes".to_vec(),
            timestamp: Timestamp(43),
            causal_deps: Vec::new(),
            source: TransportSource(HTTP_SERVER_SOURCE.to_owned()),
            envelope: TransportEnvelope::Welcome { recipient },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_passes_group_ordering_and_duplicate_replay_conformance() {
        conformance::check_group_ordering_and_duplicate_replay(&mut HttpDeliveryService::default());
    }

    #[test]
    fn reference_passes_conflicting_duplicate_rejection_conformance() {
        conformance::check_conflicting_duplicate_rejection(&mut HttpDeliveryService::default());
    }

    #[test]
    fn reference_passes_commit_admission_conformance() {
        conformance::check_commit_admission_per_source_epoch(&mut HttpDeliveryService::default());
    }

    #[test]
    fn reference_passes_inbox_sequencing_conformance() {
        conformance::check_inbox_sequencing(&mut HttpDeliveryService::default());
    }

    #[test]
    fn reference_passes_publish_target_envelope_conformance() {
        conformance::check_publish_target_envelope_match(&mut HttpDeliveryService::default());
    }

    #[test]
    fn reference_passes_key_package_lifecycle_conformance() {
        conformance::check_key_package_lifecycle(&mut HttpDeliveryService::default());
    }

    #[test]
    fn reference_passes_full_conformance_suite_on_one_service() {
        conformance::check_all(&mut HttpDeliveryService::default());
    }

    #[test]
    fn check_publish_agrees_with_publish() {
        use finitechat_transport::transport::{Timestamp, TransportEnvelope, TransportSource};

        let mut service = HttpDeliveryService::default();
        let target = HttpPublishTarget::Group {
            group_id: GroupId::new(b"dry-run-group".to_vec()),
            transport_group_id: b"dry-run-transport".to_vec(),
            commit_admission: Some(HttpCommitAdmission {
                source_epoch: EpochId(3),
            }),
        };
        let message = TransportMessage {
            id: MessageId::new(b"dry-run-msg-1".to_vec()),
            payload: b"dry-run-payload".to_vec(),
            timestamp: Timestamp(42),
            causal_deps: Vec::new(),
            source: TransportSource(HTTP_SERVER_SOURCE.to_owned()),
            envelope: TransportEnvelope::GroupMessage {
                transport_group_id: b"dry-run-transport".to_vec(),
            },
        };

        let check = service
            .check_publish(&target, &message)
            .expect("fresh check");
        let receipt = service
            .publish(target.clone(), message.clone())
            .expect("fresh check guarantees publish succeeds");
        assert_eq!(
            check,
            HttpPublishCheck::Fresh(receipt.clone()),
            "the dry run must predict the exact publish receipt"
        );

        // Exact duplicate: the dry run reports the replay receipt the real
        // publish would return.
        assert_eq!(
            service
                .check_publish(&target, &message)
                .expect("duplicate check"),
            HttpPublishCheck::DuplicateReplay(HttpPublishReceipt {
                duplicate: true,
                ..receipt
            })
        );

        // Conflicting bytes under the same id fail the dry run the same way
        // the real publish would.
        let conflicting = TransportMessage {
            payload: b"changed".to_vec(),
            ..message.clone()
        };
        assert_eq!(
            service.check_publish(&target, &conflicting),
            Err(HttpServerError::ConflictingMessageId {
                message_id: message.id.clone()
            })
        );

        // A second commit for the same source epoch fails the dry run.
        let loser = TransportMessage {
            id: MessageId::new(b"dry-run-msg-2".to_vec()),
            ..message
        };
        assert_eq!(
            service.check_publish(&target, &loser),
            Err(HttpServerError::StaleEpoch {
                source_epoch: EpochId(3)
            })
        );
    }

    #[test]
    fn with_limits_overrides_queue_and_group_caps() {
        use finitechat_transport::transport::{Timestamp, TransportEnvelope, TransportSource};

        let mut service = HttpDeliveryService::with_limits(HttpDeliveryLimits {
            max_groups: 1,
            max_queue_entries_per_route: 2,
            ..HttpDeliveryLimits::default()
        });
        let message = |label: &str| TransportMessage {
            id: MessageId::new(label.as_bytes().to_vec()),
            payload: b"limit-payload".to_vec(),
            timestamp: Timestamp(1),
            causal_deps: Vec::new(),
            source: TransportSource(HTTP_SERVER_SOURCE.to_owned()),
            envelope: TransportEnvelope::GroupMessage {
                transport_group_id: b"limit-transport".to_vec(),
            },
        };
        let target = HttpPublishTarget::Group {
            group_id: GroupId::new(b"limit-group".to_vec()),
            transport_group_id: b"limit-transport".to_vec(),
            commit_admission: None,
        };

        service
            .publish(target.clone(), message("limit-1"))
            .expect("first entry fits");
        service
            .publish(target.clone(), message("limit-2"))
            .expect("second entry fits");
        assert_eq!(
            service.publish(target, message("limit-3")),
            Err(HttpServerError::QueueFull {
                plane: HttpDeliveryPlane::Group,
                max: 2
            })
        );

        let second_group = HttpPublishTarget::Group {
            group_id: GroupId::new(b"limit-group-2".to_vec()),
            transport_group_id: b"limit-transport".to_vec(),
            commit_admission: None,
        };
        assert_eq!(
            service.publish(second_group, message("limit-4")),
            Err(HttpServerError::GroupLimitExceeded { max: 1 })
        );
    }
}
