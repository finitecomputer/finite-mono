use finitechat_client::{
    AppliedLogEntry, FiniteChatDevice, FiniteChatDeviceConfig, FiniteChatDeviceState,
};
use finitechat_delivery::{HttpSyncPage, MAX_HTTP_SYNC_PAGE_ENTRIES};
use finitechat_hermes::{HermesMessagePayloadV1, HermesSendRequestV1};
use finitechat_http::{FiniteAccountRoomCommitProjection, GroupSyncRequest};
use finitechat_mls::NostrSecretKey;
use finitechat_proto::{
    AppendApplicationEventRequest, DurableAppEventKind, EventAccepted, RoomLogEntry,
    UploadKeyPackageRequest, delivery_member_id_for_device,
};
use finitechat_proto::{DecryptedApplicationEventV1, DeviceRef};
use finitechat_transport::{GroupId, MemberId};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

fn error(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}
fn now() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

#[derive(Clone, Serialize, Deserialize)]
struct BrowserEvent {
    id: String,
    seq: u64,
    sender: DeviceRef,
    timestamp: u64,
    kind: DurableAppEventKind,
    conversation_id: Option<String>,
    segment_id: Option<String>,
    payload: serde_json::Value,
}

#[derive(Clone, Serialize, Deserialize)]
struct PendingSend {
    request: AppendApplicationEventRequest,
    plaintext: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    version: u32,
    device_state: Vec<u8>,
    room: String,
    after_seq: u64,
    events: Vec<BrowserEvent>,
    send: Option<PendingSend>,
    next_send: u64,
    #[serde(default)]
    join_package: Option<UploadKeyPackageRequest>,
    #[serde(default)]
    welcome_ack: Option<String>,
}

#[wasm_bindgen]
pub struct BrowserChat {
    device: FiniteChatDevice,
    secret: NostrSecretKey,
    server: String,
    http: reqwest::Client,
    room: String,
    after_seq: u64,
    persist: js_sys::Function,
    send: Option<PendingSend>,
    next_send: u64,
    join_package: Option<UploadKeyPackageRequest>,
    welcome_ack: Option<String>,
    events: Vec<BrowserEvent>,
}

