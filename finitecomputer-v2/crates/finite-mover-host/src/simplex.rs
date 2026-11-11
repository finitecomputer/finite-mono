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
//! - neither has a `-journal` or `-wal` beside it, so both are settled;
//! - together they are at most [`SIMPLEX_MAX_BYTES`].
//!
//! Nothing else of `agent/simplex/` goes: not its address, its files or
//! its relay settings.
//!
//! ## Writers: the hold, then the seal
//!
//! The agent's lease doesn't hold SimpleX's own writers. What excludes them
//! is Core's hold (FIN-463): the mover streams only once Core's
//! `runtime-stop-exact` has stopped the runtime, and from that stop Core
//! starts it again for nothing but the mover's own restart. So no SimpleX
//! core runs from before `stream` starts until the move is done. The reader
//! can't see Core: the stage id is its word for that hold, since the mover
//! names the stage only after that stop. Without one nothing is carried
//! (`no-stage-id`), whatever the tree looks like.
//!
//! The seal keeps the old core from the databases for good: a core opens
//! them by `agent/simplex`'s path, and SQLite opens a database's `-wal` and
//! `-journal` by its path too. The checks after the rename then hold: the
//! folder renamed is the one checked and copied, both databases are
//! settled, and each is as it was copied. Otherwise the folder goes back
//! where it was, unsealed, and the stage fails as a changed tree.
//!
//! ## The seal: the reader's one write in an agent's tree
//!
//! If the old core ever ran again on its copy, the conversation would fork:
//! messages lost, and the old bot able to talk to the owner. So once the
//! carried entries are staged, still holding the stopped agent's lease,
//! `stream` seals SimpleX where every later start of the old agent sees
//! it ([`seal`]):
//!
//! - `agent/simplex` is renamed
//!   `agent/simplex.moved-<stage id>.owner-<contact>`, never over
//!   anything, never deleted, so it can be undone by hand. Its name keeps
//!   the owner it was carried for, so a retry never takes another from a
//!   pairing made since. With its identity gone, `simplex_runtime.py`'s
//!   `supervise()` fails closed;
//! - managed SimpleX is turned off in `agent/hermes-home/config.yaml`
//!   (`gateway.platforms.simplex.enabled: false`), only if it was on: one
//!   token changed and every other byte as it was, written by a rename
//!   that keeps its owner and mode.
//!
//! Restarting the old agent brings back everything else, never SimpleX.
//! A stage after that carries from the sealed folder again and seals
//! nothing more; an `agent/simplex` made again after the seal stays behind
//! and never takes the sealed conversation's place, nor does a pairing
//! made since. Without a stage id there is nothing to name the seal by,
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
use rustix::thread::{CapabilitySet, capabilities, set_capabilities};
use serde::Serialize;

use crate::config_copy::CONFIG_MAX_BYTES;
use crate::error::HostError;
use crate::pairing::{Approved, approved};
use crate::tree::{Tree, open_directory, open_regular, stat_entry};
use crate::walk::same_file;

pub const CHAT_ENTRY: &str = ".finite-stage/simplex/chat.db";
pub const AGENT_ENTRY: &str = ".finite-stage/simplex/agent.db";
pub const OWNER_ENTRY: &str = ".finite-stage/simplex/owner.json";
/// The most the two databases may hold together. A bot with one contact
/// holds a few megabytes; its files stay behind.
pub const SIMPLEX_MAX_BYTES: u64 = 256 * 1024 * 1024;

const AGENT: &str = "agent";
const FOLDER: &str = "simplex";
/// A sealed folder: `simplex.moved-<stage id>.owner-<contact>`, beside
/// where it was.
const SEALED_PREFIX: &str = "simplex.moved-";
const OWNER_MARK: &str = ".owner-";
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
    /// A pairing file isn't a plain file of ids, or a sealed folder's name
    /// doesn't name its owner.
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
    /// The conversation's folder, open: the one checked and copied.
    folder: OwnedFd,
    /// Its name in `agent/`: `simplex`, or its sealed name already.
    name: String,
    /// The sealed folder's name in `agent/`.
    sealed: String,
    /// Each carried database's name in the folder, and its `fstat` once
    /// copied ([`Seal::copied`]).
    databases: Vec<(&'static str, Stat)>,
}

