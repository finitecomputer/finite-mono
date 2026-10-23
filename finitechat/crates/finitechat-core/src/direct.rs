//! Direct room engine: Finite Chat without MLS.
//!
//! In direct transport the agent keeps the one room log in plaintext and the
//! web dashboard reaches it through the agent's authenticated ingress. The log
//! carries the same application events an MLS Device stores after decryption,
//! and this engine projects them with the same `ChatProjectionState`. A log
//! folded from an existing Device therefore shows the same topics, chats,
//! order, archive state and transcripts, by construction.
//!
//! The engine owns no storage, clock or network. A [`DirectLog`] appends each
//! accepted event and assigns its sequence number, message id and timestamp.
//! Action handlers mirror the MLS runtime's handlers and reuse its payload
//! encoders.

use super::*;

/// Device id of a person's web session in direct transport. Today's hosted
/// device uses the same id, so senders keep their Hermes-visible identity.
pub const DIRECT_WEB_DEVICE_ID: &str = "hosted-web";

/// One event to append. The log assigns sequence, message id and time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectEventDraft {
    pub room_id: String,
    pub sender: DeviceRef,
    pub plaintext: Vec<u8>,
    /// Stable key for an event that must be written at most once per room
    /// (the home topic). `None` for ordinary events.
    pub idempotency_key: Option<String>,
}

/// The durable, ordered room log the engine writes through.
pub trait DirectLog {
    /// Persist one event and return it as stored. An idempotency key that
    /// was already used returns the original event instead of a new one.
    fn append(&mut self, draft: DirectEventDraft) -> Result<StoredAppEvent, FiniteChatCoreError>;
    /// A fresh random object id, e.g. `topic-<16 hex>`.
    fn generate_object_id(&mut self, prefix: &str) -> Result<String, FiniteChatCoreError>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectRoom {
    pub room_id: String,
    pub display_name: String,
    pub picture: Option<String>,
}

/// The requester claims a dashboard send carries, verified by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectRequesterContext {
    pub email: String,
    pub sites_assertion: String,
}

/// Where a handled action leaves the viewer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirectSelection {
    pub room_id: Option<String>,
    pub topic_id: Option<String>,
    pub chat_id: Option<String>,
    /// Older transcript anchor for `LoadOlderMessages`.
    pub oldest_message_id: Option<String>,
    pub status: String,
}

/// The view the person last opened. A view naming no room resumes it, as the
/// Device's saved navigation did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectLastView {
    pub room_id: String,
    pub topic_id: Option<String>,
    pub chat_id: Option<String>,
}

#[derive(Clone, Debug)]
struct DirectActivity {
    member: AppTypingMember,
    expires_at_ms: u64,
}

