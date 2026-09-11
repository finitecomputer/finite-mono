//! Disposable browser experiment. All operations execute on the existing Core writer.
use super::*;
use serde_json::{Value, json};

pub const REQUEST: &str = "finitechat.browser.request.v1";
pub const RESPONSE: &str = "finitechat.browser.response.v1";
pub const ROOM: &str = "wasm-hermes-room";

pub enum Command {
    Initialize,
    Enroll {
        device: DeviceRef,
        key_package_id: String,
        key_package_hash: String,
    },
    Respond,
}
impl FiniteChatRuntime {
    pub fn browser_spike(&self, request: Command) -> Result<Value, FiniteChatCoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::BrowserSpike {
                request,
                response: tx,
            })
            .map_err(client_error)?;
        rx.recv().map_err(client_error)?
    }
}
impl AppRuntimeState {
    pub(super) fn browser_spike(&mut self, request: Command) -> Result<Value, FiniteChatCoreError> {
        self.core
            .device
            .set_now_unix_seconds(self.core.now_unix_seconds()?);
        match request {
            Command::Initialize => {
                if !self.core.has_room(ROOM) {
                    self.core.bootstrap_room(ROOM, Some("Hermes".into()))?;
                    self.upsert_room(ROOM, "Hermes", None, AppRoomState::Connected, "connected");
                    self.persist_room_projection(ROOM)?;
                    self.ensure_home_topic(ROOM)?;
                    self.start_topic_chat(ROOM.into(), HOME_TOPIC_ID.into(), None)?;
                }
                Ok(json!({"room": ROOM}))
            }
            Command::Enroll {
                device,
                key_package_id,
                key_package_hash,
            } => {
                let synced = self.core.sync_room_with_projection(ROOM)?;
                self.apply_targeted_sync_projection(ROOM, synced)?;
                // The grant is scoped to one public Device and KeyPackage, never a profile-wide claim.
                let journal = self.core.data_dir.join(format!(
                    "browser-admit-{}.json",
                    hex::encode(Sha256::digest(
                        serde_json::to_vec(&device).map_err(client_error)?
                    ))
                ));
                if self
                    .core
                    .device
                    .room_members(ROOM)
                    .map_err(client_error)?
                    .contains(&device)
                {
                    return Ok(json!({"room": ROOM}));
                }
                let prepared: PreparedCommit = if journal.exists() {
                    let request: SubmitCommitRequest =
                        serde_json::from_slice(&fs::read(&journal).map_err(client_error)?)
                            .map_err(client_error)?;
                    PreparedCommit {
                        message_id: request.envelope.message_id().map_err(client_error)?,
                        request,
                    }
                } else {
                    let claim = self
                        .core
                        .home_delivery()
                        .claim_key_package_for_device(&device)
                        .map_err(send_delivery_error)?
                        .ok_or_else(|| client_error("Device has no KeyPackage"))?;
                    if claim.key_package_id != key_package_id
                        || claim.key_package_hash != key_package_hash
                    {
                        return Err(client_error(
                            "KeyPackage differs from authorized enrollment",
                        ));
                    }
                    let prepared = self
                        .core
                        .device
                        .prepare_add_member_commit(
                            ROOM,
                            &claim,
                            format!("welcome-{key_package_id}"),
                            format!("admit-{key_package_id}"),
                        )
                        .map_err(client_error)?;
                    // Exact randomized Commit is retained across uncertain HTTP acceptance.
                    // See HOW_TO_DO_IT_FOR_REAL_LOG for the file/SQLite atomicity gap.
                    fs::write(
                        &journal,
                        serde_json::to_vec(&prepared.request).map_err(client_error)?,
                    )
                    .map_err(client_error)?;
                    prepared
                };
                if !self.submit_prepared_members_commit(
                    prepared,
                    "admission pending",
                    "Retry admission",
                )? {
                    return Err(client_error("Admission is pending; retry the same Device"));
                }
                let synced = self.core.sync_room_with_projection(ROOM)?;
                self.apply_targeted_sync_projection(ROOM, synced)?;
                Ok(json!({"room": ROOM}))
            }
            Command::Respond => self.browser_respond(),
        }
    }

