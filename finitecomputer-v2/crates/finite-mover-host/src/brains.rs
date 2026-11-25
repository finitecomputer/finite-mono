//! Brain working trees (FIN-499). `fbrain open` puts each at
//! `workspace/finitebrain/<folder>/` (`FBRAIN_WORKING_TREE_ROOT`). The
//! stage carries every one there, whoever owns it, less its
//! `.finitebrain/` control folder, and beside each writes its name file,
//! `workspace/finitebrain/<folder>.json`, as phase 0's export did
//! (fragment's `finite-computer` import profile reads it):
//!
//! ```json
//! {"name": "Notes", "kind": "personal", "owner_npub": "npub1…", "owner": "person",
//!  "versions": {"sequence": 41, "folders": [{"path": "Field Notes", "read": true}],
//!               "pages": [{"path": "Field Notes/a.md", "revision": 3, "sha256": "…"}]}}
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
//!
//! `versions` (FIN-773) is the working tree's own record of what it holds,
//! from `.finitebrain/working-tree-state.json` (finite-brain-core's
//! `WorkingTreeState`, `finite-brain-working-tree-state-v1`), so the new
//! agent can tell which of two copies of a team's brain is newer:
//!
//! - `sequence`: the brain's latest change the tree had synced;
//! - `folders`: each folder root, by its path in the tree (a child
//!   folder's is under its parent's, `Parent/Child`), and whether the old
//!   agent could `read` it. One it could not lists no pages, even when the
//!   tree still holds files from before it lost access: nothing with no
//!   version is ever newer than anything;
//! - `pages`: each page the tree held, by its path in the tree (its
//!   folder's path, `/`, its own), with the brain's `revision` of it and
//!   the SHA-256 of that revision's text, as hex.
//!
//! A staged page whose bytes hash to another `sha256` was edited in the
//! tree after `revision` and never synced. `versions` is null when the
//! record is missing, of another version, over a limit, or malformed in
//! any part: whole or not at all, never a guess. It holds no key and no
//! encrypted sync data, only paths, numbers and hashes of text the stage
//! carries anyway.
//!
//! The name file is JSON with a line per value, so no unbroken run in it
//! is longer than a path: the uploader's second check scans text in
//! chunks and can't check one longer than `SCAN_CARRY_MAX`.

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::io::Read;
use std::os::fd::OwnedFd;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::HostError;
use crate::tree::{Tree, open_directory, open_regular};

/// Where `fbrain open` puts brains, relative to the tree.
pub const BRAIN_ROOT: [&str; 2] = ["workspace", "finitebrain"];
/// The most person keys one stream may name.
pub const PERSON_KEYS_MAX: usize = 16;
/// The largest manifest read.
const MANIFEST_MAX_BYTES: u64 = 1024 * 1024;
/// The largest working tree state read (a brain of 100,000 pages is about
/// 35 MB of it).
const STATE_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// The working tree state's version `versions` reads.
const STATE_VERSION: &str = "finite-brain-working-tree-state-v1";
/// The most pages and top folders one brain's `versions` lists.
pub const PAGES_MAX: usize = 100_000;
pub const FOLDERS_MAX: usize = 1_000;
/// The longest path in `versions`, in bytes.
const PATH_MAX: usize = 4096;
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

/// What the stage reads of a brain working tree's control folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub manifest: Option<Manifest>,
    pub versions: Option<Versions>,
}