impl Seal {
    /// Record a database as copied, for the seal's last check.
    pub fn copied(&mut self, database: &Database, stat: Stat) {
        self.databases.push((database.name, stat));
    }
}

/// One database to stage: open, at its start, with the `fstat` it was
/// checked by.
pub struct Database {
    /// Its path in the tree.
    pub source: String,
    /// Its name in the folder.
    pub name: &'static str,
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
    let Some(agent) = open_in(tree, tree.root(), AGENT)? else {
        return Ok(Err(Refused::NoSimplex));
    };
    // The conversation is in `agent/simplex`, or, after a stage, in the one
    // folder it was sealed in.
    // Once sealed, the sealed folder is the conversation: an `agent/simplex`
    // made again after it (by the old dashboard, say) stays behind and never
    // takes its place.
    let mut sealed = sealed_folders(&agent)?;
    let name = match (sealed.pop(), sealed.pop()) {
        (Some(_), Some(_)) => return Ok(Err(Refused::SeveralSealed)),
        (Some(one), None) => one,
        (None, _) if open_in(tree, &agent, FOLDER)?.is_some() => FOLDER.to_owned(),
        (None, _) => return Ok(Err(Refused::NoSimplex)),
    };
    let Some(folder) = open_in(tree, &agent, &name)? else {
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
    if name == FOLDER && stage.is_none() {
        return Ok(Err(Refused::NoStageId));
    }
    // Once sealed, the owner is the one it was sealed with, never the live
    // pairing: a reset clears that, and a new pairing may approve another.
    let contact = if name != FOLDER {
        match sealed_owner(&name) {
            Some(contact) => contact,
            None => return Ok(Err(Refused::PairingUnreadable)),
        }
    } else {
        match approved(tree, "simplex")? {
            Approved::Unreadable => return Ok(Err(Refused::PairingUnreadable)),
            Approved::Ids(ids) => {
                let mut ids = ids.into_iter();
                match (ids.next(), ids.next()) {
                    (None, _) => return Ok(Err(Refused::NoOwner)),
                    (Some(id), None) => id,
                    _ => return Ok(Err(Refused::SeveralContacts)),
                }
            }
        }
    };
    let sealed = match stage {
        Some(stage) if name == FOLDER => format!("{SEALED_PREFIX}{stage}{OWNER_MARK}{contact}"),
        _ => name.clone(),
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
            name,
            entry: entry_name,
            file,
            stat,
        });
    }
    if !settled(&folder)? {
        return Ok(Err(Refused::JournalPresent));
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
            folder,
            name,
            sealed,
            databases: Vec::new(),
        },
    }))
}

