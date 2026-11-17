//! Brain working trees (FIN-499). `fbrain open` puts each at
//! `workspace/finitebrain/<folder>/` (`FBRAIN_WORKING_TREE_ROOT`). The
//! stage carries every one there, whoever owns it, less its
//! `.finitebrain/` control folder, and beside each writes its name file,
//! `workspace/finitebrain/<folder>.json`, as phase 0's export did
//! (fragment's `finite-computer` import profile reads it):
//!
//! ```json
//! {"name": "Notes", "kind": "personal", "owner_npub": "npub1…", "owner": "person"}
//! ```
//!
//! `name`, `kind` and `owner_npub` are the brain's own manifest's
//! (`.finitebrain/brain-directory.json`), or null. `owner` says whose the
//! brain is to the old agent, so the new one recreates its own and its
//! person's and knows a team's or anyone else's for what it is:
//!
//! - `agent`: a Personal Brain owned by the old agent's own key;
//! - `person`: a Personal Brain owned by one of its person's keys;
//! - `other`: an Organization Brain, or a Personal Brain owned by neither,
//!   when the mover knows both the agent's key and its person's;
//! - `unknown`: anything else, never a guess: a manifest that can't be
//!   read, or an owner that can't be placed.
//!
//! The mover names the keys it knows from Core (`--agent-key`,
//! `--person-key`).

use std::io::Read;
use std::os::fd::OwnedFd;

use serde::Serialize;
use serde_json::{Value, json};

use crate::error::HostError;
use crate::tree::{Tree, open_directory, open_regular};

/// Where `fbrain open` puts brains, relative to the tree.
pub const BRAIN_ROOT: [&str; 2] = ["workspace", "finitebrain"];
/// The most person keys one stream may name.
pub const PERSON_KEYS_MAX: usize = 16;
/// The largest manifest read.
const MANIFEST_MAX_BYTES: u64 = 1024 * 1024;
/// The longest name, kind or owner taken from a manifest, in bytes
/// (finite-brain's names are at most 128).
const FIELD_MAX: usize = 256;

/// The name file of the brain in `workspace/finitebrain/<folder>/`.
pub fn name_path(folder: &str) -> String {
    format!("{}/{folder}.json", BRAIN_ROOT.join("/"))
}

/// What a brain's manifest says of it (finite-brain-core's
/// `BrainDirectoryManifest`, version 1: `brain.name`, `brain.kind`,
/// `brain.ownerNpub`). A field that isn't short text is `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    pub name: Option<String>,
    pub kind: Option<String>,
    pub owner_npub: Option<String>,
}

/// The keys the mover knows, each as its 32 bytes: the old agent's own
/// (Core's pin of the key it reports health with), and its person's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OwnerKeys {
    pub agent: Option<[u8; 32]>,
    pub people: Vec<[u8; 32]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Owner {
    Agent,
    Person,
    Other,
    Unknown,
}

/// Whose a brain is to the old agent (the module's list).
pub fn owner(manifest: Option<&Manifest>, keys: &OwnerKeys) -> Owner {
    let Some(manifest) = manifest else {
        return Owner::Unknown;
    };
    match manifest.kind.as_deref() {
        Some("organization") => return Owner::Other,
        Some("personal") => {}
        _ => return Owner::Unknown,
    }
    let Some(key) = manifest.owner_npub.as_deref().and_then(npub_key) else {
        return Owner::Unknown;
    };
    if keys.agent == Some(key) {
        Owner::Agent
    } else if keys.people.contains(&key) {
        Owner::Person
    } else if keys.agent.is_some() && !keys.people.is_empty() {
        Owner::Other
    } else {
        Owner::Unknown
    }
}

/// The name file's text.
pub fn name_file(manifest: Option<&Manifest>, keys: &OwnerKeys) -> Vec<u8> {
    let field = |get: fn(&Manifest) -> &Option<String>| manifest.and_then(|m| get(m).clone());
    let body = json!({
        "name": field(|m| &m.name),
        "kind": field(|m| &m.kind),
        "owner_npub": field(|m| &m.owner_npub),
        "owner": owner(manifest, keys),
    });
    serde_json::to_vec(&body).expect("a name file serializes")
}

/// The manifest of the brain working tree `brain`, if it can be read: a
/// plain file with one name, on the tree's filesystem, of at most
/// `MANIFEST_MAX_BYTES`, whose JSON has a `brain` object.
pub fn read_manifest(tree: &Tree, brain: &OwnedFd) -> Option<Manifest> {
    let control = open_directory(brain, c".finitebrain").ok()?;
    let file = open_regular(&control, c"brain-directory.json").ok()??;
    if !tree.holds(&file).ok()? {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MANIFEST_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MANIFEST_MAX_BYTES {
        return None;
    }
    let manifest: Value = serde_json::from_slice(&bytes).ok()?;
    let brain = manifest.get("brain")?.as_object()?;
    let text = |key: &str| {
        brain
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| {
                !text.is_empty() && text.len() <= FIELD_MAX && !text.chars().any(char::is_control)
            })
            .map(str::to_owned)
    };
    Some(Manifest {
        name: text("name"),
        kind: text("kind"),
        owner_npub: text("ownerNpub"),
    })
}

