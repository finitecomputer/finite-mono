//! The inbox owns refusal delivery, including recovery when the adapter or
//! sidecar dies. A persisted prepared reply is never replaced after an
//! uncertain outcome. Network calls run outside the inbox lock.

use super::*;
use finitechat_core::PreparedBridgeReply;
use finitechat_hermes::HermesSendKindV1;

const MAX_PENDING: usize = 32;
const MAX_REFUSAL_BYTES: usize = 8 * 1024;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Operation {
    request: HermesSendRequestV1,
    prepared: Option<PreparedBridgeReply>,
    attempts: u32,
    retry_at_ms: u64,
    last_error: Option<String>,
}

pub(super) fn diagnostics(state: &HermesServiceState) -> Result<Value, CliError> {
    let _guard = lock_service_mutex(&state.inbox_lock)?;
    let inbox = load_hermes_inbox(&state.agent_home)?;
    let operations: Vec<_> = inbox
        .events
        .iter()
        .filter_map(|entry| match &entry.lease {
            HermesInboxLease::RefusalV1 { operation } => Some(json!({
                "room_id": entry.room_id, "seq": entry.seq, "message_id": entry.message_id,
                "attempts": operation.attempts, "retry_at_ms": operation.retry_at_ms,
                "last_error": operation.last_error,
            })),
            _ => None,
        })
        .collect();
    Ok(
        json!({"capability": "durable_refusal_v1", "pending": operations.len(),
        "legacy_downgrade_ready": operations.is_empty(), "operations": operations}),
    )
}

#[derive(Deserialize)]
struct RefuseRequest {
    #[serde(flatten)]
    inbound: HermesAckRequestV1,
    text: String,
}

pub(super) fn begin(state: &HermesServiceState, payload: Value) -> Result<Value, CliError> {
    let request: RefuseRequest = serde_json::from_value(payload).map_err(CliError::Json)?;
    request
        .inbound
        .validate_limits()
        .map_err(|e| CliError::Hermes(e.to_string()))?;
    if request.text.trim().is_empty() || request.text.len() > MAX_REFUSAL_BYTES {
        return Err(CliError::Hermes("invalid refusal text".into()));
    }
    let key = hermes_inbox_key(
        &request.inbound.room_id,
        request.inbound.seq,
        &request.inbound.message_id,
    );
    {
        let _guard = lock_service_mutex(&state.inbox_lock)?;
        let mut inbox = load_hermes_inbox(&state.agent_home)?;
        if inbox_key_recently_acked(&inbox, &key) {
            return Ok(json!({"acked": true, "durable": true}));
        }
        let pending = inbox
            .events
            .iter()
            .filter(|e| matches!(e.lease, HermesInboxLease::RefusalV1 { .. }))
            .count();
        let entry = inbox
            .events
            .iter_mut()
            .find(|entry| entry.key == key)
            .ok_or_else(|| CliError::Hermes("refusal requires a live inbox entry".into()))?;
        if let HermesInboxLease::RefusalV1 { operation } = &entry.lease
            && operation.request.text != request.text
        {
            return Err(CliError::Hermes(
                "refusal decision already persisted; cannot replace it".into(),
            ));
        }
        if !matches!(entry.lease, HermesInboxLease::RefusalV1 { .. }) {
            if pending >= MAX_PENDING {
                return Err(CliError::Hermes(
                    "durable refusal capacity reached; retry later".into(),
                ));
            }
            // The canonical inbound event owns the route. The caller supplies
            // only its identity and the policy explanation, never a destination.
            let send = HermesSendRequestV1 {
                room_id: entry.room_id.clone(),
                conversation_id: entry.event.conversation_id.clone(),
                segment_id: entry.event.segment_id.clone(),
                thread_id: if entry.event.conversation_id.is_none()
                    && entry.event.segment_id.is_none()
                {
                    entry.event.source.thread_id.clone()
                } else {
                    None
                },
                text: request.text,
                kind: HermesSendKindV1::Message,
                status: HermesMessageStatusV1::Complete,
                attachments: Vec::new(),
                reply_to_message_id: Some(entry.message_id.clone()),
                metadata: BTreeMap::new(),
            };
            entry.lease = HermesInboxLease::RefusalV1 {
                operation: Box::new(Operation {
                    request: send,
                    prepared: None,
                    attempts: 0,
                    retry_at_ms: 0,
                    last_error: None,
                }),
            };
            save_hermes_inbox(&state.agent_home, &inbox)?;
        }
    }
    // Acceptance of ownership is durable. A process-owned worker drives the
    // operation, so client cancellation cannot cancel delivery or settlement.
    signal_bridge_update(state);
    Ok(json!({"acked": false, "durable": true}))
}

