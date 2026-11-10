//! Hermes' approved contacts for one platform: `<platform>-approved.json`,
//! keyed by the contact's id, in the folder Hermes keeps it in now
//! (`platforms/pairing/`) and the one it kept it in before (`pairing/`).
//! The owner notice reads Telegram's; the SimpleX carry reads SimpleX's.

use std::collections::BTreeSet;

use crate::error::HostError;
use crate::tree::{Tree, Whole};

/// The largest pairing file read.
const PAIRING_MAX_BYTES: u64 = 64 * 1024;

/// A platform's approved contacts, from both folders together.
#[derive(Debug, PartialEq, Eq)]
pub enum Approved {
    Ids(BTreeSet<i64>),
    /// A pairing file is there but isn't a plain file of ids that can be
    /// read whole. It is never taken for no file: it may name someone.
    Unreadable,
}

pub fn approved(tree: &Tree, platform: &str) -> Result<Approved, HostError> {
    let name = format!("{platform}-approved.json");
    let folders: [&[&str]; 2] = [
        &["agent", "hermes-home", "platforms", "pairing"],
        &["agent", "hermes-home", "pairing"],
    ];
    let mut ids = BTreeSet::new();
    for folder in folders {
        let path: Vec<&str> = folder.iter().copied().chain([name.as_str()]).collect();
        let bytes = match tree.read_whole(&path, PAIRING_MAX_BYTES)? {
            Whole::Absent => continue,
            Whole::Unreadable => return Ok(Approved::Unreadable),
            Whole::Bytes(bytes) => bytes,
        };
        // Hermes keys approved contacts by id; anything else is not a pairing.
        let Ok(serde_json::Value::Object(approved)) = serde_json::from_slice(&bytes) else {
            return Ok(Approved::Unreadable);
        };
        for key in approved.keys() {
            // A positive id, written as Hermes writes it.
            match key.parse::<i64>() {
                Ok(id) if id > 0 && id.to_string() == *key => ids.insert(id),
                _ => return Ok(Approved::Unreadable),
            };
        }
    }
    Ok(Approved::Ids(ids))
}