#[wasm_bindgen]
impl BrowserChat {
    #[wasm_bindgen(constructor)]
    pub fn new(
        nsec: &str,
        server: &str,
        device_id: &str,
        saved: Option<String>,
        persist: js_sys::Function,
    ) -> Result<BrowserChat, JsValue> {
        let key = nostr::SecretKey::parse(nsec).map_err(error)?;
        let secret = NostrSecretKey::from_bytes(key.to_secret_bytes()).map_err(error)?;
        let config = FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: now(),
            credential_not_before_unix_seconds: now() - 60,
            credential_not_after_unix_seconds: now() + 86400,
        };
        let saved: Option<Checkpoint> = saved
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(error)?;
        if saved.as_ref().is_some_and(|saved| saved.version != 1) {
            return Err(error(
                "Unsupported browser checkpoint; refusing a replacement Device",
            ));
        }
        let device = if let Some(saved) = &saved {
            FiniteChatDevice::from_state(
                config,
                FiniteChatDeviceState::decode_snapshot(&saved.device_state).map_err(error)?,
            )
        } else {
            FiniteChatDevice::new(config)
        }
        .map_err(error)?;
        Ok(Self {
            device,
            secret,
            persist,
            server: server.trim_end_matches('/').to_owned(),
            http: reqwest::Client::new(),
            room: saved.as_ref().map(|s| s.room.clone()).unwrap_or_default(),
            after_seq: saved.as_ref().map(|s| s.after_seq).unwrap_or_default(),
            send: saved.as_ref().and_then(|s| s.send.clone()),
            next_send: saved.as_ref().map(|s| s.next_send).unwrap_or_default(),
            join_package: saved.as_ref().and_then(|s| s.join_package.clone()),
            welcome_ack: saved.as_ref().and_then(|s| s.welcome_ack.clone()),
            events: saved.map(|s| s.events).unwrap_or_default(),
        })
    }

    pub fn checkpoint(&self) -> Result<String, JsValue> {
        serde_json::to_string(&Checkpoint {
            version: 1,
            device_state: self
                .device
                .export_state()
                .map_err(error)?
                .encode_snapshot()
                .map_err(error)?,
            room: self.room.clone(),
            after_seq: self.after_seq,
            events: self.events.clone(),
            send: self.send.clone(),
            next_send: self.next_send,
            join_package: self.join_package.clone(),
            welcome_ack: self.welcome_ack.clone(),
        })
        .map_err(error)
    }

    /// Each browser owns an independent MLS leaf. The existing agent admits it.
    pub async fn join(&mut self, agent: &str, agent_url: &str, room: &str) -> Result<(), JsValue> {
        if !self.room.is_empty() {
            if self.room != room {
                return Err(error("Stored Device belongs to a different Room"));
            }
            self.sync().await?;
            return Ok(());
        }
        if self.join_package.is_none() {
            self.join_package = Some(
                self.device
                    .upload_key_package_auto_id_request()
                    .map_err(error)?,
            );
            self.save_checkpoint().await?;
        }
        let package = self.join_package.clone().unwrap();
        let publication = finitechat_delivery::HttpKeyPackagePublication {
            key_package_id: finitechat_delivery::HttpKeyPackageId::new(
                package.key_package_id.as_bytes().to_vec(),
            ),
            owner: MemberId::new(delivery_member_id_for_device(self.device.device_ref())),
            key_package: finitechat_transport::engine::KeyPackage::new(
                serde_json::to_vec(&package).map_err(error)?,
            ),
        };
        let _: serde_json::Value = self.post("/key-packages", &publication).await?;
        let _: serde_json::Value = self.post(&format!("{agent_url}/spike/enroll"), &serde_json::json!({
            "room": room, "device": self.device.device_ref(),
            "key_package_id": package.key_package_id, "key_package_hash": package.key_package_hash,
        })).await?;
        let welcomes: Vec<finitechat_http::HttpClaimedWelcome> = self
            .post(
                "/welcomes/claim",
                &finitechat_http::ClaimWelcomesRequest {
                    recipient: MemberId::new(delivery_member_id_for_device(
                        self.device.device_ref(),
                    )),
                    limit: 16,
                },
            )
            .await?;
        let mut activated = false;
        if let Some(claimed) = welcomes.into_iter().next() {
            let welcome: finitechat_proto::WelcomeRecord =
                serde_json::from_slice(&claimed.message.payload).map_err(error)?;
            if welcome.recipient != *self.device.device_ref()
                || welcome.sender.account_id != agent
                || welcome.room_id != room
                || welcome.key_package_id != package.key_package_id
                || claimed.message.id.as_slice() != welcome.welcome_id.as_bytes()
            {
                return Err(error("Unsolicited or mismatched enrollment Welcome"));
            }
            self.device
                .activate_delivered_welcome(&welcome)
                .map_err(error)?;
            // Verify authenticated MLS membership, not just relay-supplied routing metadata.
            if !self
                .device
                .room_members(room)
                .map_err(error)?
                .iter()
                .any(|member| member.account_id == agent)
            {
                return Err(error("Welcome group is missing the expected agent"));
            }
            self.room = room.to_owned();
            self.after_seq = welcome.commit_seq;
            self.welcome_ack = Some(welcome.welcome_id);
            self.join_package = None;
            self.save_checkpoint().await?;
            activated = true;
        }
        if !activated {
            return Err(error("Agent admission pending; retrying this Device"));
        }
        self.sync().await?;
        Ok(())
    }

    pub async fn send_chat(
        &mut self,
        text: &str,
        topic: Option<String>,
        chat: Option<String>,
        metadata: Option<String>,
    ) -> Result<String, JsValue> {
        if text.trim().is_empty() {
            return Err(error("Enter a message"));
        }
        let mut request = HermesSendRequestV1::from_hermes_send(
            &self.room,
            text,
            None::<String>,
            Default::default(),
        )
        .map_err(error)?;
        request.conversation_id = topic.clone();
        request.segment_id = chat.clone();
        if let Some(metadata) = metadata {
            request.metadata = serde_json::from_str(&metadata).map_err(error)?;
        }
        let event = DecryptedApplicationEventV1 {
            kind: DurableAppEventKind::ChatMessage,
            conversation_id: topic,
            segment_id: chat,
            payload: HermesMessagePayloadV1::from_send(&request)
                .encode()
                .map_err(error)?,
        };
        self.publish(event).await
    }

    /// Organizational actions use the same encrypted durable event protocol as native Core.
    pub async fn publish_event(
        &mut self,
        kind: &str,
        topic: Option<String>,
        payload: &str,
    ) -> Result<String, JsValue> {
        let event = DecryptedApplicationEventV1 {
            kind: serde_json::from_str(kind).map_err(error)?,
            conversation_id: topic,
            segment_id: None,
            payload: payload.as_bytes().to_vec(),
        };
        self.publish(event).await
    }

    async fn publish(&mut self, event: DecryptedApplicationEventV1) -> Result<String, JsValue> {
        if self.room.is_empty() {
            return Err(error("Connect before sending"));
        }
        event.validate_limits().map_err(error)?;
        self.sync().await?;
        self.device.set_now_unix_seconds(now());
        let plaintext = serde_json::to_vec(&event).map_err(error)?;
        self.next_send += 1;
        let request = self
            .device
            .create_application_request(
                &self.room,
                &plaintext,
                format!(
                    "web_{}_{}",
                    self.device.device_ref().device_id,
                    self.next_send
                ),
            )
            .map_err(error)?;
        self.send = Some(PendingSend {
            request: AppendApplicationEventRequest {
                event: request,
                delivery_policy: event.kind.delivery_policy(),
            },
            plaintext,
        });
        // Consumed MLS generation + exact randomized envelope are one durable checkpoint.
        // No network publish is allowed until IndexedDB commits this checkpoint.
        self.save_checkpoint().await?;
        self.resume_pending().await?;
        self.snapshot()
    }

    pub async fn sync(&mut self) -> Result<String, JsValue> {
        self.resume_pending().await?;
        if self.room.is_empty() {
            return self.snapshot();
        }
        self.device.set_now_unix_seconds(now());
        for _ in 0..64 {
            let page: HttpSyncPage = self
                .post(
                    "/sync/group",
                    &GroupSyncRequest {
                        group_id: GroupId::new(self.room.as_bytes().to_vec()),
                        after_seq: self.after_seq,
                        limit: MAX_HTTP_SYNC_PAGE_ENTRIES,
                        requester: Some(MemberId::new(delivery_member_id_for_device(
                            self.device.device_ref(),
                        ))),
                    },
                )
                .await?;
            let advanced = !page.entries.is_empty();
            for queued in page.entries {
                let mut entry: RoomLogEntry = match serde_json::from_slice::<
                    FiniteAccountRoomCommitProjection,
                >(&queued.message.payload)
                {
                    Ok(projection) => projection.entry,
                    Err(_) => serde_json::from_slice(&queued.message.payload).map_err(error)?,
                };
                entry.seq = queued.seq;
                if entry.room_id != self.room || entry.seq <= self.after_seq {
                    return Err(error("Invalid ordered Room log"));
                }
                let applied = match self.device.apply_synced_log_entry(&self.room, &entry) {
                    Ok(applied) => applied,
                    Err(failure) => {
                        if matches!(
                            failure,
                            finitechat_client::ClientStoreError::Client(
                                finitechat_client::ClientError::DeviceStateBehindServer { .. }
                            )
                        ) {
                            self.save_checkpoint().await?;
                        }
                        return Err(error(failure));
                    }
                };
                if let Some(AppliedLogEntry::Application { plaintext, sender }) = applied {
                    self.record_event(&plaintext, entry.message_id.clone(), entry.seq, sender)?;
                }
                self.after_seq = entry.seq;
            }
            if advanced {
                self.save_checkpoint().await?;
            }
            if !page.has_more {
                self.device.finish_room_sync(&self.room).map_err(error)?;
                return self.snapshot();
            }
        }
        Err(error("Room sync exceeded the spike page bound"))
    }

    pub fn snapshot(&self) -> Result<String, JsValue> {
        serde_json::to_string(&serde_json::json!({ "room": self.room,
            "device": self.device.device_ref(), "afterSeq": self.after_seq,
            "epoch": if self.room.is_empty() { 0 } else { self.device.group_epoch(&self.room).map_err(error)? },
            "events": self.events, "pendingSend": self.send.is_some() })).map_err(error)
    }
}