/// A key as the mover sends it: 64 hex digits.
pub fn parse_key(hex_key: &str) -> Result<[u8; 32], HostError> {
    let refused = || HostError::Usage("a key is 64 hex digits".into());
    if hex_key.len() != 64 {
        return Err(refused());
    }
    hex::decode(hex_key)
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())
}

/// An `npub1…`'s 32 bytes.
fn npub_key(npub: &str) -> Option<[u8; 32]> {
    let (hrp, data) = bech32::decode(npub).ok()?;
    if hrp.as_str() != "npub" {
        return None;
    }
    data.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn npub(byte: u8) -> String {
        let hrp = bech32::Hrp::parse("npub").unwrap();
        bech32::encode::<bech32::Bech32>(hrp, &[byte; 32]).unwrap()
    }

    fn personal(owner_npub: Option<String>) -> Manifest {
        Manifest {
            name: Some("Notes".into()),
            kind: Some("personal".into()),
            owner_npub,
        }
    }

    /// Each `owner`, and that a brain whose owner can't be placed is
    /// `unknown`, never a guess.
    #[test]
    fn each_brain_is_placed_by_the_keys_the_mover_knows() {
        let known = OwnerKeys {
            agent: Some([1; 32]),
            people: vec![[2; 32], [3; 32]],
        };
        let organization = Manifest {
            kind: Some("organization".into()),
            ..Manifest::default()
        };
        for (manifest, keys, expected) in [
            (Some(personal(Some(npub(1)))), &known, Owner::Agent),
            (Some(personal(Some(npub(3)))), &known, Owner::Person),
            (Some(organization.clone()), &known, Owner::Other),
            (Some(organization), &OwnerKeys::default(), Owner::Other),
            (Some(personal(Some(npub(9)))), &known, Owner::Other),
            (None, &known, Owner::Unknown),
            (Some(personal(None)), &known, Owner::Unknown),
            (
                Some(personal(Some("npub1nonsense".into()))),
                &known,
                Owner::Unknown,
            ),
            (
                Some(personal(Some(npub(2).replacen("npub", "nsec", 1)))),
                &known,
                Owner::Unknown,
            ),
            (
                Some(Manifest {
                    kind: Some("shared".into()),
                    ..personal(Some(npub(1)))
                }),
                &known,
                Owner::Unknown,
            ),
        ] {
            assert_eq!(owner(manifest.as_ref(), keys), expected, "{manifest:?}");
        }
        // Without its person's keys, or its own, a brain neither key owns
        // can't be placed.
        let agent_only = OwnerKeys {
            agent: Some([1; 32]),
            people: vec![],
        };
        let people_only = OwnerKeys {
            agent: None,
            people: vec![[2; 32]],
        };
        let else_owned = personal(Some(npub(9)));
        assert_eq!(owner(Some(&else_owned), &agent_only), Owner::Unknown);
        assert_eq!(owner(Some(&else_owned), &people_only), Owner::Unknown);
        assert_eq!(
            owner(Some(&personal(Some(npub(1)))), &agent_only),
            Owner::Agent
        );
        assert_eq!(
            owner(Some(&personal(Some(npub(2)))), &people_only),
            Owner::Person
        );
    }

    #[test]
    fn a_name_file_holds_the_manifest_and_the_owner() {
        let keys = OwnerKeys {
            agent: Some([1; 32]),
            people: vec![],
        };
        let named: Value =
            serde_json::from_slice(&name_file(Some(&personal(Some(npub(1)))), &keys)).unwrap();
        assert_eq!(
            named,
            json!({"name": "Notes", "kind": "personal", "owner_npub": npub(1), "owner": "agent"})
        );
        let unread: Value = serde_json::from_slice(&name_file(None, &keys)).unwrap();
        assert_eq!(
            unread,
            json!({"name": null, "kind": null, "owner_npub": null, "owner": "unknown"})
        );
    }

    #[test]
    fn keys_are_64_hex_digits() {
        assert_eq!(parse_key(&"0a".repeat(32)).unwrap(), [10; 32]);
        for bad in ["", "0a", &"0a".repeat(33), &"zz".repeat(32), &npub(1)] {
            assert!(parse_key(bad).is_err(), "{bad}");
        }
    }
}
