//! Read-only handoff check for the sidecar inbox. Run only after every writer
//! using the volume has stopped. Missing image capability means legacy.

use serde_json::Value;

pub const READER_LABEL: &str = "computer.finite.chat.inbox_reader";
/// Images whose sidecar reads prepared refusal replies at wrapper version 2,
/// which carry the room's own-send watermark. The inbox lease tag stays
/// `refusal_v1`; this label names the prepared wrapper the reader accepts.
pub const REFUSAL_V2_READER: &str = "refusal-v2";

pub fn check(raw: &[u8], reader: Option<&str>) -> Result<(), String> {
    let inbox: Value =
        serde_json::from_slice(raw).map_err(|e| format!("unreadable chat inbox: {e}"))?;
    if !inbox.is_object() {
        return Err("chat inbox must be an object".into());
    }
    let Some(events) = inbox.get("events") else {
        return Ok(());
    };
    let events = events
        .as_array()
        .ok_or_else(|| "chat inbox events must be an array".to_owned())?;
    for entry in events {
        match entry.get("lease") {
            None => {}, // Legacy files predate leases.
            Some(lease) => match lease.get("state").and_then(Value::as_str) {
                Some("pending" | "leased") => {},
                Some("refusal_v1") if reader == Some(REFUSAL_V2_READER) => {
                    if let Some(prepared) = lease.get("operation").and_then(|op| op.get("prepared"))
                        && !prepared.is_null() {
                        if prepared.get("version").and_then(Value::as_u64) != Some(2) {
                            return Err("unsupported prepared refusal version; runtime replacement blocked".into());
                        }
                        if prepared.get("own_send_high_water_seq").and_then(Value::as_u64).is_none() {
                            return Err("prepared refusal lacks its send watermark; runtime replacement blocked".into());
                        }
                    }
                },
                Some("refusal_v1") => return Err("pending durable command refusals require a capable runtime; drain or repair forward before downgrade".into()),
                _ => return Err("unknown chat inbox lease version; runtime replacement blocked".into()),
            },
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