pub(super) fn start_recovery(state: HermesServiceState) -> Result<(), CliError> {
    std::thread::Builder::new()
        .name("finitechat-refusal-recovery".into())
        .spawn(move || {
            loop {
                if let Err(error) = recover(&state) {
                    eprintln!("[finitechat] durable refusal recovery: {error}");
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .map(|_| ())
        .map_err(|error| CliError::Hermes(error.to_string()))
}

fn recover(state: &HermesServiceState) -> Result<(), CliError> {
    let _worker = lock_service_mutex(&state.refusal_lock)?;
    let pending: Vec<_> = {
        let _guard = lock_service_mutex(&state.inbox_lock)?;
        load_hermes_inbox(&state.agent_home)?
            .events
            .into_iter()
            .filter_map(|entry| match entry.lease {
                HermesInboxLease::RefusalV1 { operation } if operation.retry_at_ms <= now_ms() => {
                    Some((entry.key, *operation))
                }
                _ => None,
            })
            .take(1)
            .collect()
    };
    for (key, mut operation) in pending {
        if let Err(error) = advance(state, &key, &mut operation) {
            operation.attempts = operation.attempts.saturating_add(1);
            operation.retry_at_ms = now_ms().saturating_add(
                1_000_u64
                    .saturating_mul(1_u64 << operation.attempts.min(8))
                    .min(300_000),
            );
            operation.last_error = Some(error.to_string());
            // Never consume an entry because a failure is classified terminal.
            // Keep its exact request and diagnostic for forward recovery.
            persist(state, &key, &operation)?;
        }
    }
    Ok(())
}

fn advance(
    state: &HermesServiceState,
    key: &str,
    operation: &mut Operation,
) -> Result<(), CliError> {
    advance_with(state, key, operation, || Ok(()))
}

fn advance_with(
    state: &HermesServiceState,
    key: &str,
    operation: &mut Operation,
    after_submit: impl FnOnce() -> Result<(), CliError>,
) -> Result<(), CliError> {
    if operation.prepared.is_none() {
        resolve_hermes_send_route(&state.runtime, &mut operation.request)?;
        let payload = HermesMessagePayloadV1::from_send(&operation.request)
            .encode()
            .map_err(|error| CliError::Hermes(error.to_string()))?;
        let plaintext = encode_application_event(
            DurableAppEventKind::ChatMessage,
            operation.request.conversation_id.clone(),
            operation.request.segment_id.clone(),
            &payload,
        )?;
        let prepared = state
            .runtime
            .prepare_bridge_reply_and_wait(operation.request.room_id.clone(), plaintext)?;
        if serde_json::to_vec(&prepared)
            .map_err(CliError::Serialize)?
            .len()
            > 256 * 1024
        {
            return Err(CliError::Hermes(
                "prepared refusal exceeds storage limit".into(),
            ));
        }
        operation.prepared = Some(prepared);
        persist(state, key, operation)?;
    }
    let sent = state
        .runtime
        .submit_bridge_reply_and_wait(operation.prepared.clone().expect("prepared"))?;
    after_submit()?;
    settle(
        state,
        key,
        finitechat_proto::EventAccepted {
            message_id: sent.message_id,
            seq: sent.seq,
        },
    )
}

fn persist(state: &HermesServiceState, key: &str, operation: &Operation) -> Result<(), CliError> {
    let _guard = lock_service_mutex(&state.inbox_lock)?;
    let mut inbox = load_hermes_inbox(&state.agent_home)?;
    let entry = inbox
        .events
        .iter_mut()
        .find(|entry| entry.key == key)
        .ok_or_else(|| CliError::Hermes("durable refusal entry disappeared".into()))?;
    if !matches!(entry.lease, HermesInboxLease::RefusalV1 { .. }) {
        return Err(CliError::Hermes("durable refusal lost ownership".into()));
    }
    entry.lease = HermesInboxLease::RefusalV1 {
        operation: Box::new(operation.clone()),
    };
    save_hermes_inbox(&state.agent_home, &inbox)
}

fn settle(
    state: &HermesServiceState,
    key: &str,
    accepted: finitechat_proto::EventAccepted,
) -> Result<(), CliError> {
    let _guard = lock_service_mutex(&state.inbox_lock)?;
    let mut inbox = load_hermes_inbox(&state.agent_home)?;
    if !inbox
        .events
        .iter()
        .any(|entry| entry.key == key && matches!(entry.lease, HermesInboxLease::RefusalV1 { .. }))
    {
        return Err(CliError::Hermes(
            "durable refusal lost ownership before settlement".into(),
        ));
    }
    inbox.events.retain(|entry| entry.key != key);
    record_hermes_inbox_acked(&mut inbox, key, now_ms());
    if let Some(acked) = inbox.acked.iter_mut().find(|entry| entry.key == key) {
        acked.refusal_reply = Some(accepted);
    }
    save_hermes_inbox(&state.agent_home, &inbox)?;
    signal_bridge_update(state);
    Ok(())
}
