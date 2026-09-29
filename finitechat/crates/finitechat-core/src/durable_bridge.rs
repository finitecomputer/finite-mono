//! Preparation and exact replay for a bridge-owned durable reply. The bridge
//! must atomically persist this opaque value before submitting it. Ordinary
//! chat sends keep their synchronous, non-queued contract.

use super::*;
use finitechat_client::HttpRuntimeTransport;
use finitechat_http::{ApplicationEffectRequest, HttpApplicationDeliveryEffect};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedBridgeReply {
    version: u32,
    server_url: String,
    message: PreparedOutboundMessage,
}

impl FiniteChatRuntime {
    /// Ratchet and persist Device state, but do not submit the message. Store
    /// the returned value durably before calling `submit_bridge_reply_and_wait`.
    pub fn prepare_bridge_reply_and_wait(
        &self,
        room_id: String,
        plaintext: Vec<u8>,
    ) -> Result<PreparedBridgeReply, FiniteChatCoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::PrepareBridgeReply {
                room_id,
                plaintext,
                response: tx,
            })
            .map_err(bridge_actor_error)?;
        rx.recv().map_err(bridge_actor_error)?
    }

    /// Recover acceptance or replay the saved request byte-for-byte. An error
    /// is never permission to prepare a replacement reply.
    pub fn submit_bridge_reply_and_wait(
        &self,
        prepared: PreparedBridgeReply,
    ) -> Result<AppSentMessage, FiniteChatCoreError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::SubmitBridgeReply {
                prepared: Box::new(prepared),
                response: tx,
            })
            .map_err(bridge_actor_error)?;
        rx.recv().map_err(bridge_actor_error)?
    }
}

fn bridge_actor_error(error: impl std::fmt::Display) -> FiniteChatCoreError {
    FiniteChatCoreError::Client {
        reason: format!("bridge reply actor: {error}"),
    }
}

impl AppRuntimeState {
    pub(super) fn prepare_bridge_reply(
        &mut self,
        room_id: String,
        plaintext: Vec<u8>,
    ) -> Result<PreparedBridgeReply, FiniteChatCoreError> {
        if self.core.read_only {
            return Err(FiniteChatCoreError::ReadOnly);
        }
        if !self.room_is_connected(&room_id) {
            return Err(bridge_actor_error("room is not ready to send"));
        }
        let (plaintext, _, _) = self.scope_unscoped_chat_event_to_default(&room_id, plaintext)?;
        let message = self
            .core
            .prepare_outbound_chat_message(&room_id, plaintext)?;
        Ok(PreparedBridgeReply {
            version: 1,
            server_url: self.core.room_server_url(&room_id),
            message,
        })
    }

    pub(super) fn submit_bridge_reply(
        &mut self,
        prepared: &PreparedBridgeReply,
    ) -> Result<AppSentMessage, FiniteChatCoreError> {
        if self.core.read_only {
            return Err(FiniteChatCoreError::ReadOnly);
        }
        let message = &prepared.message;
        if prepared.version != 1
            || message.sender != *self.core.device.device_ref()
            || prepared.server_url != self.core.room_server_url(&message.room_id)
            || message.append_request.sender != message.sender
            || message.append_request.room_id != message.room_id
            || message.append_request.timestamp_unix_seconds != message.timestamp_unix_seconds
            || message
                .append_request
                .envelope
                .message_id()
                .map_err(client_error)?
                != message.message_id
        {
            return Err(bridge_actor_error(
                "prepared reply identity, version, or server mismatch",
            ));
        }
        if !matches!(
            decode_application_event(&message.plaintext),
            DecodedAppEvent::ChatMessage(_)
        ) {
            return Err(bridge_actor_error("prepared reply is not a chat message"));
        }
        // A saved envelope is not authority to advance a rewound Device's
        // high-water mark. Sync and enforce the existing currency fence first.
        self.core.currency_gate_before_send(&message.room_id)?;
        self.core
            .device
            .ensure_current_for_send(&message.room_id)
            .map_err(|error| send_error(&message.room_id, error))?;
        let device = self.core.device.export_state().map_err(client_error)?;
        let room = device
            .rooms
            .iter()
            .find(|room| room.room_id == message.room_id)
            .ok_or_else(|| bridge_actor_error("prepared reply room is missing"))?;
        // Epoch validation precedes replay detection on the server. Recover a
        // positive receipt first, including when acceptance happened before a
        // crash or the sender's local projection failed. A negative/failed
        // lookup never causes re-encryption.
        let mut delivery = self.core.delivery_for(&prepared.server_url);
        let effect: Option<HttpApplicationDeliveryEffect> = delivery
            .transport_mut()
            .post_json(
                "/application-effects/get",
                &ApplicationEffectRequest {
                    message_id: message.message_id.clone(),
                },
            )
            .map_err(delivery_error)?;
        let (accepted, projection) = match effect {
            Some(effect) => {
                if effect.room_id != message.room_id
                    || effect.sender != message.sender
                    || effect.message_id != message.message_id
                    || effect.delivery_policy != DurableAppEventKind::ChatMessage.delivery_policy()
                {
                    return Err(bridge_actor_error("prepared reply receipt mismatch"));
                }
                self.core.project_accepted_chat_message(
                    message,
                    EventAccepted {
                        seq: effect.seq,
                        message_id: effect.message_id,
                    },
                )?
            }
            None => {
                if !room
                    .unacknowledged_own_message_ids
                    .contains(&message.message_id)
                {
                    return Err(bridge_actor_error(
                        "prepared reply was not minted by this retained Device state",
                    ));
                }
                self.core.submit_prepared_chat_message(message)?
            }
        };
        self.apply_projection_events(projection.events)?;
        self.append_messages(projection.result.messages, projection.attachment_blobs);
        Ok(AppSentMessage {
            message_id: accepted.message_id,
            seq: accepted.seq,
        })
    }
}
