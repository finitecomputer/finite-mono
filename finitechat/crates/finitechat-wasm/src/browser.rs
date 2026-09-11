use finitechat_client::{
    AppliedLogEntry, FiniteChatDevice, FiniteChatDeviceConfig, FiniteChatDeviceState,
};
use finitechat_delivery::{HttpClaimedKeyPackage, HttpSyncPage, MAX_HTTP_SYNC_PAGE_ENTRIES};
use finitechat_hermes::{HermesMessagePayloadV1, HermesSendRequestV1};
use finitechat_http::{
    BootstrapAccountRoomRequest, BootstrapAccountRoomResponse, ClaimKeyPackageForAccountRequest,
    FiniteAccountRoomCommitProjection, GroupSyncRequest,
};
use finitechat_mls::NostrSecretKey;
use finitechat_proto::SubmitCommitRequest;
use finitechat_proto::{
    AppendApplicationEventRequest, ClaimKeyPackageResult, CommitAccepted, DurableAppEventKind,
    EventAccepted, RoomLogEntry, UploadKeyPackageRequest, delivery_member_id_for_device,
    lease_token_for,
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
struct BootstrapPlan {
    request: BootstrapAccountRoomRequest,
    claim: ClaimKeyPackageResult,
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
    bootstrap: Option<BootstrapPlan>,
    commit: Option<SubmitCommitRequest>,
    send: Option<PendingSend>,
    next_send: u64,
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
    bootstrap: Option<BootstrapPlan>,
    commit: Option<SubmitCommitRequest>,
    send: Option<PendingSend>,
    next_send: u64,
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
            bootstrap: saved.as_ref().and_then(|s| s.bootstrap.clone()),
            commit: saved.as_ref().and_then(|s| s.commit.clone()),
            send: saved.as_ref().and_then(|s| s.send.clone()),
            next_send: saved.as_ref().map(|s| s.next_send).unwrap_or_default(),
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
            bootstrap: self.bootstrap.clone(),
            commit: self.commit.clone(),
            send: self.send.clone(),
            next_send: self.next_send,
        })
        .map_err(error)
    }

    /// The browser owns Room creation, MLS admission, Commit and Welcome encryption.
    pub async fn connect(&mut self, agent_account_id: &str, room_id: &str) -> Result<(), JsValue> {
        if !self.room.is_empty() {
            self.sync().await?;
            return Ok(());
        }
        // Pin this Device before the first network mutation.
        self.save_checkpoint().await?;
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
        self.bootstrap = Some(BootstrapPlan {
            request: BootstrapAccountRoomRequest {
                room_id: self.room.clone(),
                mls_group_id: group_id,
                creator: self.device.device_ref().clone(),
                protocol: Default::default(),
            },
            claim,
        });
        self.save_checkpoint().await?;
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
        if let Some(plan) = &self.bootstrap {
            let _: BootstrapAccountRoomResponse =
                self.post("/account-rooms/bootstrap", &plan.request).await?;
            let commit = self
                .device
                .prepare_add_member_commit(
                    &self.room,
                    &plan.claim,
                    format!("welcome_{}", self.room),
                    format!("admit_{}", self.room),
                )
                .map_err(error)?;
            self.commit = Some(commit.request);
            self.bootstrap = None;
            self.save_checkpoint().await?;
        }
        if let Some(commit) = &self.commit {
            let receipt: CommitAccepted = self.post("/commits", commit).await?;
            if receipt.message_id != commit.envelope.message_id().map_err(error)? {
                return Err(error("Commit receipt mismatch"));
            }
            self.commit = None;
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