impl BrowserChat {
    async fn save_checkpoint(&self) -> Result<(), JsValue> {
        let result = self
            .persist
            .call1(&JsValue::NULL, &JsValue::from_str(&self.checkpoint()?))?;
        JsFuture::from(js_sys::Promise::resolve(&result)).await?;
        Ok(())
    }

    async fn resume_pending(&mut self) -> Result<(), JsValue> {
        if let Some(id) = &self.welcome_ack {
            let _: serde_json::Value = self
                .post(
                    "/welcomes/ack",
                    &finitechat_http::AckWelcomeRequest {
                        message_id: finitechat_transport::MessageId::new(id.as_bytes().to_vec()),
                    },
                )
                .await?;
            self.device.record_welcome_acknowledged(id).map_err(error)?;
            self.welcome_ack = None;
            self.save_checkpoint().await?;
        }
        if let Some(pending) = &self.send {
            // Always reconcile an uncertain own send BEFORE replaying the Room log:
            // otherwise native sender-currency checks would correctly reject it.
            let receipt: EventAccepted = self.post("/events", &pending.request).await?;
            if receipt.message_id != pending.request.event.envelope.message_id().map_err(error)? {
                return Err(error("Message receipt mismatch"));
            }
            let plaintext = pending.plaintext.clone();
            self.device
                .record_own_send_accepted(&self.room, receipt.seq, &receipt.message_id)
                .map_err(error)?;
            self.record_event(
                &plaintext,
                receipt.message_id,
                receipt.seq,
                self.device.device_ref().clone(),
            )?;
            self.send = None;
            self.save_checkpoint().await?;
        }
        Ok(())
    }

