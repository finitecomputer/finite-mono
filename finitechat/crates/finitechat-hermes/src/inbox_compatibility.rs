//! Read-only handoff check for the sidecar inbox. Run only after every writer
//! using the volume has stopped. Missing image capability means legacy.

use serde_json::Value;

pub const READER_LABEL: &str = "computer.finite.chat.inbox_reader";
pub const REFUSAL_V1_READER: &str = "refusal-v1";

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
                Some("refusal_v1") if reader == Some(REFUSAL_V1_READER) => {
                    if let Some(prepared) = lease.get("operation").and_then(|op| op.get("prepared"))
                        && !prepared.is_null() && prepared.get("version").and_then(Value::as_u64) != Some(1) {
                        return Err("unsupported prepared refusal version; runtime replacement blocked".into());
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
