//! The SimpleX carry (FIN-463): the old agent's SimpleX conversation with
//! its owner goes to the bound agent, so the owner's phone keeps talking
//! to it with no rescan. The old core (simplex-chat v7.0.2) and the new
//! (v7.0.3) share the database format, so its two databases are the whole
//! conversation (`finite-agentd/simplex_runtime.py`: `-d agent/simplex/identity`).
//!
//! `stream` stages, besides the tree's own paths:
//!
//! - `.finite-stage/simplex/chat.db`: `agent/simplex/identity_chat.db`;
//! - `.finite-stage/simplex/agent.db`: `agent/simplex/identity_agent.db`;
//! - `.finite-stage/simplex/owner.json`: `{"contact": <id>}`, the one
//!   contact Hermes approved.
//!
//! It does so only when every one of these holds; otherwise it stages
//! nothing of SimpleX, and the summary says why ([`Refused`]):
//!
//! - Hermes approved exactly one contact, by a single decimal id;
//! - both databases are there, plain files with one name on the tree's
//!   filesystem, and plain SQLite (their header, so not encrypted); the
//!   folder and the pairing files are on it too, as the walk requires;
//! - neither has a `-journal` or `-wal` beside it, so both are settled,
//!   and still none at the stage's end ([`still_settled`]): the agent's
//!   lease doesn't hold SimpleX's own writers;
//! - together they are at most [`SIMPLEX_MAX_BYTES`].
//!
//! Nothing else of `agent/simplex/` goes: not its address, its files or
//! its relay settings.
//!
//! ## Writers: the hold
//!
//! The agent's lease doesn't hold SimpleX's own writers. What excludes them
//! is Core's hold (FIN-463): the mover streams only once Core's
//! `runtime-stop-exact` has stopped the runtime, and from that stop Core
//! starts it again for nothing but the mover's own restart. So no SimpleX
//! core runs from before `stream` starts until the move is done. The reader
//! can't see Core: the stage id is its word for that hold, since the mover
//! names the stage only after that stop. Without one nothing is carried
//! (`no-stage-id`), whatever the tree looks like.

use std::ffi::CString;
use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt;

use rustix::fs::Stat;
use rustix::io::Errno;
use serde::Serialize;

use crate::error::HostError;
use crate::pairing::{Approved, approved};
use crate::tree::{Tree, open_directory, open_regular, stat_entry};

pub const CHAT_ENTRY: &str = ".finite-stage/simplex/chat.db";
pub const AGENT_ENTRY: &str = ".finite-stage/simplex/agent.db";
pub const OWNER_ENTRY: &str = ".finite-stage/simplex/owner.json";
/// The most the two databases may hold together. A bot with one contact
/// holds a few megabytes; its files stay behind.
pub const SIMPLEX_MAX_BYTES: u64 = 256 * 1024 * 1024;

const FOLDER: [&str; 2] = ["agent", "simplex"];
/// Each database, and the stage entry it becomes.
const DATABASES: [(&str, &str); 2] = [
    ("identity_chat.db", CHAT_ENTRY),
    ("identity_agent.db", AGENT_ENTRY),
];
/// What SQLite leaves beside a database it hasn't settled.
const UNSETTLED: [&str; 2] = ["-journal", "-wal"];
const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

/// Why the conversation stayed behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Refused {
    /// Neither database: the agent never used SimpleX.
    NoSimplex,
    /// One database without the other.
    DatabaseMissing,
    /// Hermes approved no contact.
    NoOwner,
    /// Hermes approved more than one: whose conversation is it?
    SeveralContacts,
    /// A pairing file isn't a plain file of ids.
    PairingUnreadable,
    /// A database is a link, has a second name, or isn't a regular file.
    DatabaseUnreadable,
    /// A database isn't plain SQLite (encrypted, or not a database).
    DatabaseNotPlainSqlite,
    /// A `-journal` or `-wal` beside a database.
    JournalPresent,
    /// Over [`SIMPLEX_MAX_BYTES`].
    TooLarge,
    /// No stage id: nothing says the runtime is held stopped for its move.
    NoStageId,
}

/// The summary's record of the carry (`stream` only).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Carry {
    pub carried: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<Refused>,
}

impl Carry {
    pub fn carried() -> Self {
        Self {
            carried: true,
            why: None,
        }
    }

    pub fn refused(why: Refused) -> Self {
        Self {
            carried: false,
            why: Some(why),
        }
    }
}

/// One database to stage: open, at its start, with the `fstat` it was
/// checked by.
pub struct Database {
    /// Its path in the tree.
    pub source: String,
    pub entry: &'static str,
    pub file: File,
    pub stat: Stat,
}

/// What the carry stages.
pub struct Carried {
    pub contact: i64,
    pub databases: Vec<Database>,
    /// The folder they are in, in `agent/`.
    pub folder: String,
}

impl Carried {
    /// `owner.json`.
    pub fn owner(&self) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ "contact": self.contact }))
            .expect("an id serializes")
    }
}

