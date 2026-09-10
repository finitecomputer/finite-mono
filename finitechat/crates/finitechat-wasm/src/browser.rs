use finitechat_client::{AppliedLogEntry, FiniteChatDevice, FiniteChatDeviceConfig};
use finitechat_delivery::{HttpClaimedKeyPackage, HttpSyncPage, MAX_HTTP_SYNC_PAGE_ENTRIES};
use finitechat_hermes::{HermesMessagePayloadV1, HermesSendRequestV1};
use finitechat_http::{
    BootstrapAccountRoomRequest, BootstrapAccountRoomResponse, ClaimKeyPackageForAccountRequest,
    FiniteAccountRoomCommitProjection, GroupSyncRequest,
};
use finitechat_mls::NostrSecretKey;
use finitechat_proto::{
    AppendApplicationEventRequest, ClaimKeyPackageResult, CommitAccepted, DurableAppEventKind,
    EventAccepted, LogEntryKind, RoomLogEntry, UploadKeyPackageRequest,
    delivery_member_id_for_device, lease_token_for,
};
use finitechat_proto::{DecryptedApplicationEventV1, DeviceRef};
use finitechat_transport::{GroupId, MemberId};
use serde::{Serialize, de::DeserializeOwned};
use wasm_bindgen::prelude::*;

fn error(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}
fn now() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

#[derive(Serialize)]
struct Message {
    id: String,
    sender: String,
    text: String,
}

#[derive(Serialize)]
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

#[wasm_bindgen]
pub struct BrowserChat {
    device: FiniteChatDevice,
    secret: NostrSecretKey,
    server: String,
    http: reqwest::Client,
    room: String,
    after_seq: u64,
    messages: Vec<Message>,
    events: Vec<BrowserEvent>,
}

#[wasm_bindgen]
impl BrowserChat {
    #[wasm_bindgen(constructor)]
    pub fn new(nsec: &str, server: &str, device_id: &str) -> Result<BrowserChat, JsValue> {
        let key = nostr::SecretKey::parse(nsec).map_err(error)?;
        let secret = NostrSecretKey::from_bytes(key.to_secret_bytes()).map_err(error)?;
        let device = FiniteChatDevice::new(FiniteChatDeviceConfig {
            account_secret_key: secret.clone(),
            device_id: device_id.to_owned(),
            now_unix_seconds: now(),
            credential_not_before_unix_seconds: now() - 60,
            credential_not_after_unix_seconds: now() + 86400,
        })
        .map_err(error)?;
        Ok(Self {
            device,
            secret,
            server: server.trim_end_matches('/').to_owned(),
            http: reqwest::Client::new(),
            room: String::new(),
            after_seq: 0,
            messages: Vec::new(),
            events: Vec::new(),
        })
    }

    /// The browser owns Room creation, MLS admission, Commit and Welcome encryption.
    pub async fn connect(&mut self, agent_account_id: &str, room_id: &str) -> Result<(), JsValue> {
        if !self.room.is_empty() {
            return Err(error("Already connected; reload for a fresh Device"));
        }
        self.device.set_now_unix_seconds(now());
        let claimed: Option<HttpClaimedKeyPackage> = self
            .post(
                "/key-packages/claim-account",
                &ClaimKeyPackageForAccountRequest {
                    account_id: agent_account_id.to_owned(),
                },
            )
            .await?;
        let claimed = claimed.ok_or_else(|| error("Agent has no available KeyPackage"))?;
        let upload: UploadKeyPackageRequest =
            serde_json::from_slice(claimed.key_package.bytes()).map_err(error)?;
        if upload.owner.account_id != agent_account_id
            || claimed.key_package_id.as_slice() != upload.key_package_id.as_bytes()
        {
            return Err(error("Agent KeyPackage identity mismatch"));
        }
        let claim = ClaimKeyPackageResult {
            lease_token: lease_token_for(&upload.key_package_id, &upload.owner),
            key_package_id: upload.key_package_id,
            owner: upload.owner,
            key_package_ref: upload.key_package_ref,
            key_package_hash: upload.key_package_hash,
            key_package_payload: upload.key_package_payload,
        };
        let group_id = format!("mls_{room_id}");
        self.device
            .create_group_state(room_id, &group_id)
            .map_err(error)?;
        self.room = room_id.to_owned();
        let _: BootstrapAccountRoomResponse = self
            .post(
                "/account-rooms/bootstrap",
                &BootstrapAccountRoomRequest {
                    room_id: self.room.clone(),
                    mls_group_id: group_id,
                    creator: self.device.device_ref().clone(),
                    protocol: Default::default(),
                },
            )
            .await?;
        let commit = self
            .device
            .prepare_add_member_commit(
                room_id,
                &claim,
                format!("welcome_{room_id}"),
                format!("admit_{room_id}"),
            )
            .map_err(error)?;
        let accepted: CommitAccepted = self.post("/commits", &commit.request).await?;
        if accepted.message_id != commit.message_id {
            return Err(error("Commit receipt mismatch"));
        }
        self.sync().await?;
        Ok(())
    }

    pub async fn send(&mut self, text: &str) -> Result<String, JsValue> {
        self.send_chat(text, None, None, None).await
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
        let request = self
            .device
            .create_application_request(
                &self.room,
                &plaintext,
                format!(
                    "web_{}_{}",
                    self.device.device_ref().device_id,
                    js_sys::Date::now()
                ),
            )
            .map_err(error)?;
        let expected_id = request.envelope.message_id().map_err(error)?;
        let receipt: EventAccepted = self
            .post(
                "/events",
                &AppendApplicationEventRequest {
                    event: request,
                    delivery_policy: event.kind.delivery_policy(),
                },
            )
            .await?;
        if receipt.message_id != expected_id {
            return Err(error("Message receipt mismatch"));
        }
        self.device
            .record_own_send_accepted(&self.room, receipt.seq, &receipt.message_id)
            .map_err(error)?;
        self.record_event(
            &plaintext,
            receipt.message_id,
            receipt.seq,
            self.device.device_ref().clone(),
        )?;
        self.snapshot()
    }

    pub async fn sync(&mut self) -> Result<String, JsValue> {
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
                if !(entry.kind == LogEntryKind::Application
                    && entry.sender == *self.device.device_ref())
                    && let AppliedLogEntry::Application { plaintext, sender } = self
                        .device
                        .apply_log_entry(&self.room, &entry)
                        .map_err(error)?
                {
                    self.record_event(&plaintext, entry.message_id.clone(), entry.seq, sender)?;
                }
                self.after_seq = entry.seq;
            }
            if !page.has_more {
                return self.snapshot();
            }
        }
        Err(error("Room sync exceeded the spike page bound"))
    }

    pub fn snapshot(&self) -> Result<String, JsValue> {
        serde_json::to_string(&serde_json::json!({ "room": self.room,
            "device": self.device.device_ref(), "afterSeq": self.after_seq,
            "epoch": if self.room.is_empty() { 0 } else { self.device.group_epoch(&self.room).map_err(error)? },
            "messages": self.messages, "events": self.events })).map_err(error)
    }
}

impl BrowserChat {
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
        if let Some(text) = crate::event_text(plaintext) {
            self.messages.push(Message {
                id: id.clone(),
                sender: sender.account_id.clone(),
                text,
            });
        }
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
        let url = format!("{}{path}", self.server);
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