    fn record_event(
        &mut self,
        plaintext: &[u8],
        id: String,
        seq: u64,
        sender: DeviceRef,
    ) -> Result<(), JsValue> {
        let Ok(event) = serde_json::from_slice::<DecryptedApplicationEventV1>(plaintext) else {
            return Ok(());
        };
        event.validate_limits().map_err(error)?;
        self.events.push(BrowserEvent {
            id,
            seq,
            sender,
            timestamp: now(),
            kind: event.kind,
            conversation_id: event.conversation_id,
            segment_id: event.segment_id,
            payload: serde_json::from_slice(&event.payload).unwrap_or(serde_json::Value::Null),
        });
        Ok(())
    }

    async fn post<B: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        value: &B,
    ) -> Result<R, JsValue> {
        let url = if path.starts_with("http://") {
            path.to_owned()
        } else {
            format!("{}{path}", self.server)
        };
        let body = serde_json::to_vec(value).map_err(error)?;
        let auth = finite_nostr::sign_http_auth_header_with_secret(
            self.secret.as_bytes(),
            &finite_nostr::HttpAuthEventRequest::new("POST", &url, now()).with_body(body.clone()),
        )
        .map_err(error)?;
        let response = self
            .http
            .post(url)
            .timeout(std::time::Duration::from_secs(15))
            .header("Content-Type", "application/json")
            .header("Authorization", auth)
            .body(body)
            .send()
            .await
            .map_err(error)?;
        if !response.status().is_success() {
            return Err(error(format!("{path}: HTTP {}", response.status())));
        }
        response.json().await.map_err(error)
    }
}