/// Decide the carry for a stopped tree, staged as `stage`.
pub fn decide(tree: &Tree, stage: Option<&str>) -> Result<Result<Carried, Refused>, HostError> {
    let Some(folder) = folder(tree)? else {
        return Ok(Err(Refused::NoSimplex));
    };
    let mut present = 0;
    for (name, _) in DATABASES {
        if present_in(&folder, name)? {
            present += 1;
        }
    }
    match present {
        0 => return Ok(Err(Refused::NoSimplex)),
        1 => return Ok(Err(Refused::DatabaseMissing)),
        _ => {}
    }
    if stage.is_none() {
        return Ok(Err(Refused::NoStageId));
    }
    let contact = match approved(tree, "simplex")? {
        Approved::Unreadable => return Ok(Err(Refused::PairingUnreadable)),
        Approved::Ids(ids) => {
            let mut ids = ids.into_iter();
            match (ids.next(), ids.next()) {
                (None, _) => return Ok(Err(Refused::NoOwner)),
                (Some(id), None) => id,
                _ => return Ok(Err(Refused::SeveralContacts)),
            }
        }
    };
    let mut databases = Vec::new();
    for (name, entry_name) in DATABASES {
        let source = format!("{}/{name}", FOLDER.join("/"));
        let file = match open_regular(&folder, &c_name(name)?) {
            Ok(Some(file)) => file,
            Ok(None) | Err(Errno::LOOP | Errno::NOENT) => {
                return Ok(Err(Refused::DatabaseUnreadable));
            }
            Err(error) => return Err(HostError::io(source.into(), error.into())),
        };
        if !tree
            .holds(&file)
            .map_err(|error| HostError::io(source.clone().into(), error.into()))?
        {
            return Ok(Err(Refused::DatabaseUnreadable));
        }
        let stat = rustix::fs::fstat(&file)
            .map_err(|error| HostError::io(source.clone().into(), error.into()))?;
        let mut header = [0u8; 16];
        let plain = file.read_exact_at(&mut header, 0).is_ok() && &header == SQLITE_HEADER;
        if !plain {
            return Ok(Err(Refused::DatabaseNotPlainSqlite));
        }
        databases.push(Database {
            source,
            entry: entry_name,
            file,
            stat,
        });
    }
    for (name, _) in DATABASES {
        for suffix in UNSETTLED {
            if present_in(&folder, &format!("{name}{suffix}"))? {
                return Ok(Err(Refused::JournalPresent));
            }
        }
    }
    let total: u64 = databases
        .iter()
        .map(|database| u64::try_from(database.stat.st_size).unwrap_or(u64::MAX))
        .fold(0, u64::saturating_add);
    if total > SIMPLEX_MAX_BYTES {
        return Ok(Err(Refused::TooLarge));
    }
    Ok(Ok(Carried {
        contact,
        databases,
        folder: FOLDER[1].to_owned(),
    }))
}

/// Whether the carried databases in `agent/<folder>` are still settled at
/// the stage's end: no `-journal` or `-wal` beside them. A SimpleX writer
/// that committed meanwhile may leave the database itself as it was, its
/// rows in a WAL the stage doesn't hold.
pub fn still_settled(tree: &Tree, folder: &str) -> Result<bool, HostError> {
    let Some(folder) = folder_at(tree, folder)? else {
        return Ok(false);
    };
    for (name, _) in DATABASES {
        for suffix in UNSETTLED {
            if present_in(&folder, &format!("{name}{suffix}"))? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// `agent/simplex/`, if it is a folder in the tree.
fn folder(tree: &Tree) -> Result<Option<OwnedFd>, HostError> {
    folder_at(tree, FOLDER[1])
}

/// `agent/<name>/`, if it is a folder in the tree, on its filesystem.
fn folder_at(tree: &Tree, name: &str) -> Result<Option<OwnedFd>, HostError> {
    let failed = |error: Errno| HostError::io(FOLDER.join("/").into(), error.into());
    let mut directory = rustix::io::dup(tree.root()).map_err(failed)?;
    for part in [FOLDER[0], name] {
        directory = match open_directory(&directory, &c_name(part)?) {
            Ok(next) => next,
            Err(Errno::NOENT | Errno::NOTDIR | Errno::LOOP) => return Ok(None),
            Err(error) => return Err(failed(error)),
        };
        if !tree.holds(&directory).map_err(failed)? {
            return Ok(None);
        }
    }
    Ok(Some(directory))
}

/// Whether anything at all (of any kind) is at `name` in the folder.
fn present_in(folder: &OwnedFd, name: &str) -> Result<bool, HostError> {
    match stat_entry(folder, &c_name(name)?) {
        Ok(_) => Ok(true),
        Err(Errno::NOENT) => Ok(false),
        Err(error) => Err(HostError::io(
            format!("{}/{name}", FOLDER.join("/")).into(),
            error.into(),
        )),
    }
}

fn c_name(name: &str) -> Result<CString, HostError> {
    CString::new(name).map_err(|_| HostError::BadId)
}
