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
//! - both databases are there, plain files with one name, and plain SQLite
//!   (their header, so not encrypted);
//! - neither has a `-journal` or `-wal` beside it, so both are settled;
//! - together they are at most [`SIMPLEX_MAX_BYTES`].
//!
//! Nothing else of `agent/simplex/` goes: not its address, its files or
//! its relay settings. `stream` already refuses a running agent.
//!
//! ## The seal: the reader's one write in an agent's tree
//!
//! If the old core ever ran again on its copy, the conversation would fork:
//! messages lost, and the old bot able to talk to the owner. So once the
//! carried entries are staged and checked, still holding the stopped
//! agent's lease, `stream` seals SimpleX where every later start of the
//! old agent sees it ([`seal`]):
//!
//! - `agent/simplex` is renamed `agent/simplex.moved-<stage id>`, never
//!   over anything, never deleted, so it can be undone by hand. With its
//!   identity gone, `simplex_runtime.py`'s `supervise()` fails closed;
//! - managed SimpleX is turned off in `agent/hermes-home/config.yaml`
//!   (`gateway.platforms.simplex.enabled: false`), rewritten whole by a
//!   rename that keeps its owner and mode, only if it was on.
//!
//! Restarting the old agent brings back everything else, never SimpleX.
//! A stage after that carries from the sealed folder again and seals
//! nothing more. Without a stage id there is nothing to name the seal by,
//! so an unsealed conversation is not carried at all. The summary records
//! the sealed folder.

use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt;

use rustix::fs::{
    AtFlags, Dir, FileType, Gid, Mode, OFlags, RenameFlags, Stat, Uid, fchmod, fchown, fsync,
    openat, renameat, renameat_with, unlinkat,
};
use rustix::io::Errno;
use serde::Serialize;

use crate::config_copy::CONFIG_MAX_BYTES;
use crate::error::HostError;
use crate::pairing::{Approved, approved};
use crate::tree::{Tree, open_directory, open_regular, stat_entry};

pub const CHAT_ENTRY: &str = ".finite-stage/simplex/chat.db";
pub const AGENT_ENTRY: &str = ".finite-stage/simplex/agent.db";
pub const OWNER_ENTRY: &str = ".finite-stage/simplex/owner.json";
/// The most the two databases may hold together. A bot with one contact
/// holds a few megabytes; its files stay behind.
pub const SIMPLEX_MAX_BYTES: u64 = 256 * 1024 * 1024;

const AGENT: &str = "agent";
const FOLDER: &str = "simplex";
/// A sealed folder: `simplex.moved-<stage id>`, beside where it was.
const SEALED_PREFIX: &str = "simplex.moved-";
/// The Hermes config, and the temporary file it is rewritten through.
const HERMES_HOME: &str = "hermes-home";
const CONFIG: &str = "config.yaml";
const CONFIG_REWRITE: &str = ".config.yaml.finite-mover-seal";
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
    /// No stage id to seal by, and it isn't sealed yet.
    NoStageId,
    /// More than one sealed folder and no `agent/simplex`: which is it?
    SeveralSealed,
}

/// The summary's record of the carry (`stream` only).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Carry {
    pub carried: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<Refused>,
    /// The folder the old SimpleX is sealed in, `agent/simplex.moved-<id>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sealed: Option<String>,
}

impl Carry {
    pub fn carried(sealed: String) -> Self {
        Self {
            carried: true,
            why: None,
            sealed: Some(sealed),
        }
    }

    pub fn refused(why: Refused) -> Self {
        Self {
            carried: false,
            why: Some(why),
            sealed: None,
        }
    }
}

/// Where the conversation is, and the seal to make once it is staged.
pub struct Seal {
    agent: OwnedFd,
    /// `agent/simplex`, to rename to this sealed name; or already sealed.
    rename_to: Option<String>,
    /// The sealed folder's name in `agent/`.
    sealed: String,
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

/// What the carry stages, and the seal to make after.
pub struct Carried {
    pub contact: i64,
    pub databases: Vec<Database>,
    pub seal: Seal,
}

impl Carried {
    /// `owner.json`.
    pub fn owner(&self) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ "contact": self.contact }))
            .expect("an id serializes")
    }
}

/// Decide the carry for a stopped tree, to be sealed by `stage`.
pub fn decide(tree: &Tree, stage: Option<&str>) -> Result<Result<Carried, Refused>, HostError> {
    let Some(agent) = open_in(tree.root(), AGENT)? else {
        return Ok(Err(Refused::NoSimplex));
    };
    // The conversation is in `agent/simplex`, or, after a stage, in the one
    // folder it was sealed in.
    let (name, rename_to) = if open_in(&agent, FOLDER)?.is_some() {
        let sealed = stage.map(|stage| format!("{SEALED_PREFIX}{stage}"));
        (FOLDER.to_owned(), sealed)
    } else {
        let mut sealed = sealed_folders(&agent)?;
        match (sealed.pop(), sealed.pop()) {
            (None, _) => return Ok(Err(Refused::NoSimplex)),
            (Some(one), None) => (one, None),
            _ => return Ok(Err(Refused::SeveralSealed)),
        }
    };
    let Some(folder) = open_in(&agent, &name)? else {
        return Ok(Err(Refused::NoSimplex));
    };
    let path = format!("{AGENT}/{name}");
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
    let sealed = match (&rename_to, name == FOLDER) {
        (Some(sealed), _) => sealed.clone(),
        (None, true) => return Ok(Err(Refused::NoStageId)),
        (None, false) => name.clone(),
    };
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
        let source = format!("{path}/{name}");
        let file = match open_regular(&folder, &c_name(name)?) {
            Ok(Some(file)) => file,
            Ok(None) | Err(Errno::LOOP | Errno::NOENT) => {
                return Ok(Err(Refused::DatabaseUnreadable));
            }
            Err(error) => return Err(HostError::io(source.into(), error.into())),
        };
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
        seal: Seal {
            agent,
            rename_to,
            sealed,
        },
    }))
}