/// Whether both databases in `folder` are settled: no `-journal` or
/// `-wal` beside them. A SimpleX writer that committed may leave the
/// database itself as it was, its rows in a WAL the stage doesn't hold.
fn settled(folder: &OwnedFd) -> Result<bool, HostError> {
    for (name, _) in DATABASES {
        for suffix in UNSETTLED {
            if present_in(folder, &format!("{name}{suffix}"))? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Seal the old SimpleX once its conversation is staged: the folder
/// renamed aside (unless it already was), checked a last time, and managed
/// SimpleX turned off. A folder that fails its last checks goes back where
/// it was, unsealed, and the stage fails. Returns the sealed folder's path.
pub fn seal(tree: &Tree, seal: Seal) -> Result<String, HostError> {
    let path = format!("{AGENT}/{}", seal.sealed);
    let failed = |error: Errno| HostError::io(path.clone().into(), error.into());
    let unsealed = seal.name != seal.sealed;
    let rename = |from: &str, to: &str| {
        with_writes(|| {
            renameat_with(&seal.agent, from, &seal.agent, to, RenameFlags::NOREPLACE)
                .and_then(|()| fsync(&seal.agent))
                .map_err(failed)
        })
    };
    if unsealed {
        rename(&seal.name, &seal.sealed)?;
    }
    match last_checks(tree, &seal) {
        Ok(true) => {}
        checked => {
            if unsealed {
                rename(&seal.sealed, &seal.name)?;
            }
            return Err(checked.err().unwrap_or(HostError::TreeChanged(path.into())));
        }
    }
    with_writes(|| turn_off(tree))?;
    Ok(path)
}

/// The carry's last checks, once sealed, so no core can reach the
/// databases (the module's "Writers"): what holds now holds for good. The
/// folder sealed is the one checked and copied; both databases are
/// settled; and each is as it was copied. Settled first, so a WAL
/// checkpointed into its database at any moment shows either there or in
/// the database, changed.
fn last_checks(tree: &Tree, seal: &Seal) -> Result<bool, HostError> {
    let Some(sealed) = open_in(tree, &seal.agent, &seal.sealed)? else {
        return Ok(false);
    };
    if !same_folder(&sealed, &seal.folder) || !settled(&seal.folder)? {
        return Ok(false);
    }
    for (name, copied) in &seal.databases {
        match stat_entry(&seal.folder, &c_name(name)?) {
            Ok(now) if same_file(copied, &now) => {}
            Ok(_) | Err(Errno::NOENT) => return Ok(false),
            Err(error) => return Err(HostError::io(name.into(), error.into())),
        }
    }
    Ok(true)
}

/// Whether two open folders are the same one.
fn same_folder(a: &OwnedFd, b: &OwnedFd) -> bool {
    match (rustix::fs::fstat(a), rustix::fs::fstat(b)) {
        (Ok(a), Ok(b)) => (a.st_dev, a.st_ino) == (b.st_dev, b.st_ino),
        _ => false,
    }
}

/// The owner a sealed folder was sealed with, from its name
/// (`simplex.moved-<stage id>.owner-<contact>`): a positive decimal id.
fn sealed_owner(name: &str) -> Option<i64> {
    let (stage, contact) = name.strip_prefix(SEALED_PREFIX)?.rsplit_once(OWNER_MARK)?;
    let id: i64 = contact.parse().ok()?;
    (!stage.is_empty() && id > 0 && id.to_string() == contact).then_some(id)
}

/// Turn managed SimpleX off in the Hermes config, if it is on, by one
/// token ([`turned_off`]): written through a temporary beside it, with the
/// original's owner and mode, then renamed over it. A config that can't
/// be turned off that way is left exactly as it is; the folder renamed
/// aside already keeps the old core from starting.
fn turn_off(tree: &Tree) -> Result<(), HostError> {
    let path = format!("{AGENT}/{HERMES_HOME}/{CONFIG}");
    let failed = |error: Errno| HostError::io(path.clone().into(), error.into());
    let Some(agent) = open_in(tree, tree.root(), AGENT)? else {
        return Ok(());
    };
    let Some(home) = open_in(tree, &agent, HERMES_HOME)? else {
        return Ok(());
    };
    let file = match open_regular(&home, &c_name(CONFIG)?) {
        Ok(Some(file)) if tree.holds(&file).map_err(failed)? => file,
        Ok(_) | Err(Errno::NOENT | Errno::LOOP) => return Ok(()),
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
    let Some(rewritten) = turned_off(&original) else {
        return Ok(());
    };
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
            .write_all(&rewritten)
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

/// `config.yaml` with managed SimpleX off, every byte but one token as it
/// was, so Hermes' YAML reader (PyYAML) reads every other value as before;
/// or `None` if it isn't on, or can't be turned off that way.
///
/// The token is the plain `true` of `gateway.platforms.simplex.enabled`,
/// on a block-mapping line of its own, the way Hermes writes its config;
/// it becomes `false`. The edit is kept only if the edited document parses
/// to the original with that one value changed, so a look-alike line
/// inside a string is never the one changed.
fn turned_off(original: &[u8]) -> Option<Vec<u8>> {
    const PATH: [&str; 3] = ["gateway", "platforms", "simplex"];
    let text = std::str::from_utf8(original).ok()?;
    let mut parents: Vec<(usize, &str)> = Vec::new();
    let mut token = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let body = line.trim_end_matches(['\n', '\r']);
        let content = body.trim_start_matches(' ');
        if content.is_empty() || content.starts_with('#') {
            continue;
        }
        if content.starts_with('\t') {
            return None;
        }
        let indent = body.len() - content.len();
        while parents.last().is_some_and(|(at, _)| *at >= indent) {
            parents.pop();
        }
        let Some((key, rest)) = content.split_once(':') else {
            continue;
        };
        let plain_key = !key.is_empty()
            && key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte));
        if !plain_key || !(rest.is_empty() || rest.starts_with(' ')) {
            continue;
        }
        let value = rest.split(" #").next().unwrap_or("").trim();
        if value.is_empty() {
            parents.push((indent, key));
            continue;
        }
        let under_simplex = parents.iter().map(|(_, key)| *key).eq(PATH);
        if under_simplex && key == "enabled" && value == "true" {
            if token.is_some() {
                return None;
            }
            let at = start + indent + key.len() + 1 + (rest.len() - rest.trim_start().len());
            token = Some(at..at + "true".len());
        }
    }
    let token = token?;
    let edited = format!("{}false{}", &text[..token.start], &text[token.end..]);
    let mut expected: serde_yaml::Value = serde_yaml::from_str(text).ok()?;
    let enabled = expected
        .get_mut("gateway")?
        .get_mut("platforms")?
        .get_mut("simplex")?
        .get_mut("enabled")?;
    if *enabled != serde_yaml::Value::Bool(true) {
        return None;
    }
    *enabled = serde_yaml::Value::Bool(false);
    let parsed: serde_yaml::Value = serde_yaml::from_str(&edited).ok()?;
    (parsed == expected).then(|| edited.into_bytes())
}

/// The capabilities only the seal uses: held permitted, effective only
/// while it writes ([`seal`]). Every operation lowers them first
/// ([`lower_writes`]), so nothing else the reader does can write.
fn writes() -> CapabilitySet {
    CapabilitySet::DAC_OVERRIDE | CapabilitySet::CHOWN
}

/// Lower the seal's capabilities from the effective set.
pub fn lower_writes() -> Result<(), HostError> {
    let mut sets = capabilities(None).map_err(capability_error)?;
    if sets.effective.intersects(writes()) {
        sets.effective.remove(writes());
        set_capabilities(None, sets).map_err(capability_error)?;
    }
    Ok(())
}

/// Run `write` with the seal's capabilities raised, and lower them after.
fn with_writes<T>(write: impl FnOnce() -> Result<T, HostError>) -> Result<T, HostError> {
    raise_writes()?;
    let written = write();
    lower_writes()?;
    written
}

/// Raise them for the seal, where they are permitted (a test run as the
/// tree's owner needs none).
fn raise_writes() -> Result<(), HostError> {
    let mut sets = capabilities(None).map_err(capability_error)?;
    if sets.permitted.contains(writes()) && !sets.effective.contains(writes()) {
        sets.effective.insert(writes());
        set_capabilities(None, sets).map_err(capability_error)?;
    }
    Ok(())
}

fn capability_error(error: Errno) -> HostError {
    HostError::Write(format!("the seal's capabilities: {error}"))
}

/// The folder `name` in `parent`, if it is one (never through a link) on
/// the tree's filesystem.
fn open_in(
    tree: &Tree,
    parent: impl std::os::fd::AsFd,
    name: &str,
) -> Result<Option<OwnedFd>, HostError> {
    let failed = |error: Errno| HostError::io(name.into(), error.into());
    match open_directory(parent, &c_name(name)?) {
        Ok(folder) if tree.holds(&folder).map_err(failed)? => Ok(Some(folder)),
        Ok(_) | Err(Errno::NOENT | Errno::NOTDIR | Errno::LOOP) => Ok(None),
        Err(error) => Err(failed(error)),
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