    fn browser_respond(&mut self) -> Result<Value, FiniteChatCoreError> {
        // Bounded full scan is deliberately a spike shortcut. Source is the agent's
        // encrypted durable transcript, including requests accepted before restart.
        let stored = self
            .core
            .store
            .load_app_events(self.core.device.device_ref(), 10000)
            .map_err(store_error)?;
        let parsed: Vec<_> = stored
            .iter()
            .filter_map(|s| {
                serde_json::from_slice::<DecryptedApplicationEventV1>(&s.plaintext)
                    .ok()
                    .map(|e| (s, e))
            })
            .collect();
        let completed: BTreeSet<String> = parsed.iter().filter(|(s,e)| s.sender == *self.core.device.device_ref() && matches!(&e.kind, DurableAppEventKind::Namespaced { name, .. } if name == RESPONSE)).filter_map(|(_,e)| serde_json::from_slice::<Value>(&e.payload).ok()?.get("request_id")?.as_str().map(str::to_owned)).collect();
        let allowed = std::env::var("FINITECHAT_BROWSER_SPIKE_USER").unwrap_or_default();
        for (stored, event) in &parsed {
            if stored.room_id != ROOM
                || stored.sender.account_id != allowed
                || !matches!(&event.kind, DurableAppEventKind::Namespaced { name, .. } if name == REQUEST)
                || completed.contains(&stored.message_id)
            {
                continue;
            }
            if !self
                .core
                .device
                .room_members(ROOM)
                .map_err(client_error)?
                .contains(&stored.sender)
            {
                continue;
            }
            let Ok(request) = serde_json::from_slice::<Value>(&event.payload) else {
                continue;
            };
            let boundary = self
                .core
                .device
                .last_applied_seq(ROOM)
                .map_err(client_error)?;
            let mut payload = match request["operation"].as_str() {
                Some("metadata") => {
                    let mut topics = self
                        .app
                        .topics
                        .iter()
                        .filter(|t| t.room_id == ROOM)
                        .cloned()
                        .collect::<Vec<_>>();
                    for topic in &mut topics {
                        topic.last_message_preview.clear();
                        topic.message_count = 0;
                        topic.unread_count = 0;
                        for chat in &mut topic.chats {
                            chat.last_message_preview.clear();
                            chat.message_count = 0;
                            chat.unread_count = 0;
                        }
                    }
                    json!({"operation": "metadata", "boundary": boundary, "topics": topics})
                }
                Some("history") => match self.browser_history_page(&request, boundary) {
                    Ok(page) => page,
                    Err(error) => {
                        json!({"operation":"history", "topic_id":request["topic_id"], "chat_id":request["chat_id"], "error":error.to_string()})
                    }
                },
                _ => continue,
            };
            payload["request_id"] = json!(stored.message_id);
            payload["target"] = json!(stored.sender);
            self.core.send_application_event(
                ROOM,
                DurableAppEventKind::Namespaced {
                    name: RESPONSE.into(),
                    policy: ApplicationDeliveryPolicy::NON_NOTIFYING,
                },
                None,
                &serde_json::to_vec(&payload).map_err(client_error)?,
                "browser-response",
            )?;
        }
        Ok(json!({"ok":true}))
    }
    fn browser_history_page(
        &self,
        request: &Value,
        boundary: u64,
    ) -> Result<Value, FiniteChatCoreError> {
        let topic = request["topic_id"]
            .as_str()
            .ok_or_else(|| client_error("Missing Topic"))?;
        let chat = request["chat_id"]
            .as_str()
            .ok_or_else(|| client_error("Missing Chat"))?;
        if !self.app.topics.iter().any(|t| {
            t.room_id == ROOM && t.topic_id == topic && t.chats.iter().any(|c| c.chat_id == chat)
        }) {
            return Err(client_error(
                "The agent has no stored metadata for this Chat",
            ));
        }
        let before = request["before_seq"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= boundary.saturating_add(1))
            .ok_or_else(|| client_error("Invalid history cursor"))?;
        let limit = request["limit"].as_u64().unwrap_or(80).clamp(1, 100) as usize;
        let mut matching = VecDeque::new();
        let (mut after, mut after_id) = (0, String::new());
        let mut more = false;
        // Read the agent's authoritative transcript in bounded SQLite pages. This
        // intentionally scans the Room; a real deployment needs a per-Chat index.
        loop {
            let page = self
                .core
                .store
                .load_app_events_for_room_page(
                    self.core.device.device_ref(),
                    ROOM,
                    before - 1,
                    after,
                    &after_id,
                    256,
                )
                .map_err(store_error)?;
            if page.is_empty() {
                break;
            }
            for stored in page {
                after = stored.seq;
                after_id = stored.message_id.clone();
                let Ok(event) =
                    serde_json::from_slice::<DecryptedApplicationEventV1>(&stored.plaintext)
                else {
                    continue;
                };
                if !matches!(
                    event.kind,
                    DurableAppEventKind::ChatMessage | DurableAppEventKind::ChatEdit
                ) || event.conversation_id.as_deref() != Some(topic)
                {
                    continue;
                }
                let Ok(payload) = serde_json::from_slice::<Value>(&event.payload) else {
                    continue;
                };
                if event
                    .segment_id
                    .as_deref()
                    .or_else(|| payload["segment_id"].as_str())
                    != Some(chat)
                {
                    continue;
                }
                matching.push_back(json!({"id":stored.message_id, "seq":stored.seq, "timestamp":stored.timestamp_unix_seconds,
                    "sender":stored.sender, "kind":event.kind, "conversation_id":event.conversation_id, "segment_id":chat, "payload":payload}));
                if matching.len() > limit {
                    matching.pop_front();
                    more = true;
                }
            }
        }
        // Leave room for the outer event/envelope. Oversized individual messages
        // produce a history-only error, never a failure of live Room sync.
        while serde_json::to_vec(&matching).map_err(client_error)?.len() > 48 * 1024 {
            if matching.len() == 1 {
                return Err(client_error(
                    "This historical message needs chunked transfer; live chat is still available",
                ));
            }
            matching.pop_front();
            more = true;
        }
        let next_before = matching
            .front()
            .and_then(|event| event["seq"].as_u64())
            .unwrap_or(before);
        Ok(
            json!({"operation":"history", "topic_id":topic, "chat_id":chat, "before_seq":before,
            "next_before_seq":next_before, "has_more":more, "events":matching}),
        )
    }
}