/// A working tree's record of what it holds (the module's `versions`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Versions {
    pub sequence: u64,
    pub folders: Vec<Folder>,
    pub pages: Vec<Page>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Folder {
    pub path: String,
    pub read: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Page {
    pub path: String,
    pub revision: u64,
    pub sha256: String,
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

/// The name file's text, with `versions` (null when there are none).
pub fn name_file(
    manifest: Option<&Manifest>,
    versions: Option<&Versions>,
    keys: &OwnerKeys,
) -> Vec<u8> {
    let field = |get: fn(&Manifest) -> &Option<String>| manifest.and_then(|m| get(m).clone());
    let body = json!({
        "name": field(|m| &m.name),
        "kind": field(|m| &m.kind),
        "owner_npub": field(|m| &m.owner_npub),
        "owner": owner(manifest, keys),
        "versions": versions,
    });
    serde_json::to_vec_pretty(&body).expect("a name file serializes")
}

/// What the brain working tree `brain`'s control folder says of it.
pub fn read(tree: &Tree, brain: &OwnedFd) -> Facts {
    let Ok(control) = open_directory(brain, c".finitebrain") else {
        return Facts::default();
    };
    Facts {
        manifest: read_control(tree, &control, c"brain-directory.json", MANIFEST_MAX_BYTES)
            .and_then(|bytes| manifest(&bytes)),
        versions: read_control(tree, &control, c"working-tree-state.json", STATE_MAX_BYTES)
            .and_then(|bytes| versions(&bytes)),
    }
}

/// A control file's bytes, if it can be read: a plain file with one name,
/// on the tree's filesystem, of at most `max` bytes.
fn read_control(tree: &Tree, control: &OwnedFd, name: &CStr, max: u64) -> Option<Vec<u8>> {
    let file = open_regular(control, name).ok()??;
    if !tree.holds(&file).ok()? {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= max).then_some(bytes)
}

/// The manifest, if its JSON has a `brain` object.
fn manifest(bytes: &[u8]) -> Option<Manifest> {
    let manifest: Value = serde_json::from_slice(bytes).ok()?;
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

/// The working tree state as `versions`, if every part of it is as
/// finite-brain-core writes it and within the limits.
fn versions(bytes: &[u8]) -> Option<Versions> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct State {
        version: String,
        folder_roots: Vec<Root>,
        objects: Vec<Object>,
        sync: SyncState,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Root {
        folder_id: String,
        source_brain_id: Option<String>,
        path: String,
        can_read: bool,
        metadata_only: bool,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Object {
        folder_id: String,
        source_brain_id: Option<String>,
        path: String,
        revision: u64,
        content_hash: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct SyncState {
        latest_sequence: u64,
    }

    let state: State = serde_json::from_slice(bytes).ok()?;
    if state.version != STATE_VERSION
        || state.folder_roots.len() > FOLDERS_MAX
        || state.objects.len() > PAGES_MAX
    {
        return None;
    }
    // finite-brain-core keys a root by its brain and its folder: a tree
    // can mount two brains' folders of one id.
    let mut roots = BTreeMap::new();
    let mut folders = Vec::with_capacity(state.folder_roots.len());
    for root in state.folder_roots {
        let read = root.can_read && !root.metadata_only;
        if !clean_path(&root.path) {
            return None;
        }
        if roots
            .insert(
                (root.source_brain_id, root.folder_id),
                (root.path.clone(), read),
            )
            .is_some()
        {
            return None;
        }
        folders.push(Folder {
            path: root.path,
            read,
        });
    }
    let mut pages = Vec::with_capacity(state.objects.len());
    for object in state.objects {
        let (root, read) = roots.get(&(object.source_brain_id, object.folder_id))?;
        // A folder the old agent could not read holds no text in the tree.
        if !read {
            continue;
        }
        let path = format!("{root}/{}", object.path);
        if !clean_path(&object.path) || path.len() > PATH_MAX || !sha256_hex(&object.content_hash) {
            return None;
        }
        pages.push(Page {
            path,
            revision: object.revision,
            sha256: object.content_hash,
        });
    }
    folders.sort_by(|a, b| a.path.cmp(&b.path));
    pages.sort_by(|a, b| a.path.cmp(&b.path));
    if pages.windows(2).any(|pair| pair[0].path == pair[1].path)
        || folders.windows(2).any(|pair| pair[0].path == pair[1].path)
    {
        return None;
    }
    Some(Versions {
        sequence: state.sync.latest_sequence,
        folders,
        pages,
    })
}

/// A relative path of `/`-separated names, none empty, `.` or `..`, with
/// no control character, of at most `PATH_MAX` bytes.
fn clean_path(path: &str) -> bool {
    path.len() <= PATH_MAX
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|name| !name.is_empty() && name != "." && name != "..")
}

/// 64 lowercase hex digits.
fn sha256_hex(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
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
            serde_json::from_slice(&name_file(Some(&personal(Some(npub(1)))), None, &keys))
                .unwrap();
        assert_eq!(
            named,
            json!({"name": "Notes", "kind": "personal", "owner_npub": npub(1), "owner": "agent", "versions": null})
        );
        let unread: Value = serde_json::from_slice(&name_file(None, None, &keys)).unwrap();
        assert_eq!(
            unread,
            json!({"name": null, "kind": null, "owner_npub": null, "owner": "unknown", "versions": null})
        );
        let versions = versions(state().to_string().as_bytes()).unwrap();
        let with: Value = serde_json::from_slice(&name_file(None, Some(&versions), &keys)).unwrap();
        assert_eq!(with["versions"]["sequence"], 41);
        assert_eq!(with["versions"]["pages"][0]["path"], "Field Notes/a.md");
    }

    const HASH_A: &str = "0f244b0aef5c92d0aafbb7619adb2faece8d2c15da5ed51a8efd8702fd69e51f";
    const HASH_B: &str = "70043c6f2ae84be0bb0f5bd4aaae0d06fa21a7ec97901ea3219bf462becf4794";

    /// `working-tree-state.json` as finite-brain-core writes it: two
    /// folders the agent reads, one it doesn't, and three pages (one in
    /// the folder it can't read, with no text).
    fn state() -> Value {
        json!({
            "version": "finite-brain-working-tree-state-v1",
            "folderRoots": [
                {"folderId": "programs", "path": "Programs Grants", "canRead": true, "metadataOnly": false},
                {"folderId": "notes", "path": "Field Notes", "canRead": true, "metadataOnly": false},
                {"folderId": "lead", "path": "Leadership", "canRead": false, "metadataOnly": true},
            ],
            "objects": [
                {"folderId": "programs", "path": "raw/b.md", "objectId": "obj_2", "revision": 7, "keyVersion": 1, "contentType": "text/markdown", "contentHash": HASH_B},
                {"folderId": "notes", "path": "a.md", "objectId": "obj_1", "revision": 3, "keyVersion": 1, "contentType": "text/markdown", "contentHash": HASH_A},
                {"folderId": "lead", "path": "secret-plan.md", "objectId": "obj_3", "revision": 1, "keyVersion": 1, "contentType": "text/markdown", "contentHash": ""},
            ],
            "sync": {"latestSequence": 41},
        })
    }

    /// Valid: each page by its path in the tree, sorted, with its revision
    /// and hash; a folder the agent can't read is listed unread and none
    /// of its pages are.
    #[test]
    fn versions_are_the_trees_own_record() {
        let read = versions(state().to_string().as_bytes()).unwrap();
        assert_eq!(read.sequence, 41);
        assert_eq!(
            read.folders,
            vec![
                Folder {
                    path: "Field Notes".into(),
                    read: true
                },
                Folder {
                    path: "Leadership".into(),
                    read: false
                },
                Folder {
                    path: "Programs Grants".into(),
                    read: true
                },
            ]
        );
        assert_eq!(
            read.pages,
            vec![
                Page {
                    path: "Field Notes/a.md".into(),
                    revision: 3,
                    sha256: HASH_A.into()
                },
                Page {
                    path: "Programs Grants/raw/b.md".into(),
                    revision: 7,
                    sha256: HASH_B.into()
                },
            ]
        );
    }

    /// Invalid: any part not as finite-brain-core writes it, or over a
    /// limit, and there are no versions at all, never a part of them.
    #[test]
    fn versions_are_whole_or_none() {
        let broken = |change: &dyn Fn(&mut Value)| {
            let mut state = state();
            change(&mut state);
            versions(state.to_string().as_bytes())
        };
        type Change<'a> = &'a dyn Fn(&mut Value);
        let cases: [(&str, Change); 12] = [
            ("another version", &|s| {
                s["version"] = json!("finite-brain-working-tree-state-v2")
            }),
            ("no sequence", &|s| s["sync"] = json!({})),
            ("a negative revision", &|s| {
                s["objects"][0]["revision"] = json!(-1)
            }),
            ("a short hash", &|s| {
                s["objects"][0]["contentHash"] = json!("0f24")
            }),
            ("an uppercase hash", &|s| {
                s["objects"][0]["contentHash"] = json!(HASH_A.to_uppercase())
            }),
            ("a page in no folder", &|s| {
                s["objects"][0]["folderId"] = json!("gone")
            }),
            ("a page climbing out", &|s| {
                s["objects"][0]["path"] = json!("../../agent/x.md")
            }),
            ("an absolute page", &|s| {
                s["objects"][0]["path"] = json!("/etc/x.md")
            }),
            ("a control character", &|s| {
                s["objects"][0]["path"] = json!("a\nb.md")
            }),
            ("a folder root climbing out", &|s| {
                s["folderRoots"][0]["path"] = json!("a/../../b")
            }),
            ("one folder twice", &|s| {
                s["folderRoots"][1]["folderId"] = json!("programs")
            }),
            ("one page twice", &|s| {
                s["objects"][1]["folderId"] = json!("programs");
                s["objects"][1]["path"] = json!("raw/b.md");
            }),
        ];
        for (why, change) in cases {
            assert_eq!(broken(change), None, "{why}");
        }
        assert_eq!(versions(b"not json"), None);
        let too_long = format!("{}.md", "a".repeat(PATH_MAX));
        assert_eq!(
            broken(&|s| s["objects"][0]["path"] = json!(too_long)),
            None,
            "a long path"
        );
    }

    /// Over the page limit, none; at it, all.
    #[test]
    fn versions_hold_at_most_the_page_limit() {
        let with_pages = |count: usize| {
            let mut state = state();
            state["objects"] = (0..count)
                .map(|n| json!({"folderId": "notes", "path": format!("{n}.md"), "revision": 1, "contentHash": HASH_A}))
                .collect();
            versions(state.to_string().as_bytes())
        };
        assert_eq!(with_pages(PAGES_MAX).unwrap().pages.len(), PAGES_MAX);
        assert_eq!(with_pages(PAGES_MAX + 1), None);
    }

    /// Valid: a child folder's root is under its parent's, and two brains'
    /// folders of one id are two folders.
    #[test]
    fn versions_take_nested_roots_and_two_brains_folders() {
        let mut state = state();
        state["folderRoots"] = json!([
            {"folderId": "notes", "path": "Field Notes", "canRead": true, "metadataOnly": false},
            {"folderId": "talks", "path": "Field Notes/Interviews", "canRead": true, "metadataOnly": false},
            {"folderId": "notes", "sourceBrainId": "brain_team", "path": "Team Notes", "canRead": true, "metadataOnly": false},
        ]);
        state["objects"] = json!([
            {"folderId": "notes", "path": "a.md", "revision": 3, "contentHash": HASH_A},
            {"folderId": "talks", "path": "b.md", "revision": 4, "contentHash": HASH_B},
            {"folderId": "notes", "sourceBrainId": "brain_team", "path": "a.md", "revision": 9, "contentHash": HASH_B},
        ]);
        let read = versions(state.to_string().as_bytes()).unwrap();
        let paths: Vec<(&str, u64)> = read
            .pages
            .iter()
            .map(|page| (page.path.as_str(), page.revision))
            .collect();
        assert_eq!(
            paths,
            [
                ("Field Notes/Interviews/b.md", 4),
                ("Field Notes/a.md", 3),
                ("Team Notes/a.md", 9)
            ]
        );
        // A page of a brain the tree mounts no root of is in no folder.
        state["objects"][2]["sourceBrainId"] = json!("brain_gone");
        assert_eq!(versions(state.to_string().as_bytes()), None);
    }

    /// A name file of the most pages passes the uploader's second check:
    /// no unbroken run in it is longer than a path. Without a line per
    /// value, the same brain's would be one run over the scan's carry.
    #[test]
    fn a_name_file_of_the_most_pages_is_checkable() {
        let versions = Versions {
            sequence: 41,
            folders: vec![],
            pages: (0..PAGES_MAX)
                .map(|n| Page {
                    path: format!(
                        "Programs_Grants/raw/2026-11-{n:06}-board-meeting-interview-transcript.md"
                    ),
                    revision: 7,
                    sha256: HASH_A.into(),
                })
                .collect(),
        };
        let body = name_file(None, Some(&versions), &OwnerKeys::default());
        let scan = |body: &[u8]| crate::shapes::scan(body).unwrap();
        assert_eq!(scan(&body), crate::shapes::TextVerdict::Clean);
        let compact = serde_json::to_vec(&serde_json::from_slice::<Value>(&body).unwrap()).unwrap();
        assert!(compact.len() > 16 * 1024 * 1024, "{}", compact.len());
        assert_eq!(scan(&compact), crate::shapes::TextVerdict::Uncheckable);
    }

    #[test]
    fn keys_are_64_hex_digits() {
        assert_eq!(parse_key(&"0a".repeat(32)).unwrap(), [10; 32]);
        for bad in ["", "0a", &"0a".repeat(33), &"zz".repeat(32), &npub(1)] {
            assert!(parse_key(bad).is_err(), "{bad}");
        }
    }
}