/// Seal the old SimpleX after its conversation was staged: the folder
/// renamed aside (unless it already was) and managed SimpleX turned off.
/// Returns the sealed folder's path.
pub fn seal(tree: &Tree, seal: Seal) -> Result<String, HostError> {
    let path = format!("{AGENT}/{}", seal.sealed);
    if let Some(rename_to) = &seal.rename_to {
        let failed = |error: Errno| HostError::io(path.clone().into(), error.into());
        renameat_with(
            &seal.agent,
            FOLDER,
            &seal.agent,
            rename_to.as_str(),
            RenameFlags::NOREPLACE,
        )
        .map_err(failed)?;
        fsync(&seal.agent).map_err(failed)?;
    }
    turn_off(tree)?;
    Ok(path)
}

/// Turn managed SimpleX off in the Hermes config, if it is on: the whole
/// file rewritten through a temporary beside it, with the original's
/// owner and mode, then renamed over it. A config that can't be read as
/// YAML is left as it is (`simplex_runtime.py` reads it as off too).
fn turn_off(tree: &Tree) -> Result<(), HostError> {
    let path = format!("{AGENT}/{HERMES_HOME}/{CONFIG}");
    let failed = |error: Errno| HostError::io(path.clone().into(), error.into());
    let Some(agent) = open_in(tree.root(), AGENT)? else {
        return Ok(());
    };
    let Some(home) = open_in(&agent, HERMES_HOME)? else {
        return Ok(());
    };
    let file = match open_regular(&home, &c_name(CONFIG)?) {
        Ok(Some(file)) => file,
        Ok(None) | Err(Errno::NOENT | Errno::LOOP) => return Ok(()),
        Err(error) => return Err(failed(error)),
    };
    let stat = rustix::fs::fstat(&file).map_err(failed)?;
    let mut original = Vec::new();
    (&file)
        .take(CONFIG_MAX_BYTES + 1)
        .read_to_end(&mut original)
        .map_err(|error| HostError::io(path.clone().into(), error))?;
    if original.len() as u64 > CONFIG_MAX_BYTES {
        return Ok(());
    }
    let Ok(mut config) = serde_yaml::from_slice::<serde_yaml::Value>(&original) else {
        return Ok(());
    };
    let enabled = config
        .get_mut("gateway")
        .and_then(|gateway| gateway.get_mut("platforms"))
        .and_then(|platforms| platforms.get_mut("simplex"))
        .and_then(|simplex| simplex.get_mut("enabled"));
    match enabled {
        Some(enabled) if *enabled == serde_yaml::Value::Bool(true) => {
            *enabled = serde_yaml::Value::Bool(false);
        }
        _ => return Ok(()),
    }
    let rewritten = serde_yaml::to_string(&config)
        .map_err(|error| HostError::Write(format!("{path}: {error}")))?;
    let _ = unlinkat(&home, CONFIG_REWRITE, AtFlags::empty());
    let mut temporary = File::from(
        openat(
            &home,
            CONFIG_REWRITE,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(failed)?,
    );
    let mut write = || -> Result<(), Errno> {
        temporary
            .write_all(rewritten.as_bytes())
            .map_err(|error| Errno::from_io_error(&error).unwrap_or(Errno::IO))?;
        fchmod(&temporary, Mode::from_raw_mode(stat.st_mode & 0o777))?;
        fsync(&temporary)?;
        // Its owner last: once it isn't ours, its mode is no longer ours to set.
        fchown(
            &temporary,
            Some(Uid::from_raw(stat.st_uid)),
            Some(Gid::from_raw(stat.st_gid)),
        )?;
        renameat(&home, CONFIG_REWRITE, &home, CONFIG)?;
        fsync(&home)
    };
    if let Err(error) = write() {
        let _ = unlinkat(&home, CONFIG_REWRITE, AtFlags::empty());
        return Err(failed(error));
    }
    Ok(())
}

/// The folder `name` in `parent`, if it is one (never through a link).
fn open_in(parent: impl std::os::fd::AsFd, name: &str) -> Result<Option<OwnedFd>, HostError> {
    match open_directory(parent, &c_name(name)?) {
        Ok(folder) => Ok(Some(folder)),
        Err(Errno::NOENT | Errno::NOTDIR | Errno::LOOP) => Ok(None),
        Err(error) => Err(HostError::io(name.into(), error.into())),
    }
}

/// The sealed folders in `agent/`, by name.
fn sealed_folders(agent: &OwnedFd) -> Result<Vec<String>, HostError> {
    let failed = |error: Errno| HostError::io(AGENT.into(), error.into());
    let mut sealed = Vec::new();
    for entry in Dir::read_from(agent).map_err(failed)? {
        let entry = entry.map_err(failed)?;
        let Ok(name) = entry.file_name().to_str() else {
            continue;
        };
        if name.starts_with(SEALED_PREFIX)
            && stat_entry(agent, entry.file_name())
                .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Directory)
        {
            sealed.push(name.to_owned());
        }
    }
    sealed.sort();
    Ok(sealed)
}

/// Whether anything at all (of any kind) is at `name` in the folder.
fn present_in(folder: &OwnedFd, name: &str) -> Result<bool, HostError> {
    match stat_entry(folder, &c_name(name)?) {
        Ok(_) => Ok(true),
        Err(Errno::NOENT) => Ok(false),
        Err(error) => Err(HostError::io(name.into(), error.into())),
    }
}

fn c_name(name: &str) -> Result<CString, HostError> {
    CString::new(name).map_err(|_| HostError::BadId)
}