/// Per-chat and per-room digests of what a person sees, for comparing a
/// folded direct log with the hosted device it replaces. Hashes only; no
/// transcript text leaves the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectRoomDigest {
    pub room_id: String,
    pub digest: String,
    pub topics: Vec<DirectTopicDigest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectTopicDigest {
    pub topic_id: String,
    pub title_sha256: String,
    pub archived: bool,
    pub chats: Vec<DirectChatDigest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectChatDigest {
    pub chat_id: String,
    pub title_sha256: String,
    pub archived: bool,
    pub placement_topic_id: Option<String>,
    pub position: Option<String>,
    pub message_count: u32,
    pub digest: String,
}

pub struct DirectRoomEngine {
    /// The person whose view this projects: authorship is account-scoped.
    owner: DeviceRef,
    agent: DeviceRef,
    rooms: BTreeMap<String, DirectRoom>,
    projection: ChatProjectionState,
    local_read_seq: BTreeMap<String, u64>,
    profiles: BTreeMap<String, AppProfileSummary>,
    activities: BTreeMap<String, DirectActivity>,
    /// Running edits shown while a reply streams, keyed by (room, edited
    /// message). Never durable: the final edit replaces them in the log.
    drafts: BTreeMap<(String, String), ChatMessage>,
    max_seq: BTreeMap<String, u64>,
    rev: u64,
    /// Topic summaries for `rev`: each one walks every message, and a single
    /// action validates its route several times.
    topics_cache: std::sync::Mutex<Option<(u64, Vec<AppTopicSummary>)>>,
    last_view: Option<DirectLastView>,
}

impl DirectRoomEngine {
    pub fn new(owner: DeviceRef, agent: DeviceRef) -> Self {
        Self {
            owner,
            agent,
            rooms: BTreeMap::new(),
            projection: ChatProjectionState::default(),
            local_read_seq: BTreeMap::new(),
            profiles: BTreeMap::new(),
            activities: BTreeMap::new(),
            drafts: BTreeMap::new(),
            max_seq: BTreeMap::new(),
            rev: 0,
            topics_cache: std::sync::Mutex::new(None),
            last_view: None,
        }
    }

    pub fn last_view(&self) -> Option<&DirectLastView> {
        self.last_view.as_ref()
    }

    pub fn set_last_view(&mut self, view: Option<DirectLastView>) {
        self.last_view = view;
    }

    fn remember_view(&mut self, selection: &DirectSelection) {
        if let Some(room_id) = &selection.room_id {
            self.last_view = Some(DirectLastView {
                room_id: room_id.clone(),
                topic_id: selection.topic_id.clone(),
                chat_id: selection.chat_id.clone(),
            });
        }
    }

    pub fn owner(&self) -> &DeviceRef {
        &self.owner
    }

    pub fn agent(&self) -> &DeviceRef {
        &self.agent
    }

    pub fn rev(&self) -> u64 {
        self.rev
    }

    pub fn room_ids(&self) -> Vec<String> {
        self.rooms.keys().cloned().collect()
    }

    pub fn has_room(&self, room_id: &str) -> bool {
        self.rooms.contains_key(room_id)
    }

    pub fn max_seq(&self, room_id: &str) -> u64 {
        self.max_seq.get(room_id).copied().unwrap_or_default()
    }

    pub fn upsert_room(&mut self, room: DirectRoom) {
        self.rooms.insert(room.room_id.clone(), room);
        self.rev += 1;
    }

    pub fn upsert_profile(&mut self, profile: AppProfileSummary) {
        self.profiles.insert(profile.account_id.clone(), profile);
        self.rev += 1;
    }

    pub fn set_read_seq(&mut self, room_id: &str, seq: u64) {
        self.local_read_seq.insert(room_id.to_owned(), seq);
        self.rev += 1;
    }

    pub fn read_seq(&self, room_id: &str) -> u64 {
        self.local_read_seq
            .get(room_id)
            .copied()
            .unwrap_or_default()
    }

    /// Project one stored event. Replay and live appends use this alone.
    pub fn apply(&mut self, event: StoredAppEvent) {
        let room_id = event.room_id.clone();
        let seq = event.seq;
        if is_device_link_control_event(&event.plaintext) {
            return;
        }
        let parsed = ParsedAppEvent::decode(event);
        if matches!(parsed.decoded, DecodedAppEvent::ChatMessage(_)) {
            self.clear_activities_for_sender(&room_id, &parsed.event.sender);
            if let Some((message, _)) = project_chat_message(
                room_id.clone(),
                seq,
                parsed.event.message_id.clone(),
                parsed.event.sender.clone(),
                parsed.event.plaintext.clone(),
                parsed.event.timestamp_unix_seconds,
                &self.owner,
            ) && let Some(target) = message.edit_of_message_id
            {
                self.drafts.remove(&(room_id.clone(), target));
            }
        }
        self.projection.apply_parsed_event(parsed, &self.owner);
        let max = self.max_seq.entry(room_id).or_default();
        *max = (*max).max(seq);
        self.rev += 1;
    }

    /// Keep a running edit of an earlier message as an ephemeral draft
    /// instead of a durable event, so a streamed reply costs two log rows
    /// (its first part and its final edit). Returns the edited message's id
    /// and sequence when `plaintext` is such an edit; `None` means append it.
    pub fn set_draft(
        &mut self,
        room_id: &str,
        sender: &DeviceRef,
        plaintext: &[u8],
        now_unix_seconds: u64,
    ) -> Option<(String, u64)> {
        let (mut draft, _) = project_chat_message(
            room_id.to_owned(),
            0,
            String::new(),
            sender.clone(),
            plaintext.to_vec(),
            now_unix_seconds,
            &self.owner,
        )?;
        if draft.status != ChatMessageStatus::Running {
            return None;
        }
        let target_id = draft.edit_of_message_id.clone()?;
        let target = self.projection.message(room_id, &target_id)?;
        draft.seq = target.seq;
        draft.message_id = format!("draft-{target_id}");
        draft.conversation_id = target.conversation_id.clone();
        draft.chat_id = target.chat_id.clone();
        let seq = target.seq;
        self.drafts
            .insert((room_id.to_owned(), target_id.clone()), draft);
        self.rev += 1;
        Some((target_id, seq))
    }

    pub fn topics(&self) -> Vec<AppTopicSummary> {
        let mut cache = self
            .topics_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((rev, topics)) = cache.as_ref()
            && *rev == self.rev
        {
            return topics.clone();
        }
        let topics = self.projection.topics(&self.local_read_seq);
        *cache = Some((self.rev, topics.clone()));
        topics
    }

    pub fn message(&self, room_id: &str, message_id: &str) -> Option<&ChatMessage> {
        self.projection.message(room_id, message_id)
    }

    pub fn attachment_reference(
        &self,
        room_id: &str,
        message_id: &str,
        attachment_id: &str,
    ) -> Option<AttachmentBlobReferenceV1> {
        let message = self.projection.message(room_id, message_id)?;
        self.projection
            .attachment_blob_map()
            .get(&message_key(message))?
            .get(attachment_id)
            .cloned()
    }

    /// Record ephemeral agent activity (typing, thinking, working).
    pub fn apply_activity(
        &mut self,
        input: &AppBridgeActivityInput,
        sender: &DeviceRef,
        now_ms: u64,
    ) -> AppAcceptedActivity {
        let route_key = format!(
            "{}\0{}\0{}\0{}\0{}",
            input.room_id,
            input.conversation_id.as_deref().unwrap_or_default(),
            input.segment_id.as_deref().unwrap_or_default(),
            input.activity_kind,
            input.activity_id.as_deref().unwrap_or_default()
        );
        match input.action {
            EphemeralActivityActionV1::Set => {
                let profile_name = self
                    .profiles
                    .get(&sender.account_id)
                    .map(|profile| profile.display_name.trim().to_owned())
                    .filter(|name| !name.is_empty());
                self.activities.insert(
                    route_key.clone(),
                    DirectActivity {
                        member: AppTypingMember {
                            room_id: input.room_id.clone(),
                            topic_id: input.conversation_id.clone(),
                            chat_id: input.segment_id.clone(),
                            account_id: sender.account_id.clone(),
                            device_id: sender.device_id.clone(),
                            display_name: sender_display_name(
                                sender,
                                profile_name.as_deref(),
                                false,
                            ),
                            picture: self
                                .profiles
                                .get(&sender.account_id)
                                .and_then(|profile| profile.picture.clone()),
                            npub: npub_encode(&sender.account_id).ok(),
                            activity_kind: input.activity_kind.clone(),
                        },
                        expires_at_ms: now_ms.saturating_add(input.expires_in_millis),
                    },
                );
            }
            EphemeralActivityActionV1::Clear => {
                self.activities.remove(&route_key);
            }
        }
        self.rev += 1;
        AppAcceptedActivity {
            route_key,
            cached_events_for_route: 1,
        }
    }

    /// Drop expired activity. Returns true if anything changed.
    pub fn expire_activities(&mut self, now_ms: u64) -> bool {
        let before = self.activities.len();
        self.activities
            .retain(|_, activity| activity.expires_at_ms > now_ms);
        let changed = self.activities.len() != before;
        if changed {
            self.rev += 1;
        }
        changed
    }

    fn clear_activities_for_sender(&mut self, room_id: &str, sender: &DeviceRef) {
        let before = self.activities.len();
        self.activities.retain(|_, activity| {
            !(activity.member.room_id == room_id
                && activity.member.account_id == sender.account_id
                && activity.member.device_id == sender.device_id
                && is_chat_live_indicator_activity(&activity.member.activity_kind))
        });
        if self.activities.len() != before {
            self.rev += 1;
        }
    }

    fn topic_exists(&self, room_id: &str, topic_id: &str) -> bool {
        self.topics()
            .iter()
            .any(|topic| topic.room_id == room_id && topic.topic_id == topic_id && !topic.archived)
    }

    fn chat_exists(&self, room_id: &str, topic_id: &str, chat_id: &str) -> bool {
        self.topics()
            .iter()
            .find(|topic| topic.room_id == room_id && topic.topic_id == topic_id && !topic.archived)
            .is_some_and(|topic| topic.chats.iter().any(|chat| chat.chat_id == chat_id))
    }

    fn default_chat_id_for_topic(&self, room_id: &str, topic_id: &str) -> Option<String> {
        default_chat_id_for_topic_in(&self.topics(), room_id, topic_id)
    }

    fn validate_room(&self, room_id: &str) -> Result<(), FiniteChatCoreError> {
        if self.rooms.contains_key(room_id) {
            Ok(())
        } else {
            Err(client_error_message(format!("room not found: {room_id}")))
        }
    }

    fn validate_topic(&self, room_id: &str, topic_id: &str) -> Result<(), FiniteChatCoreError> {
        if self.topic_exists(room_id, topic_id) {
            Ok(())
        } else {
            Err(client_error_message(format!(
                "topic '{topic_id}' is not available in room '{room_id}'"
            )))
        }
    }

    fn validate_chat_route(
        &self,
        room_id: &str,
        topic_id: &str,
        chat_id: &str,
    ) -> Result<(), FiniteChatCoreError> {
        self.validate_topic(room_id, topic_id)?;
        if self.chat_exists(room_id, topic_id, chat_id) {
            Ok(())
        } else {
            Err(client_error_message(format!(
                "chat '{chat_id}' is not available in topic '{topic_id}' in room '{room_id}'"
            )))
        }
    }

    fn append(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
        sender: DeviceRef,
        plaintext: Vec<u8>,
        idempotency_key: Option<String>,
    ) -> Result<StoredAppEvent, FiniteChatCoreError> {
        let event = log.append(DirectEventDraft {
            room_id: room_id.to_owned(),
            sender,
            plaintext,
            idempotency_key,
        })?;
        self.apply(event.clone());
        Ok(event)
    }

    /// Create the room's Home topic once, as the MLS runtime does when a
    /// Device pairs with its agent.
    pub fn ensure_home_topic(
        &mut self,
        room_id: &str,
        log: &mut dyn DirectLog,
    ) -> Result<(), FiniteChatCoreError> {
        self.validate_room(room_id)?;
        if self.topic_exists(room_id, HOME_TOPIC_ID) {
            return Ok(());
        }
        let metadata = ConversationMetadataV1 {
            title: Some(HOME_TOPIC_TITLE.to_owned()),
            description: None,
            external_topic: None,
            skill_binding: None,
        };
        metadata.validate_limits().map_err(client_error)?;
        let payload = serde_json::to_vec(&metadata).map_err(client_error)?;
        let plaintext = encode_application_event(
            DurableAppEventKind::ConversationCreate,
            Some(HOME_TOPIC_ID.to_owned()),
            &payload,
        )?;
        let sender = self.agent.clone();
        let key = home_topic_idempotency_key(room_id, &sender);
        self.append(log, room_id, sender, plaintext, Some(key))?;
        Ok(())
    }

    fn append_topic_chat_with_id(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
        topic_id: &str,
        reason: Option<String>,
        segment_id: String,
    ) -> Result<String, FiniteChatCoreError> {
        self.validate_topic(room_id, topic_id)?;
        let reason = reason
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let segment = ConversationSegmentStartV1 { segment_id, reason };
        segment.validate_limits().map_err(client_error)?;
        let payload = serde_json::to_vec(&segment).map_err(client_error)?;
        let plaintext = encode_application_event(
            DurableAppEventKind::ConversationSegmentStart,
            Some(topic_id.to_owned()),
            &payload,
        )?;
        let sender = self.owner.clone();
        self.append(log, room_id, sender, plaintext, None)?;
        Ok(segment.segment_id)
    }

    fn append_topic_chat(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
        topic_id: &str,
        reason: Option<String>,
    ) -> Result<String, FiniteChatCoreError> {
        self.validate_topic(room_id, topic_id)?;
        let segment_id = log.generate_object_id("segment")?;
        self.append_topic_chat_with_id(log, room_id, topic_id, reason, segment_id)
    }

    fn append_namespaced(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
        name: &str,
        topic_id: &str,
        chat_id: &str,
        payload: Vec<u8>,
    ) -> Result<(), FiniteChatCoreError> {
        let plaintext = encode_application_event_with_segment(
            DurableAppEventKind::Namespaced {
                name: name.to_owned(),
                policy: ApplicationDeliveryPolicy::NON_NOTIFYING,
            },
            Some(topic_id.to_owned()),
            Some(chat_id.to_owned()),
            &payload,
        )?;
        let sender = self.owner.clone();
        self.append(log, room_id, sender, plaintext, None)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn send_text(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
        topic_id: &str,
        chat_id: &str,
        text: &str,
        metadata_json: Option<&str>,
        requester: Option<&DirectRequesterContext>,
    ) -> Result<(), FiniteChatCoreError> {
        self.validate_chat_route(room_id, topic_id, chat_id)?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let requester = requester.map(|requester| VerifiedRequesterContext {
            email: requester.email.clone(),
            sites_assertion: requester.sites_assertion.clone(),
        });
        let payload = encode_text_message_payload_scoped(
            trimmed,
            None,
            Some(topic_id),
            Some(chat_id),
            metadata_json,
            requester.as_ref(),
        )?;
        let plaintext = encode_application_event_with_segment(
            DurableAppEventKind::ChatMessage,
            Some(topic_id.to_owned()),
            Some(chat_id.to_owned()),
            &payload,
        )?;
        let sender = self.owner.clone();
        self.append(log, room_id, sender, plaintext, None)?;
        Ok(())
    }

    fn default_chat_route_for_room(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
    ) -> Result<(String, String), FiniteChatCoreError> {
        self.ensure_home_topic(room_id, log)?;
        let chat_id = self
            .default_chat_id_for_topic(room_id, HOME_TOPIC_ID)
            .unwrap_or_else(|| HOME_CHAT_ID.to_owned());
        Ok((HOME_TOPIC_ID.to_owned(), chat_id))
    }

    fn topic_chat_or_start(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
        topic_id: &str,
    ) -> Result<String, FiniteChatCoreError> {
        self.validate_topic(room_id, topic_id)?;
        if let Some(chat_id) = self.default_chat_id_for_topic(room_id, topic_id) {
            return Ok(chat_id);
        }
        self.append_topic_chat(log, room_id, topic_id, None)?;
        self.default_chat_id_for_topic(room_id, topic_id)
            .ok_or_else(|| {
                client_error_message(format!(
                    "topic '{topic_id}' in room '{room_id}' has no available chat"
                ))
            })
    }

    /// Handle one action from the person this engine projects for.
    pub fn execute(
        &mut self,
        action: AppAction,
        requester: Option<&DirectRequesterContext>,
        log: &mut dyn DirectLog,
    ) -> Result<DirectSelection, FiniteChatCoreError> {
        let navigates = matches!(
            action,
            AppAction::OpenRoom { .. }
                | AppAction::OpenTopic { .. }
                | AppAction::OpenChat { .. }
                | AppAction::CreateTopic { .. }
                | AppAction::StartTopicChat { .. }
        );
        let selection = self.execute_action(action, requester, log)?;
        if navigates {
            self.remember_view(&selection);
        }
        Ok(selection)
    }

    fn execute_action(
        &mut self,
        action: AppAction,
        requester: Option<&DirectRequesterContext>,
        log: &mut dyn DirectLog,
    ) -> Result<DirectSelection, FiniteChatCoreError> {
        let selection = |room_id: String,
                         topic_id: Option<String>,
                         chat_id: Option<String>,
                         status: &str| DirectSelection {
            room_id: Some(room_id),
            topic_id,
            chat_id,
            oldest_message_id: None,
            status: status.to_owned(),
        };
        match action {
            AppAction::StartRuntime | AppAction::RefreshDevices => Ok(DirectSelection {
                status: "connected".to_owned(),
                ..DirectSelection::default()
            }),
            AppAction::OpenRoom { room_id } => {
                self.validate_room(&room_id)?;
                Ok(selection(room_id, None, None, "room opened"))
            }
            AppAction::OpenTopic { room_id, topic_id } => {
                self.validate_topic(&room_id, &topic_id)?;
                let chat_id = self.default_chat_id_for_topic(&room_id, &topic_id);
                Ok(selection(room_id, Some(topic_id), chat_id, "topic opened"))
            }
            AppAction::OpenChat {
                room_id,
                topic_id,
                chat_id,
            } => {
                self.validate_chat_route(&room_id, &topic_id, &chat_id)?;
                Ok(selection(
                    room_id,
                    Some(topic_id),
                    Some(chat_id),
                    "chat opened",
                ))
            }
            AppAction::CreateTopic { room_id, title } => {
                self.validate_room(&room_id)?;
                let title =
                    normalize_bounded_non_empty_string("topic title", &title, MAX_OBJECT_ID_BYTES)?;
                let topic_id = log.generate_object_id("topic")?;
                let metadata = ConversationMetadataV1 {
                    title: Some(title),
                    description: None,
                    external_topic: None,
                    skill_binding: None,
                };
                metadata.validate_limits().map_err(client_error)?;
                let payload = serde_json::to_vec(&metadata).map_err(client_error)?;
                let plaintext = encode_application_event(
                    DurableAppEventKind::ConversationCreate,
                    Some(topic_id.clone()),
                    &payload,
                )?;
                let sender = self.owner.clone();
                self.append(log, &room_id, sender, plaintext, None)?;
                let chat_id = self.append_topic_chat(log, &room_id, &topic_id, None)?;
                Ok(selection(
                    room_id,
                    Some(topic_id),
                    Some(chat_id),
                    "topic created",
                ))
            }
            AppAction::StartTopicChat {
                room_id,
                topic_id,
                reason,
            } => {
                let chat_id = self.append_topic_chat(log, &room_id, &topic_id, reason)?;
                Ok(selection(
                    room_id,
                    Some(topic_id),
                    Some(chat_id),
                    "chat created",
                ))
            }
            AppAction::RenameChat {
                room_id,
                topic_id,
                chat_id,
                title,
            } => {
                self.validate_chat_route(&room_id, &topic_id, &chat_id)?;
                let title =
                    normalize_bounded_non_empty_string("chat title", &title, MAX_CHAT_TITLE_BYTES)?;
                let rename = ChatRenameV1 {
                    topic_id: topic_id.clone(),
                    chat_id: chat_id.clone(),
                    title,
                };
                rename.validate_limits().map_err(client_error)?;
                let payload = serde_json::to_vec(&rename).map_err(client_error)?;
                self.append_namespaced(
                    log,
                    &room_id,
                    FINITECHAT_CHAT_RENAME_EVENT_V1,
                    &topic_id,
                    &chat_id,
                    payload,
                )?;
                Ok(selection(
                    room_id,
                    Some(topic_id),
                    Some(chat_id),
                    "chat renamed",
                ))
            }
            AppAction::SetChatArchived {
                room_id,
                topic_id,
                chat_id,
                archived,
            } => {
                self.validate_chat_route(&room_id, &topic_id, &chat_id)?;
                let archive = ChatArchiveV1 {
                    topic_id: topic_id.clone(),
                    chat_id: chat_id.clone(),
                    archived,
                };
                archive.validate_limits().map_err(client_error)?;
                let payload = serde_json::to_vec(&archive).map_err(client_error)?;
                self.append_namespaced(
                    log,
                    &room_id,
                    FINITECHAT_CHAT_ARCHIVE_EVENT_V1,
                    &topic_id,
                    &chat_id,
                    payload,
                )?;
                let status = if archived {
                    "chat archived"
                } else {
                    "chat restored"
                };
                Ok(selection(room_id, Some(topic_id), Some(chat_id), status))
            }
            AppAction::MoveChat {
                room_id,
                topic_id,
                chat_id,
                destination_topic_id,
                before,
            } => {
                self.move_chat(
                    log,
                    &room_id,
                    &topic_id,
                    &chat_id,
                    destination_topic_id,
                    before,
                )?;
                Ok(selection(
                    room_id,
                    Some(topic_id),
                    Some(chat_id),
                    "chat moved",
                ))
            }
            AppAction::SendMessage {
                room_id,
                text,
                metadata_json,
            } => {
                let (topic_id, chat_id) = self.default_chat_route_for_room(log, &room_id)?;
                self.send_text(
                    log,
                    &room_id,
                    &topic_id,
                    &chat_id,
                    &text,
                    metadata_json.as_deref(),
                    requester,
                )?;
                Ok(selection(room_id, Some(topic_id), Some(chat_id), "sent"))
            }
            AppAction::SendTopicMessage {
                room_id,
                topic_id,
                text,
                metadata_json,
            } => {
                let chat_id = self.topic_chat_or_start(log, &room_id, &topic_id)?;
                self.send_text(
                    log,
                    &room_id,
                    &topic_id,
                    &chat_id,
                    &text,
                    metadata_json.as_deref(),
                    requester,
                )?;
                Ok(selection(room_id, Some(topic_id), Some(chat_id), "sent"))
            }
            AppAction::SendChatMessage {
                room_id,
                topic_id,
                chat_id,
                text,
                metadata_json,
            } => {
                self.send_text(
                    log,
                    &room_id,
                    &topic_id,
                    &chat_id,
                    &text,
                    metadata_json.as_deref(),
                    requester,
                )?;
                Ok(selection(room_id, Some(topic_id), Some(chat_id), "sent"))
            }
            AppAction::LoadOlderMessages {
                room_id,
                before_message_id,
                limit,
            } => self.load_older(&room_id, &before_message_id, limit),
            AppAction::MarkRoomRead { room_id } => {
                self.validate_room(&room_id)?;
                let seq = self.max_seq(&room_id);
                self.set_read_seq(&room_id, seq);
                Ok(selection(room_id, None, None, "read"))
            }
            AppAction::SetTyping { room_id, .. } => {
                self.validate_room(&room_id)?;
                Ok(selection(room_id, None, None, "typing"))
            }
            other => Err(FiniteChatCoreError::Client {
                reason: format!(
                    "{} is not available in direct transport",
                    app_action_name(&other)
                ),
            }),
        }
    }

    fn start_topic_chat_intent(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
        topic_id: &str,
        reason: Option<String>,
        intent_key: &str,
    ) -> Result<String, FiniteChatCoreError> {
        let intent_key = intent_key.trim();
        validate_string_bytes("new chat intent key", intent_key, MAX_OBJECT_ID_BYTES)
            .map_err(client_error)?;
        if intent_key.is_empty() {
            return Err(client_error_message(
                "new chat intent key must not be empty".to_owned(),
            ));
        }
        let chat_id = new_chat_intent_id(room_id, topic_id, intent_key);
        if !self.chat_exists(room_id, topic_id, &chat_id) {
            self.append_topic_chat_with_id(log, room_id, topic_id, reason, chat_id.clone())?;
        }
        Ok(chat_id)
    }

    /// The deterministic new-chat route the hosted device's `/v1/app/new-chat`
    /// uses, with its canonical-room checks left to the caller.
    pub fn new_chat(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
        topic_id: &str,
        reason: Option<String>,
        intent_key: &str,
    ) -> Result<DirectSelection, FiniteChatCoreError> {
        let chat_id = self.start_topic_chat_intent(log, room_id, topic_id, reason, intent_key)?;
        let selection = DirectSelection {
            room_id: Some(room_id.to_owned()),
            topic_id: Some(topic_id.to_owned()),
            chat_id: Some(chat_id),
            oldest_message_id: None,
            status: "chat created".to_owned(),
        };
        self.remember_view(&selection);
        Ok(selection)
    }

    fn move_chat(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
        topic_id: &str,
        chat_id: &str,
        destination_topic_id: String,
        before: Option<ChatAddress>,
    ) -> Result<(), FiniteChatCoreError> {
        self.validate_chat_route(room_id, topic_id, chat_id)?;
        let invalid = || {
            client_error_message(
                "The chats changed. Refresh and try moving the chat again.".to_owned(),
            )
        };
        if !self.topic_exists(room_id, &destination_topic_id) {
            return Err(invalid());
        }
        let source = ChatAddress {
            topic_id: topic_id.to_owned(),
            chat_id: chat_id.to_owned(),
        };
        let topics = self.topics();
        let mut destination = Vec::new();
        for topic in topics
            .iter()
            .filter(|topic| topic.room_id == room_id && !topic.archived)
        {
            for chat in &topic.chats {
                let address = ChatAddress {
                    topic_id: topic.topic_id.clone(),
                    chat_id: chat.chat_id.clone(),
                };
                if address == source {
                    if chat.archived {
                        return Err(invalid());
                    }
                    continue;
                }
                let placement = chat.placement.as_ref().ok_or_else(invalid)?;
                if !chat.archived && placement.topic_id == destination_topic_id {
                    destination.push((address, placement.position.as_str()));
                }
            }
        }
        destination.sort_by(|a, b| a.1.cmp(b.1));
        let index = match before {
            Some(before) => destination
                .iter()
                .position(|(address, _)| address == &before)
                .ok_or_else(invalid)?,
            None => destination.len(),
        };
        let lower = index
            .checked_sub(1)
            .map(|index| destination[index].1)
            .unwrap_or("");
        let upper = destination.get(index).map(|(_, position)| *position);
        let position =
            chat_organization::position_between_for_chat(lower, upper, topic_id, chat_id)
                .ok_or_else(|| {
                    client_error_message(
                    "This position is too crowded. Move the chat to a different position first."
                        .to_owned(),
                )
                })?;
        let placement = ChatPlacementV1 {
            topic_id: topic_id.to_owned(),
            chat_id: chat_id.to_owned(),
            destination_topic_id,
            position,
        };
        placement.validate_limits().map_err(client_error)?;
        let payload = serde_json::to_vec(&placement).map_err(client_error)?;
        self.append_namespaced(
            log,
            room_id,
            FINITECHAT_CHAT_PLACEMENT_EVENT_V1,
            topic_id,
            chat_id,
            payload,
        )
    }

    fn load_older(
        &self,
        room_id: &str,
        before_message_id: &str,
        limit: u32,
    ) -> Result<DirectSelection, FiniteChatCoreError> {
        self.validate_room(room_id)?;
        let anchor = self
            .projection
            .message(room_id, before_message_id.trim())
            .ok_or_else(|| {
                client_error_message("transcript anchor is not available in this chat".to_owned())
            })?;
        let (topic_id, chat_id) = (anchor.conversation_id.clone(), anchor.chat_id.clone());
        let window = match (topic_id.as_deref(), chat_id.as_deref()) {
            (Some(topic), Some(chat)) => {
                self.projection
                    .messages_for_chat_window(room_id, topic, chat, MAX_APP_MESSAGES)
            }
            _ => self
                .projection
                .messages_for_room_window(room_id, MAX_APP_MESSAGES),
        };
        let position = window
            .iter()
            .position(|message| message.message_id == anchor.message_id)
            .unwrap_or_default();
        let page = (limit.clamp(1, MAX_TRANSCRIPT_PAGE_SIZE)) as usize;
        let oldest = window
            .get(position.saturating_sub(page))
            .map(|message| message.message_id.clone());
        Ok(DirectSelection {
            room_id: Some(room_id.to_owned()),
            topic_id,
            chat_id,
            oldest_message_id: oldest,
            status: "older messages loaded".to_owned(),
        })
    }

    /// Append an attachment message whose blobs the caller has stored.
    #[allow(clippy::too_many_arguments)]
    pub fn send_attachments(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
        topic_id: Option<&str>,
        chat_id: Option<&str>,
        caption: &str,
        attachments: Vec<HermesAttachmentV1>,
        reply_to_message_id: Option<&str>,
    ) -> Result<DirectSelection, FiniteChatCoreError> {
        self.validate_room(room_id)?;
        let (topic_id, chat_id) = match (topic_id, chat_id) {
            (Some(topic_id), Some(chat_id)) => {
                self.validate_chat_route(room_id, topic_id, chat_id)?;
                (topic_id.to_owned(), chat_id.to_owned())
            }
            (None, None) => self.default_chat_route_for_room(log, room_id)?,
            _ => {
                return Err(client_error_message(
                    "attachment route must include both topic and chat ids".to_owned(),
                ));
            }
        };
        if attachments.is_empty() {
            return Err(client_error_message(
                "attachment message must include at least one attachment".to_owned(),
            ));
        }
        let reply_to_message_id = reply_to_message_id
            .map(str::trim)
            .filter(|id| !id.is_empty());
        if let Some(target) = reply_to_message_id
            && self.projection.message(room_id, target).is_none()
        {
            return Err(client_error_message(format!(
                "reply target '{target}' is not available in room '{room_id}'"
            )));
        }
        let payload = encode_attachment_message_payload(
            caption.trim(),
            attachments,
            reply_to_message_id,
            Some(&topic_id),
            Some(&chat_id),
        )?;
        let plaintext = encode_application_event_with_segment(
            DurableAppEventKind::ChatMessage,
            Some(topic_id.clone()),
            Some(chat_id.clone()),
            &payload,
        )?;
        let sender = self.owner.clone();
        self.append(log, room_id, sender, plaintext, None)?;
        Ok(DirectSelection {
            room_id: Some(room_id.to_owned()),
            topic_id: Some(topic_id),
            chat_id: Some(chat_id),
            oldest_message_id: None,
            status: "sent".to_owned(),
        })
    }

    /// Append an encoded event from the agent (a Hermes reply, a runtime
    /// command result). Unscoped chat messages go to the default Home chat,
    /// as on the MLS runtime.
    pub fn append_agent_event(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
        plaintext: Vec<u8>,
    ) -> Result<StoredAppEvent, FiniteChatCoreError> {
        self.validate_room(room_id)?;
        let plaintext = self.scope_unscoped_chat_event(log, room_id, plaintext)?;
        let sender = self.agent.clone();
        self.append(log, room_id, sender, plaintext, None)
    }

    /// Append an event a person's web session sent that is not a chat
    /// action (a runtime command request).
    pub fn append_owner_event(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
        plaintext: Vec<u8>,
    ) -> Result<StoredAppEvent, FiniteChatCoreError> {
        self.validate_room(room_id)?;
        let sender = self.owner.clone();
        self.append(log, room_id, sender, plaintext, None)
    }

    fn scope_unscoped_chat_event(
        &mut self,
        log: &mut dyn DirectLog,
        room_id: &str,
        plaintext: Vec<u8>,
    ) -> Result<Vec<u8>, FiniteChatCoreError> {
        let DecodedAppEvent::ChatMessage(decoded) = decode_application_event(&plaintext) else {
            return Ok(plaintext);
        };
        let projection = chat_projection_payload(&decoded.payload);
        let has_route = [
            decoded
                .conversation_id
                .as_deref()
                .or(projection.conversation_id.as_deref()),
            decoded
                .segment_id
                .as_deref()
                .or(projection.chat_id.as_deref()),
        ]
        .into_iter()
        .flatten()
        .any(|value| !value.trim().is_empty());
        if has_route {
            return Ok(plaintext);
        }
        let (topic_id, chat_id) = self.default_chat_route_for_room(log, room_id)?;
        encode_application_event_with_segment(
            DurableAppEventKind::ChatMessage,
            Some(topic_id),
            Some(chat_id),
            &decoded.raw_payload,
        )
    }

    /// The person's view of one room, topic or chat, as the hosted device
    /// renders `GET /v1/app/state` for the same query.
    pub fn state_for_view(
        &self,
        view: AppView,
        status: &str,
        now_ms: u64,
    ) -> Result<AppState, FiniteChatCoreError> {
        let topics = self.topics();
        let mut state = self.base_state(topics.clone(), status, now_ms);
        let Some(room_id) = view.room_id else {
            if view.topic_id.is_some()
                || view.chat_id.is_some()
                || view.limit.is_some()
                || view.oldest_message_id.is_some()
            {
                return Err(client_error_message(
                    "a transcript view requires room_id".to_owned(),
                ));
            }
            return match self.resumed_view(&topics) {
                Some(resumed) => self.state_for_view(resumed, status, now_ms),
                None => Ok(state),
            };
        };
        self.validate_room(&room_id).map_err(|_| {
            client_error_message("transcript room is not available on this Device".to_owned())
        })?;
        if view.chat_id.is_some() && view.topic_id.is_none() {
            return Err(client_error_message(
                "a chat view requires topic_id".to_owned(),
            ));
        }
        if view.oldest_message_id.is_some() && (view.topic_id.is_none() || view.chat_id.is_none()) {
            return Err(client_error_message(
                "an anchored transcript requires topic_id and chat_id".to_owned(),
            ));
        }
        let available_topic = |topic_id: &str| {
            topics.iter().any(|topic| {
                topic.room_id == room_id && topic.topic_id == topic_id && !topic.archived
            })
        };
        let topic_id = view
            .topic_id
            .or_else(|| available_topic(HOME_TOPIC_ID).then(|| HOME_TOPIC_ID.to_owned()));
        if let Some(topic_id) = &topic_id
            && !available_topic(topic_id)
        {
            return Err(client_error_message(
                "transcript topic is not available in this room".to_owned(),
            ));
        }
        let chat_id = view.chat_id.or_else(|| {
            topic_id
                .as_deref()
                .and_then(|topic| default_chat_id_for_topic_in(&topics, &room_id, topic))
        });
        if let (Some(topic_id), Some(chat_id)) = (&topic_id, &chat_id)
            && !self.chat_exists(&room_id, topic_id, chat_id)
        {
            return Err(client_error_message(
                "transcript chat is not available in this topic".to_owned(),
            ));
        }
        let mut count = view.limit.map_or(DEFAULT_TRANSCRIPT_WINDOW, |limit| {
            (limit as usize).clamp(DEFAULT_TRANSCRIPT_WINDOW, MAX_APP_MESSAGES)
        });
        if let Some(oldest_message_id) = view.oldest_message_id {
            let Some(anchor) = self.projection.message(&room_id, &oldest_message_id) else {
                return Err(client_error_message(
                    "transcript anchor is not available in this chat".to_owned(),
                ));
            };
            if anchor.conversation_id != topic_id || anchor.chat_id != chat_id {
                return Err(client_error_message(
                    "transcript anchor is not available in this chat".to_owned(),
                ));
            }
            let retained = self
                .projection
                .messages
                .values()
                .filter(|message| {
                    message.room_id == room_id
                        && message.conversation_id == topic_id
                        && message.chat_id == chat_id
                        && !message_sort(message, anchor).is_lt()
                })
                .count();
            count = count.max(retained).min(MAX_APP_MESSAGES);
        }
        state.messages = match (topic_id.as_deref(), chat_id.as_deref()) {
            (Some(topic), Some(chat)) => self
                .projection
                .messages_for_chat_window(&room_id, topic, chat, count),
            (Some(topic), None) => self
                .projection
                .messages_for_topic_window(&room_id, topic, count),
            _ => self.projection.messages_for_room_window(&room_id, count),
        };
        // A streaming reply's latest running edit, collapsed onto its message
        // by clients exactly like a durable edit row.
        state.messages.extend(
            self.drafts
                .iter()
                .filter(|((room, _), draft)| {
                    room == &room_id
                        && topic_id
                            .as_ref()
                            .is_none_or(|topic| draft.conversation_id.as_ref() == Some(topic))
                        && chat_id
                            .as_ref()
                            .is_none_or(|chat| draft.chat_id.as_ref() == Some(chat))
                })
                .map(|(_, draft)| draft.clone()),
        );
        state.messages.sort_by(message_sort);
        let total = match (topic_id.as_deref(), chat_id.as_deref()) {
            (Some(topic), Some(chat)) => self.projection.chat_message_count(&room_id, topic, chat),
            (Some(topic), None) => self.projection.topic_message_count(&room_id, topic),
            _ => self.projection.room_message_count(&room_id),
        };
        for room in &mut state.rooms {
            room.can_load_older = room.room_id == room_id && total > count;
        }
        state.selected_room_id = Some(room_id);
        state.selected_topic_id = topic_id;
        state.selected_chat_id = chat_id;
        Ok(state)
    }

    /// The last opened view while it still exists, or else the only room at
    /// Home, as on a fresh Device.
    fn resumed_view(&self, topics: &[AppTopicSummary]) -> Option<AppView> {
        let Some(last) = self
            .last_view
            .as_ref()
            .filter(|last| self.rooms.contains_key(&last.room_id))
        else {
            return (self.rooms.len() == 1).then(|| AppView {
                room_id: self.rooms.keys().next().cloned(),
                ..AppView::default()
            });
        };
        let topic_id = last.topic_id.clone().filter(|topic_id| {
            topics.iter().any(|topic| {
                topic.room_id == last.room_id && topic.topic_id == *topic_id && !topic.archived
            })
        });
        let chat_id = topic_id.as_deref().and_then(|topic_id| {
            last.chat_id
                .clone()
                .filter(|chat_id| self.chat_exists(&last.room_id, topic_id, chat_id))
        });
        Some(AppView {
            room_id: Some(last.room_id.clone()),
            topic_id,
            chat_id,
            ..AppView::default()
        })
    }

    fn base_state(&self, topics: Vec<AppTopicSummary>, status: &str, now_ms: u64) -> AppState {
        let mut rooms = self
            .rooms
            .values()
            .map(|room| AppRoomSummary {
                room_id: room.room_id.clone(),
                display_name: room.display_name.clone(),
                picture: room.picture.clone(),
                state: AppRoomState::Connected,
                status: "connected".to_owned(),
                user_status_text: String::new(),
                last_message_preview: String::new(),
                unread_count: 0,
                can_load_older: false,
                is_agent_chat: true,
            })
            .collect::<Vec<_>>();
        // One pass over references: cloning every message per view dominated
        // large rooms. Same preview and unread rules as the Device.
        let mut latest = BTreeMap::<&str, &ChatMessage>::new();
        let mut unread = BTreeMap::<&str, u32>::new();
        for message in self.projection.messages.values() {
            let room = message.room_id.as_str();
            if latest
                .get(room)
                .is_none_or(|current| message_sort(message, current).is_gt())
            {
                latest.insert(room, message);
            }
            if !message.is_mine && message.seq > self.read_seq(room) {
                *unread.entry(room).or_default() += 1;
            }
        }
        for room in &mut rooms {
            room.last_message_preview = latest
                .get(room.room_id.as_str())
                .map(|message| message_preview(message))
                .unwrap_or_default();
            room.unread_count = unread
                .get(room.room_id.as_str())
                .copied()
                .unwrap_or_default();
        }
        let paired_agent = (self.rooms.len() == 1).then(|| AppPairedAgent {
            agent_account_id: self.agent.account_id.clone(),
            canonical_room_id: self.rooms.keys().next().cloned().unwrap_or_default(),
        });
        let typing_members = self
            .activities
            .values()
            .filter(|activity| activity.expires_at_ms > now_ms)
            .filter(|activity| is_chat_live_indicator_activity(&activity.member.activity_kind))
            .map(|activity| activity.member.clone())
            .collect();
        AppState {
            rev: self.rev,
            identity: Identity {
                account_id: self.owner.account_id.clone(),
                device_id: self.owner.device_id.clone(),
                account_secret_hex: String::new(),
            },
            rooms,
            paired_agent,
            selected_room_id: None,
            topics,
            selected_topic_id: None,
            selected_chat_id: None,
            active_profile_id: None,
            status: status.to_owned(),
            toast: None,
            messages: Vec::new(),
            media_gallery: None,
            room_details: None,
            profiles: self.profiles.values().cloned().collect(),
            devices: Vec::new(),
            typing_members,
            device_link_bootstrap_receipts: Vec::new(),
            flow: AppFlowState::default(),
        }
    }

    /// Every message of one chat, oldest first, for digests and exports.
    pub fn chat_messages(&self, room_id: &str, topic_id: &str, chat_id: &str) -> Vec<ChatMessage> {
        self.projection
            .messages_for_chat_window(room_id, topic_id, chat_id, MAX_APP_MESSAGES)
    }

    pub fn digest(&self, room_id: &str) -> DirectRoomDigest {
        let topics = self.topics();
        room_digest(room_id, &topics, |topic, chat| {
            self.chat_messages(room_id, topic, chat)
        })
    }
}

fn default_chat_id_for_topic_in(
    topics: &[AppTopicSummary],
    room_id: &str,
    topic_id: &str,
) -> Option<String> {
    let topic = topics
        .iter()
        .find(|topic| topic.room_id == room_id && topic.topic_id == topic_id && !topic.archived)?;
    if let Some(active_chat_id) = topic.active_chat_id.as_deref() {
        let active_is_archived = topic
            .chats
            .iter()
            .any(|chat| chat.chat_id == active_chat_id && chat.archived);
        if !active_is_archived {
            return Some(active_chat_id.to_owned());
        }
    }
    topic
        .chats
        .iter()
        .filter(|chat| !chat.archived)
        .max_by_key(|chat| chat.updated_seq)
        .or_else(|| topic.chats.first())
        .map(|chat| chat.chat_id.clone())
}

/// The chat id the MLS runtime derives for a new-chat intent.
pub fn new_chat_intent_id(room_id: &str, topic_id: &str, intent_key: &str) -> String {
    let digest = Sha256::digest(
        format!("finitechat.new-chat-intent.v1\0{room_id}\0{topic_id}\0{intent_key}").as_bytes(),
    );
    format!("segment-{}", hex::encode(&digest[..20]))
}

fn client_error_message(reason: String) -> FiniteChatCoreError {
    FiniteChatCoreError::Client { reason }
}

fn app_action_name(action: &AppAction) -> String {
    serde_json::to_value(action)
        .ok()
        .and_then(|value| match value {
            serde_json::Value::String(name) => Some(name),
            serde_json::Value::Object(map) => map.keys().next().cloned(),
            _ => None,
        })
        .unwrap_or_else(|| "this action".to_owned())
}

fn sha256_hex_of(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Digest what a person sees in one room: topic and chat identity, titles,
/// archive state, placement and every transcript row a person can see. Both
/// the hosted device (from its `AppState`) and the direct log compute it,
/// so equal digests mean the same view.
pub fn room_digest(
    room_id: &str,
    topics: &[AppTopicSummary],
    mut chat_messages: impl FnMut(&str, &str) -> Vec<ChatMessage>,
) -> DirectRoomDigest {
    let mut room_hasher = Sha256::new();
    let mut topic_digests = Vec::new();
    let mut ordered = topics
        .iter()
        .filter(|topic| topic.room_id == room_id)
        .collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.topic_id.cmp(&b.topic_id));
    for topic in ordered {
        let mut chats = topic.chats.iter().collect::<Vec<_>>();
        chats.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
        let mut chat_digests = Vec::new();
        for chat in chats {
            let messages = chat_messages(&topic.topic_id, &chat.chat_id);
            let mut hasher = Sha256::new();
            for message in &messages {
                let line = serde_json::json!([
                    message.seq,
                    message.message_id,
                    message.sender_account_id,
                    message.sender_device_id,
                    format!("{:?}", message.kind),
                    format!("{:?}", message.status),
                    sha256_hex_of(message.text.as_bytes()),
                    message.edit_of_message_id,
                    message.reply_to_message_id,
                    message.timestamp_unix_seconds,
                    message
                        .media
                        .iter()
                        .map(|media| [
                            media.attachment_id.clone(),
                            media.filename.clone(),
                            media.mime_type.clone()
                        ])
                        .collect::<Vec<_>>(),
                ]);
                hasher.update(line.to_string().as_bytes());
                hasher.update(b"\n");
            }
            let digest = hex::encode(hasher.finalize());
            let entry = DirectChatDigest {
                chat_id: chat.chat_id.clone(),
                title_sha256: sha256_hex_of(chat.title.as_bytes()),
                archived: chat.archived,
                placement_topic_id: chat.placement.as_ref().map(|p| p.topic_id.clone()),
                position: chat.placement.as_ref().map(|p| p.position.clone()),
                message_count: messages.len() as u32,
                digest,
            };
            room_hasher.update(serde_json::to_string(&entry).unwrap_or_default().as_bytes());
            chat_digests.push(entry);
        }
        let entry = DirectTopicDigest {
            topic_id: topic.topic_id.clone(),
            title_sha256: sha256_hex_of(topic.title.as_bytes()),
            archived: topic.archived,
            chats: chat_digests,
        };
        room_hasher.update(
            serde_json::to_string(&(&entry.topic_id, &entry.title_sha256, entry.archived))
                .unwrap_or_default()
                .as_bytes(),
        );
        topic_digests.push(entry);
    }
    DirectRoomDigest {
        room_id: room_id.to_owned(),
        digest: hex::encode(room_hasher.finalize()),
        topics: topic_digests,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{
        NOW, add_runtime_member, spawn_live_http_server, test_profile, with_test_secret,
    };

    /// An in-memory direct log, as the agent's SQLite log behaves.
    #[derive(Default)]
    struct MemoryLog {
        next_seq: BTreeMap<String, u64>,
        ids: u64,
        keyed: BTreeMap<String, StoredAppEvent>,
        events: Vec<StoredAppEvent>,
    }

    impl DirectLog for MemoryLog {
        fn append(
            &mut self,
            draft: DirectEventDraft,
        ) -> Result<StoredAppEvent, FiniteChatCoreError> {
            if let Some(key) = &draft.idempotency_key
                && let Some(existing) = self.keyed.get(key)
            {
                return Ok(existing.clone());
            }
            let seq = self.next_seq.entry(draft.room_id.clone()).or_insert(1_000);
            *seq += 1;
            let event = StoredAppEvent {
                room_id: draft.room_id,
                seq: *seq,
                message_id: format!("direct-{seq}"),
                sender: draft.sender,
                plaintext: draft.plaintext,
                timestamp_unix_seconds: NOW + *seq,
            };
            if let Some(key) = draft.idempotency_key {
                self.keyed.insert(key, event.clone());
            }
            self.events.push(event.clone());
            Ok(event)
        }

        fn generate_object_id(&mut self, prefix: &str) -> Result<String, FiniteChatCoreError> {
            self.ids += 1;
            Ok(format!("{prefix}-{:016x}", self.ids))
        }
    }

    fn open(dir: &Path, server_url: &str, name: &str) -> Arc<FiniteChatRuntime> {
        FiniteChatRuntime::open(with_test_secret(OpenOptions {
            data_dir: dir.join(name).to_string_lossy().into_owned(),
            server_url: server_url.to_owned(),
            device_id: name.to_owned(),
            account_secret_hex: None,
            now_unix_seconds: Some(NOW),
        }))
        .unwrap()
    }

    fn agent_reply(
        room_id: &str,
        topic_id: &str,
        chat_id: &str,
        text: &str,
        status: HermesMessageStatusV1,
        edit_of: Option<String>,
    ) -> Vec<u8> {
        let payload = HermesMessagePayloadV1 {
            payload_type: finitechat_hermes::HERMES_MESSAGE_PAYLOAD_TYPE_V1.to_owned(),
            conversation_id: Some(topic_id.to_owned()),
            segment_id: Some(chat_id.to_owned()),
            text: text.to_owned(),
            kind: finitechat_hermes::HermesSendKindV1::Message,
            status,
            edit_of,
            attachments: Vec::new(),
            reply_to_message_id: None,
            sender_name: Some("Agent".to_owned()),
            metadata: BTreeMap::new(),
        }
        .encode()
        .unwrap();
        let _ = room_id;
        encode_application_event_with_segment(
            DurableAppEventKind::ChatMessage,
            Some(topic_id.to_owned()),
            Some(chat_id.to_owned()),
            &payload,
        )
        .unwrap()
    }

    fn sync_until(
        runtime: &FiniteChatRuntime,
        mut done: impl FnMut(&AppState) -> bool,
    ) -> AppState {
        for _ in 0..50 {
            let state = runtime.dispatch_and_wait(AppAction::StartRuntime).unwrap();
            if done(&state) {
                return state;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("runtime did not converge");
    }

    fn engine_from_fold(
        export: &DirectFoldExport,
        room_id: &str,
        owner: &DeviceRef,
        agent: &DeviceRef,
    ) -> DirectRoomEngine {
        let room = export
            .rooms
            .iter()
            .find(|room| room.room_id == room_id)
            .unwrap();
        let mut engine = DirectRoomEngine::new(owner.clone(), agent.clone());
        engine.upsert_room(DirectRoom {
            room_id: room.room_id.clone(),
            display_name: room.display_name.clone(),
            picture: room.picture.clone(),
        });
        for event in &room.events {
            engine.apply(event.clone());
        }
        engine
    }

    /// Fields a person sees in a transcript row, without Device-local
    /// delivery and download state.
    fn visible(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|message| {
                serde_json::json!({
                    "seq": message.seq,
                    "id": message.message_id,
                    "topic": message.conversation_id,
                    "chat": message.chat_id,
                    "sender": message.sender_account_id,
                    "device": message.sender_device_id,
                    "name": message.sender_display_name,
                    "mine": message.is_mine,
                    "text": message.text,
                    "display": message.display_content,
                    "rich": message.rich_text_json,
                    "metadata": message.metadata_json,
                    "kind": format!("{:?}", message.kind),
                    "status": format!("{:?}", message.status),
                    "edit_of": message.edit_of_message_id,
                    "reply_to": message.reply_to_message_id,
                    "time": message.timestamp_unix_seconds,
                    "media": message.media.iter().map(|media| serde_json::json!([media.attachment_id, media.filename, media.mime_type, format!("{:?}", media.kind)])).collect::<Vec<_>>(),
                })
            })
            .collect()
    }

    #[test]
    fn folded_device_logs_project_the_same_view_the_person_sees() {
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_http_server(dir.path().join("server.sqlite3"));
        let user = open(dir.path(), &server_url, "hosted-web");
        let agent = open(dir.path(), &server_url, "agent");
        let agent_account = agent.state().unwrap().identity.account_id;

        // The person's hosted device creates the room and admits the agent,
        // as agent binding does today.
        let room_id = user
            .dispatch_and_wait(AppAction::CreateRoom {
                display_name: "Agent".to_owned(),
            })
            .unwrap()
            .selected_room_id
            .unwrap();
        add_runtime_member(
            &user,
            &agent,
            &room_id,
            test_profile(&agent_account, "Agent"),
        );

        let home = user.state().unwrap();
        let home_chat = home
            .topics
            .iter()
            .find(|topic| topic.topic_id == HOME_TOPIC_ID)
            .and_then(|topic| topic.chats.first())
            .map(|chat| chat.chat_id.clone())
            .unwrap();
        user.dispatch_and_wait(AppAction::SendChatMessage {
            room_id: room_id.clone(),
            topic_id: HOME_TOPIC_ID.to_owned(),
            chat_id: home_chat.clone(),
            text: "hello from home".to_owned(),
            metadata_json: None,
        })
        .unwrap();
        let trip = user
            .dispatch_and_wait(AppAction::CreateTopic {
                room_id: room_id.clone(),
                title: "Trip planning".to_owned(),
            })
            .unwrap();
        let trip_topic = trip.selected_topic_id.clone().unwrap();
        let flights = trip.selected_chat_id.clone().unwrap();
        user.dispatch_and_wait(AppAction::SendChatMessage {
            room_id: room_id.clone(),
            topic_id: trip_topic.clone(),
            chat_id: flights.clone(),
            text: "find flights".to_owned(),
            metadata_json: Some(r#"{"client":"web"}"#.to_owned()),
        })
        .unwrap();
        user.dispatch_and_wait(AppAction::RenameChat {
            room_id: room_id.clone(),
            topic_id: trip_topic.clone(),
            chat_id: flights.clone(),
            title: "Flights".to_owned(),
        })
        .unwrap();
        let hotels = user
            .dispatch_and_wait(AppAction::StartTopicChat {
                room_id: room_id.clone(),
                topic_id: trip_topic.clone(),
                reason: Some("hotels".to_owned()),
            })
            .unwrap()
            .selected_chat_id
            .unwrap();
        user.dispatch_and_wait(AppAction::SendChatAttachment {
            room_id: room_id.clone(),
            topic_id: trip_topic.clone(),
            chat_id: hotels.clone(),
            filename: "hotel.png".to_owned(),
            mime_type: "image/png".to_owned(),
            kind: ChatMediaKind::Image,
            bytes: b"\x89PNG not really".to_vec(),
            caption: "the hotel".to_owned(),
            reply_to_message_id: None,
        })
        .unwrap();
        user.dispatch_and_wait(AppAction::MoveChat {
            room_id: room_id.clone(),
            topic_id: trip_topic.clone(),
            chat_id: flights.clone(),
            destination_topic_id: trip_topic.clone(),
            before: Some(ChatAddress {
                topic_id: trip_topic.clone(),
                chat_id: hotels.clone(),
            }),
        })
        .unwrap();
        let abandoned = user
            .dispatch_and_wait(AppAction::StartTopicChat {
                room_id: room_id.clone(),
                topic_id: trip_topic.clone(),
                reason: None,
            })
            .unwrap()
            .selected_chat_id
            .unwrap();
        user.dispatch_and_wait(AppAction::SetChatArchived {
            room_id: room_id.clone(),
            topic_id: trip_topic.clone(),
            chat_id: abandoned.clone(),
            archived: true,
        })
        .unwrap();

        // The agent answers in two routes, with a running row then its edit.
        let mut seen = 0;
        while seen < 6 {
            seen += agent.agent_bridge_poll_once().unwrap().events.len();
            thread::sleep(Duration::from_millis(20));
        }
        agent
            .send_encoded_chat_message_and_wait(
                room_id.clone(),
                agent_reply(
                    &room_id,
                    HOME_TOPIC_ID,
                    &home_chat,
                    "hi there",
                    HermesMessageStatusV1::Complete,
                    None,
                ),
                "hi there".to_owned(),
            )
            .unwrap();
        let running = agent
            .send_encoded_chat_message_and_wait(
                room_id.clone(),
                agent_reply(
                    &room_id,
                    &trip_topic,
                    &flights,
                    "searching…",
                    HermesMessageStatusV1::Running,
                    None,
                ),
                "searching".to_owned(),
            )
            .unwrap();
        agent
            .send_encoded_chat_message_and_wait(
                room_id.clone(),
                agent_reply(
                    &room_id,
                    &trip_topic,
                    &flights,
                    "three flights found",
                    HermesMessageStatusV1::Complete,
                    Some(running.message_id),
                ),
                "three flights found".to_owned(),
            )
            .unwrap();
        let user_state = sync_until(&user, |state| {
            state
                .topics
                .iter()
                .flat_map(|topic| &topic.chats)
                .map(|chat| chat.message_count)
                .sum::<u32>()
                >= 6
        });

        let user_device = DeviceRef {
            account_id: user_state.identity.account_id.clone(),
            device_id: user_state.identity.device_id.clone(),
        };
        let agent_device = DeviceRef {
            account_id: agent_account.clone(),
            device_id: "agent".to_owned(),
        };
        let user_fold = user.export_direct_fold().unwrap();
        let agent_fold = agent.export_direct_fold().unwrap();

        for (source, export) in [("hosted device", &user_fold), ("agent", &agent_fold)] {
            let mut engine = engine_from_fold(export, &room_id, &user_device, &agent_device);
            let fold_room = export
                .rooms
                .iter()
                .find(|room| room.room_id == room_id)
                .unwrap();
            // Read state is per Device; give the engine the person's.
            let person_read = user_fold
                .rooms
                .iter()
                .find(|room| room.room_id == room_id)
                .unwrap()
                .local_read_seq;
            engine.set_read_seq(&room_id, person_read);
            assert!(
                fold_room.member_account_ids.contains(&agent_account),
                "{source}"
            );
            let mut direct_topics = engine.topics();
            if source == "agent" {
                // The person's Device writes Home's ConversationCreate before it
                // admits the agent, so the agent cannot decrypt it (MLS
                // forward secrecy) and dates Home from its first message.
                // `created_seq` is not shown or used for order; every other
                // field must match.
                let home = direct_topics
                    .iter_mut()
                    .find(|topic| topic.topic_id == HOME_TOPIC_ID)
                    .unwrap();
                let expected_home = user_state
                    .topics
                    .iter()
                    .find(|topic| topic.topic_id == HOME_TOPIC_ID)
                    .unwrap();
                assert!(home.created_seq > expected_home.created_seq);
                home.created_seq = expected_home.created_seq;
            }
            assert_eq!(
                direct_topics, user_state.topics,
                "{source}: topics, order, titles, archive and placement"
            );
            for topic in &user_state.topics {
                for chat in &topic.chats {
                    let view = AppView {
                        room_id: Some(room_id.clone()),
                        topic_id: Some(topic.topic_id.clone()),
                        chat_id: Some(chat.chat_id.clone()),
                        limit: Some(10_000),
                        oldest_message_id: None,
                    };
                    let expected = user.state_for_view(view.clone()).unwrap().messages;
                    let direct = engine
                        .state_for_view(view, "connected", 0)
                        .unwrap()
                        .messages;
                    assert_eq!(
                        visible(&direct),
                        visible(&expected),
                        "{source}: {}/{}",
                        topic.title,
                        chat.title
                    );
                }
            }
            let expected = room_digest(&room_id, &user_state.topics, |topic, chat| {
                user.state_for_view(AppView {
                    room_id: Some(room_id.clone()),
                    topic_id: Some(topic.to_owned()),
                    chat_id: Some(chat.to_owned()),
                    limit: Some(10_000),
                    oldest_message_id: None,
                })
                .unwrap()
                .messages
            });
            assert_eq!(engine.digest(&room_id), expected, "{source}: digest");

            // After the fold, the engine keeps working like the Device did.
            let mut log = MemoryLog::default();
            let selection = engine
                .execute(
                    AppAction::SendChatMessage {
                        room_id: room_id.clone(),
                        topic_id: trip_topic.clone(),
                        chat_id: hotels.clone(),
                        text: "direct now".to_owned(),
                        metadata_json: None,
                    },
                    Some(&DirectRequesterContext {
                        email: "person@example.com".to_owned(),
                        sites_assertion: "assertion".to_owned(),
                    }),
                    &mut log,
                )
                .unwrap();
            assert_eq!(selection.chat_id.as_deref(), Some(hotels.as_str()));
            let hotels_rows = engine.chat_messages(&room_id, &trip_topic, &hotels);
            let sent = hotels_rows.last().unwrap();
            assert!(sent.is_mine && sent.text == "direct now");
            // Verified requester claims travel in the payload to Hermes; the
            // transcript projection keeps them out of display metadata.
            let DecodedAppEvent::ChatMessage(stored) =
                decode_application_event(&log.events.last().unwrap().plaintext)
            else {
                panic!("the send is a chat message");
            };
            assert!(String::from_utf8_lossy(&stored.raw_payload).contains("person@example.com"));
            let renamed = engine
                .execute(
                    AppAction::RenameChat {
                        room_id: room_id.clone(),
                        topic_id: trip_topic.clone(),
                        chat_id: hotels.clone(),
                        title: "Hotels".to_owned(),
                    },
                    None,
                    &mut log,
                )
                .unwrap();
            assert_eq!(renamed.status, "chat renamed");
            let created = engine
                .new_chat(&mut log, &room_id, HOME_TOPIC_ID, None, "intent-1")
                .unwrap();
            let again = engine
                .new_chat(&mut log, &room_id, HOME_TOPIC_ID, None, "intent-1")
                .unwrap();
            assert_eq!(
                created.chat_id, again.chat_id,
                "new-chat intents are idempotent"
            );
            assert_eq!(
                created.chat_id.as_deref(),
                Some(new_chat_intent_id(&room_id, HOME_TOPIC_ID, "intent-1").as_str())
            );
            let topics = engine.topics();
            let trip = topics
                .iter()
                .find(|topic| topic.topic_id == trip_topic)
                .unwrap();
            assert!(
                trip.chats
                    .iter()
                    .any(|chat| chat.chat_id == hotels && chat.title == "Hotels")
            );
            let unscoped = engine
                .append_agent_event(
                    &mut log,
                    &room_id,
                    encode_application_event(
                        DurableAppEventKind::ChatMessage,
                        None,
                        &encode_text_message_payload("cron result", None).unwrap(),
                    )
                    .unwrap(),
                )
                .unwrap();
            let routed = engine.message(&room_id, &unscoped.message_id).unwrap();
            assert_eq!(routed.conversation_id.as_deref(), Some(HOME_TOPIC_ID));
            assert!(!routed.is_mine);
        }
    }

    #[test]
    fn streamed_replies_keep_running_edits_as_drafts_until_the_final_edit() {
        let owner = DeviceRef {
            account_id: "a".repeat(64),
            device_id: DIRECT_WEB_DEVICE_ID.to_owned(),
        };
        let agent = DeviceRef {
            account_id: "b".repeat(64),
            device_id: "agent".to_owned(),
        };
        let mut engine = DirectRoomEngine::new(owner, agent.clone());
        engine.upsert_room(DirectRoom {
            room_id: "room-direct".to_owned(),
            display_name: "Agent".to_owned(),
            picture: None,
        });
        let mut log = MemoryLog::default();
        engine.ensure_home_topic("room-direct", &mut log).unwrap();
        let view = || AppView {
            room_id: Some("room-direct".to_owned()),
            topic_id: Some(HOME_TOPIC_ID.to_owned()),
            chat_id: Some(HOME_CHAT_ID.to_owned()),
            limit: None,
            oldest_message_id: None,
        };
        let first = engine
            .append_agent_event(
                &mut log,
                "room-direct",
                agent_reply(
                    "room-direct",
                    HOME_TOPIC_ID,
                    HOME_CHAT_ID,
                    "Thinking ▉",
                    HermesMessageStatusV1::Running,
                    None,
                ),
            )
            .unwrap();
        let durable_before = log.events.len();
        let draft = agent_reply(
            "room-direct",
            HOME_TOPIC_ID,
            HOME_CHAT_ID,
            "Thinking about it ▉",
            HermesMessageStatusV1::Running,
            Some(first.message_id.clone()),
        );
        assert_eq!(
            engine.set_draft("room-direct", &agent, &draft, NOW),
            Some((first.message_id.clone(), first.seq))
        );
        assert_eq!(log.events.len(), durable_before, "drafts are never durable");
        let streaming = engine.state_for_view(view(), "ready", 0).unwrap().messages;
        let shown = streaming.last().unwrap();
        assert_eq!(shown.text, "Thinking about it ▉");
        assert_eq!(
            shown.edit_of_message_id.as_deref(),
            Some(first.message_id.as_str())
        );
        // A first part is not a running edit, and a complete edit is durable.
        let not_an_edit = agent_reply(
            "room-direct",
            HOME_TOPIC_ID,
            HOME_CHAT_ID,
            "x ▉",
            HermesMessageStatusV1::Running,
            None,
        );
        assert_eq!(
            engine.set_draft("room-direct", &agent, &not_an_edit, NOW),
            None
        );
        let final_edit = agent_reply(
            "room-direct",
            HOME_TOPIC_ID,
            HOME_CHAT_ID,
            "Done thinking",
            HermesMessageStatusV1::Complete,
            Some(first.message_id.clone()),
        );
        assert_eq!(
            engine.set_draft("room-direct", &agent, &final_edit, NOW),
            None
        );
        engine
            .append_agent_event(&mut log, "room-direct", final_edit)
            .unwrap();
        let settled = engine.state_for_view(view(), "ready", 0).unwrap().messages;
        assert!(
            settled
                .iter()
                .all(|message| !message.message_id.starts_with("draft-"))
        );
        assert_eq!(settled.last().unwrap().text, "Done thinking");
    }

    #[test]
    fn a_fresh_direct_room_gets_one_home_topic_and_rejects_unknown_routes() {
        let owner = DeviceRef {
            account_id: "a".repeat(64),
            device_id: DIRECT_WEB_DEVICE_ID.to_owned(),
        };
        let agent = DeviceRef {
            account_id: "b".repeat(64),
            device_id: "agent".to_owned(),
        };
        let mut engine = DirectRoomEngine::new(owner, agent);
        engine.upsert_room(DirectRoom {
            room_id: "room-direct".to_owned(),
            display_name: "Agent".to_owned(),
            picture: None,
        });
        let mut log = MemoryLog::default();
        engine.ensure_home_topic("room-direct", &mut log).unwrap();
        engine.ensure_home_topic("room-direct", &mut log).unwrap();
        let topics = engine.topics();
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].title, HOME_TOPIC_TITLE);
        let sent = engine
            .execute(
                AppAction::SendMessage {
                    room_id: "room-direct".to_owned(),
                    text: "first".to_owned(),
                    metadata_json: None,
                },
                None,
                &mut log,
            )
            .unwrap();
        assert_eq!(sent.topic_id.as_deref(), Some(HOME_TOPIC_ID));
        let state = engine
            .state_for_view(
                AppView {
                    room_id: Some("room-direct".to_owned()),
                    ..AppView::default()
                },
                "connected",
                0,
            )
            .unwrap();
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.paired_agent.unwrap().canonical_room_id, "room-direct");
        assert!(
            engine
                .execute(
                    AppAction::RenameChat {
                        room_id: "room-direct".to_owned(),
                        topic_id: "missing".to_owned(),
                        chat_id: "missing".to_owned(),
                        title: "x".to_owned(),
                    },
                    None,
                    &mut log,
                )
                .is_err()
        );
        assert!(
            engine
                .execute(AppAction::StopRuntime, None, &mut log)
                .unwrap_err()
                .to_string()
                .contains("not available in direct transport")
        );
    }

    #[test]
    fn a_view_naming_no_room_resumes_the_last_opened_chat() {
        let owner = DeviceRef {
            account_id: "a".repeat(64),
            device_id: DIRECT_WEB_DEVICE_ID.to_owned(),
        };
        let agent = DeviceRef {
            account_id: "b".repeat(64),
            device_id: "agent".to_owned(),
        };
        let mut engine = DirectRoomEngine::new(owner, agent);
        engine.upsert_room(DirectRoom {
            room_id: "room-direct".to_owned(),
            display_name: "Agent".to_owned(),
            picture: None,
        });
        let mut log = MemoryLog::default();
        engine.ensure_home_topic("room-direct", &mut log).unwrap();
        let landing = |engine: &DirectRoomEngine| {
            let state = engine
                .state_for_view(AppView::default(), "connected", 0)
                .unwrap();
            (
                state.selected_room_id,
                state.selected_topic_id,
                state.selected_chat_id,
            )
        };
        // Nothing opened yet: the only room at Home, as on a fresh Device.
        let (room, topic, _) = landing(&engine);
        assert_eq!(room.as_deref(), Some("room-direct"));
        assert_eq!(topic.as_deref(), Some(HOME_TOPIC_ID));

        let created = engine
            .execute(
                AppAction::CreateTopic {
                    room_id: "room-direct".to_owned(),
                    title: "Trip".to_owned(),
                },
                None,
                &mut log,
            )
            .unwrap();
        let trip = created.topic_id.clone().unwrap();
        let first = created.chat_id.clone().unwrap();
        let second = engine
            .new_chat(&mut log, "room-direct", &trip, None, "intent-second")
            .unwrap()
            .chat_id
            .unwrap();
        engine
            .execute(
                AppAction::OpenChat {
                    room_id: "room-direct".to_owned(),
                    topic_id: trip.clone(),
                    chat_id: first.clone(),
                },
                None,
                &mut log,
            )
            .unwrap();
        // Reads and non-navigation actions leave it where the person was.
        engine
            .execute(
                AppAction::MarkRoomRead {
                    room_id: "room-direct".to_owned(),
                },
                None,
                &mut log,
            )
            .unwrap();
        engine
            .state_for_view(
                AppView {
                    room_id: Some("room-direct".to_owned()),
                    topic_id: Some(HOME_TOPIC_ID.to_owned()),
                    ..AppView::default()
                },
                "connected",
                0,
            )
            .unwrap();
        assert_eq!(
            landing(&engine),
            (
                Some("room-direct".to_owned()),
                Some(trip.clone()),
                Some(first.clone())
            )
        );
        assert_eq!(
            engine.last_view(),
            Some(&DirectLastView {
                room_id: "room-direct".to_owned(),
                topic_id: Some(trip.clone()),
                chat_id: Some(first.clone()),
            })
        );

        // A remembered chat that is gone falls back to its topic's default;
        // a room that is gone falls back to Home.
        engine.set_last_view(Some(DirectLastView {
            room_id: "room-direct".to_owned(),
            topic_id: Some(trip.clone()),
            chat_id: Some("missing".to_owned()),
        }));
        let (_, topic, chat) = landing(&engine);
        assert_eq!(topic.as_deref(), Some(trip.as_str()));
        assert!(chat == Some(first.clone()) || chat == Some(second.clone()));
        engine.set_last_view(Some(DirectLastView {
            room_id: "room-gone".to_owned(),
            topic_id: None,
            chat_id: None,
        }));
        assert_eq!(landing(&engine).1.as_deref(), Some(HOME_TOPIC_ID));
    }
}
