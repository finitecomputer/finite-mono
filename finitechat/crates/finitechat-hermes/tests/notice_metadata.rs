//! an inference route notice rides the ordinary `message` kind with
//! `metadata.finite_notice`. These tests push the adapter's exact send payload
//! through the real sidecar request type and the room payload an old or new
//! peer decodes, and pin that a message without metadata is unchanged.

use finitechat_hermes::{
    HermesMessagePayloadV1, HermesMessageStatusV1, HermesSendKindV1, HermesSendRequestV1,
};
use serde_json::{Value, json};

const NOTICE_TEXT: &str =
    "Finite Private answered this response because OpenRouter is out of credits or quota.";

fn finite_notice() -> Value {
    json!({
        "v": 1,
        "type": "inference_fallback",
        "attempted": "openrouter",
        "served_by": "finite_private",
        "reason": "billing",
    })
}

/// The body `FiniteChatAdapter._send_payload` builds for a notice.
fn adapter_send_body(text: &str, metadata: Option<Value>) -> Value {
    let mut body = json!({
        "room_id": "room-agent-1",
        "conversation_id": "topic-1",
        "segment_id": "chat-a",
        "thread_id": null,
        "text": text,
        "kind": "message",
        "status": "complete",
        "attachments": [],
        "reply_to_message_id": null,
    });
    if let Some(metadata) = metadata {
        body["metadata"] = metadata;
    }
    body
}

fn room_round_trip(request: &HermesSendRequestV1) -> (Value, HermesMessagePayloadV1) {
    let encoded = HermesMessagePayloadV1::from_send(request)
        .encode()
        .expect("room payload encodes");
    let wire: Value = serde_json::from_slice(&encoded).expect("room payload is JSON");
    let decoded = HermesMessagePayloadV1::decode(&encoded)
        .expect("room payload decodes")
        .expect("room payload is a hermes message");
    (wire, decoded)
}

#[test]
fn notice_metadata_survives_the_sidecar_request_and_room_payload() {
    let body = adapter_send_body(
        NOTICE_TEXT,
        Some(json!({ "finite_notice": finite_notice() })),
    );
    let request: HermesSendRequestV1 =
        serde_json::from_value(body).expect("sidecar accepts the notice send body");
    request.validate_limits().expect("notice is within limits");
    assert_eq!(request.kind, HermesSendKindV1::Message);
    assert_eq!(request.status, HermesMessageStatusV1::Complete);
    assert_eq!(
        request.metadata.get("finite_notice"),
        Some(&finite_notice())
    );

    let (wire, decoded) = room_round_trip(&request);
    assert_eq!(wire["kind"], "message");
    assert_eq!(
        wire["metadata"],
        json!({ "finite_notice": finite_notice() })
    );
    // A peer that ignores `finite_notice` still has the whole sentence.
    assert_eq!(decoded.text, NOTICE_TEXT);
    assert_eq!(decoded.kind, HermesSendKindV1::Message);
    assert_eq!(decoded.status, HermesMessageStatusV1::Complete);
    assert_eq!(
        decoded.metadata.get("finite_notice"),
        Some(&finite_notice())
    );
    assert!(!decoded.metadata.contains_key("notify"));
}

#[test]
fn every_notice_shape_is_accepted() {
    for (notice_type, attempted, served_by, reason) in [
        (
            "inference_fallback",
            "openai_codex",
            json!("finite_private"),
            json!(null),
        ),
        (
            "inference_backup",
            "other",
            json!("backup"),
            json!("timeout"),
        ),
        (
            "inference_fallback_failed",
            "openrouter",
            json!(null),
            json!("billing"),
        ),
    ] {
        let notice = json!({
            "v": 1,
            "type": notice_type,
            "attempted": attempted,
            "served_by": served_by,
            "reason": reason,
        });
        let body = adapter_send_body("notice", Some(json!({ "finite_notice": notice.clone() })));
        let request: HermesSendRequestV1 = serde_json::from_value(body).expect(notice_type);
        let (_, decoded) = room_round_trip(&request);
        assert_eq!(decoded.metadata.get("finite_notice"), Some(&notice));
    }
}

#[test]
fn a_message_without_metadata_is_unchanged() {
    for body in [
        adapter_send_body("the answer", None),
        adapter_send_body("the answer", Some(json!({}))),
    ] {
        let request: HermesSendRequestV1 =
            serde_json::from_value(body).expect("sidecar accepts a plain message");
        assert!(request.metadata.is_empty());
        let (wire, decoded) = room_round_trip(&request);
        assert!(
            wire.get("metadata").is_none(),
            "a plain message gains no metadata field: {wire}"
        );
        assert_eq!(
            wire,
            json!({
                "type": "finitechat.hermes.message.v1",
                "conversation_id": "topic-1",
                "segment_id": "chat-a",
                "text": "the answer",
                "kind": "message",
                "status": "complete",
            })
        );
        assert!(decoded.metadata.is_empty());
    }
}

#[test]
fn a_notice_kind_would_be_rejected_which_is_why_notices_use_metadata() {
    let mut body = adapter_send_body(NOTICE_TEXT, None);
    body["kind"] = json!("notice");
    assert!(serde_json::from_value::<HermesSendRequestV1>(body).is_err());
    assert_eq!(HermesSendKindV1::parse("notice"), None);
}
