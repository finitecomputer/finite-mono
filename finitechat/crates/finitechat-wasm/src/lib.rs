//! Feasibility spike: the real FiniteChatDevice runs in the browser.
//! Native persistence and the app runtime are deliberately outside this slice.
use finitechat_hermes::{HermesMessagePayloadV1, HermesSendRequestV1};
use finitechat_proto::{DecryptedApplicationEventV1, DurableAppEventKind};

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
pub use browser::BrowserChat;

/// Use the same durable chat payload understood by the agent's Hermes adapter.
pub fn text_event(room: &str, text: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let request =
        HermesSendRequestV1::from_hermes_send(room, text, None::<String>, Default::default())?;
    Ok(serde_json::to_vec(&DecryptedApplicationEventV1 {
        kind: DurableAppEventKind::ChatMessage,
        conversation_id: None,
        segment_id: None,
        payload: HermesMessagePayloadV1::from_send(&request).encode()?,
    })?)
}

pub fn event_text(bytes: &[u8]) -> Option<String> {
    let event: DecryptedApplicationEventV1 = serde_json::from_slice(bytes).ok()?;
    if event.kind != DurableAppEventKind::ChatMessage {
        return None;
    }
    HermesMessagePayloadV1::decode(&event.payload)
        .ok()??
        .text
        .into()
}
