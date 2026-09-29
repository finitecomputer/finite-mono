//! Preparation and exact replay for a bridge-owned durable reply. The bridge
//! must atomically persist this opaque value before submitting it. Ordinary
//! chat sends keep their synchronous, non-queued contract.
//!
//! Replay fence. Ordinary sends mint and append on one stack and are never
//! resubmitted later. A saved ciphertext is: replayed after an arbitrary
//! delay, it lands behind any later generations this Device got accepted in
//! the meantime, can fall outside receivers' out-of-order window, and then
//! freezes the room for every receiver. Every submit first proves, in this
//! call, that this store synced the room to the server head (a sync that
//! returned no error and no quarantine, then an empty probe page after the
//! cursor). Only then are a positive receipt recorded or an unaccepted reply
//! replayed. An unaccepted reply is replayed only while this store proves no
//! later generation was minted or accepted in the room since prepare:
//!
//! * the saved id is still the newest minted id. Ids enter the list at mint
//!   and leave only on accept (which raises the mark) or FIFO eviction
//!   (oldest first, so the saved id leaves before any later one);
//! * the own-send mark equals the value captured at prepare. Every accept
//!   this store records, synchronously or by paging, raises it past any seq
//!   that existed at prepare. A rewound store is refused by the currency
//!   fence, which the head sync has just fed;
//! * the room group and epoch still match the envelope.
//!
//! The fence is conservative and assumes one live lineage per Device: a
//! refused reply stays with its owner for repair and is never re-encrypted
//! here.

use super::*;
use finitechat_client::HttpRuntimeTransport;
use finitechat_http::{ApplicationEffectRequest, HttpApplicationDeliveryEffect};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedBridgeReply {
    version: u32,
    server_url: String,
    message: PreparedOutboundMessage,
    /// Room own-send mark right after minting. `0` is a valid mark (no own
    /// send accepted yet).
    own_send_high_water_seq: u64,
}

/// The only wrapper shape this build writes or submits. Any persisted shape
/// change bumps this together with the inbox reader label.
const PREPARED_BRIDGE_REPLY_VERSION: u32 = 2;

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
        let own_send_high_water_seq = self
            .core
            .device
            .export_state()
            .map_err(client_error)?
            .rooms
            .iter()
            .find(|room| room.room_id == room_id)
            .map(|room| room.own_send_high_water_seq)
            .ok_or_else(|| bridge_actor_error("prepared reply room is missing"))?;
        Ok(PreparedBridgeReply {
            version: PREPARED_BRIDGE_REPLY_VERSION,
            server_url: self.core.room_server_url(&room_id),
            message,
            own_send_high_water_seq,
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
        if prepared.version != PREPARED_BRIDGE_REPLY_VERSION
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
        // high-water mark, and a replay needs the fence below evaluated on
        // head state. Both require the head sync first.
        self.sync_bridge_room_to_head(&message.room_id, &prepared.server_url)?;
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
                let minted = &room.unacknowledged_own_message_ids;
                if !minted.contains(&message.message_id) {
                    return Err(bridge_actor_error(
                        "prepared reply was not minted by this retained Device state",
                    ));
                }
                if minted.last() != Some(&message.message_id)
                    || prepared.own_send_high_water_seq != room.own_send_high_water_seq
                {
                    return Err(bridge_actor_error(
                        "prepared reply was overtaken by a later send from this Device; \
                         replay could be undecryptable for receivers",
                    ));
                }
                let envelope = &message.append_request.envelope;
                if envelope.mls_group_id != room.mls_group_id
                    || envelope.epoch
                        != self
                            .core
                            .device
                            .group_epoch(&message.room_id)
                            .map_err(client_error)?
                {
                    return Err(bridge_actor_error(
                        "prepared reply is from a previous room epoch",
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

    /// Sync `room_id` and prove, in this call, that the store reached the
    /// server head: no delivery error, no quarantined room, currency fence
    /// clear, then one probe page after the cursor that is empty, final, and
    /// does not move the cursor. Any unseen entry fails closed; the caller
    /// retries and the next sync continues from the advanced cursor. The
    /// ordinary send gate is unchanged.
    fn sync_bridge_room_to_head(
        &mut self,
        room_id: &str,
        server_url: &str,
    ) -> Result<(), FiniteChatCoreError> {
        let earlier = std::mem::take(&mut self.core.deferred_projection);
        let mut projection = match self.core.sync_room_with_projection(room_id) {
            Ok(projection) => projection,
            Err(error) => {
                self.core.deferred_projection = earlier;
                return Err(error);
            }
        };
        let quarantined = projection
            .room_sync_failures
            .iter()
            .any(|failure| failure.room_id == room_id);
        projection.merge_earlier(earlier);
        self.core.deferred_projection = projection;
        self.core
            .device
            .ensure_current_for_send(room_id)
            .map_err(|error| send_error(room_id, error))?;
        if quarantined {
            return Err(bridge_actor_error("room sync did not complete"));
        }
        let after_seq = self
            .core
            .device
            .last_applied_seq(room_id)
            .map_err(client_error)?;
        let owner = self.core.device.device_ref().clone();
        let page = self
            .core
            .delivery_for(server_url)
            .sync_events(room_id, &owner, after_seq)
            .map_err(delivery_error)?;
        if !page.entries.is_empty() || page.has_more || page.next_after_seq != after_seq {
            return Err(bridge_actor_error(
                "room sync has not reached the server head",
            ));
        }
        Ok(())
    }
}
