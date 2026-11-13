//! Which brains each old agent's person owns (FIN-499), from finite-brain,
//! the one source of brain ownership on lat-2.
//!
//! **What "owns" means.** Phase 0's export (export.py's `build_raw`) took
//! the brains `fbrain brain list` gave the agent's own identity the role
//! `owner`: in finite-brain's store, `brains.owner_user_id` is the agent's
//! npub (`list_visible_brains`). Only a Personal Brain has an owner;
//! an Organization Brain, which teammates write to, has none, so it never
//! comes, nor does a Personal Brain the agent only serves as its Personal
//! Agent. The agent's npub is Core's pin of the key its health reports are
//! signed with (`agent_runtimes.health_reporting_npub`), the same
//! `/data/agent/identity` key `fbrain` signs with.
//!
//! **How it is read.** finite-brain's SQLite belongs to its service's
//! dynamic user, under `/var/lib/private`. The mover never opens it: it
//! runs `finite-mover-brains` through a fixed command (lat-2's module runs
//! it, by sudo, as that same dynamic user, so no file of the database is
//! ever made by another), which opens the database read-only and prints
//! every owned brain: its id, name and owner. The mover keeps those of
//! the agents it acts for.
//!
//! **Where a brain is.** `fbrain open <id>` puts its working tree at
//! `workspace/finitebrain/<id>/` (`FBRAIN_WORKING_TREE_ROOT`); the host
//! reader stages an owned one there, without `.finitebrain/`, and writes
//! `workspace/finitebrain/<id>.json` with its name.

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::child::run_within;
use crate::contract::{ROSTER_LIST_MAX, short};
use crate::fleet::FleetError;

/// The most owned brains finite-brain may list; more is refused whole.
pub const OWNED_BRAINS_MAX: usize = 100_000;
/// The longest brain name taken, in bytes (finite-brain's are at most 128).
pub const BRAIN_NAME_MAX: usize = 256;
/// The longest brain id (finite-brain's `BrainId`).
pub const BRAIN_ID_MAX: usize = 128;
/// How long the reader may take.
pub const READER_DEADLINE: Duration = Duration::from_secs(120);

/// A brain an agent's person owns, as the host reader is told of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedBrain {
    pub id: String,
    pub name: String,
}

/// One owned brain as `finite-mover-brains` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedBrainRow {
    pub id: String,
    pub name: String,
    /// The owner's npub.
    pub owner: String,
}

/// `finite-mover-brains`' answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReaderOutput {
    pub brains: Vec<OwnedBrainRow>,
}

#[derive(Debug, thiserror::Error)]
pub enum BrainsError {
    #[error("{0}: is owned by uid {1}, not this reader's {2}; it is never opened by another user")]
    NotOwner(String, u32, u32),
    #[error("finite-brain's database: {0}")]
    Database(String),
    #[error("finite-brain lists more than {OWNED_BRAINS_MAX} owned brains")]
    TooMany,
    #[error("finite-brain lists a brain this mover can't carry: {0}")]
    Unusable(String),
}

/// Every owned brain in finite-brain's database at `path`, read by the
/// database's own user, opened read-only.
pub fn read_database(path: &Path) -> Result<ReaderOutput, BrainsError> {
    let database = |error: rusqlite::Error| BrainsError::Database(error.to_string());
    let owner = std::fs::metadata(path)
        .map_err(|error| BrainsError::Database(format!("{}: {error}", path.display())))?
        .uid();
    let me = rustix::process::getuid().as_raw();
    // A reader as another user would make the database's `-shm` as itself,
    // which finite-brain could then not open (the 2026-08-11 chat outage,
    // finite-litestream.nix).
    if owner != me {
        return Err(BrainsError::NotOwner(path.display().to_string(), owner, me));
    }
    let connection = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(database)?;
    connection
        .busy_timeout(Duration::from_secs(10))
        .map_err(database)?;
    connection
        .pragma_update(None, "query_only", true)
        .map_err(database)?;
    let mut statement = connection
        .prepare(
            "SELECT id, name, owner_user_id FROM brains
             WHERE owner_user_id IS NOT NULL ORDER BY id LIMIT ?1",
        )
        .map_err(database)?;
    let rows = statement
        .query_map([OWNED_BRAINS_MAX as i64 + 1], |row| {
            Ok(OwnedBrainRow {
                id: row.get(0)?,
                name: row.get(1)?,
                owner: row.get(2)?,
            })
        })
        .map_err(database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database)?;
    if rows.len() > OWNED_BRAINS_MAX {
        return Err(BrainsError::TooMany);
    }
    Ok(ReaderOutput { brains: rows })
}

/// Whether `id` is a brain id the host reader takes.
fn brain_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= BRAIN_ID_MAX
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// Each owner's brains, by npub.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrainOwners(BTreeMap<String, Vec<OwnedBrain>>);

impl BrainOwners {
    /// Owners from the reader's answer. A brain the host reader couldn't
    /// be told of refuses the whole answer: finite-brain makes none.
    pub fn from_output(output: ReaderOutput) -> Result<Self, BrainsError> {
        if output.brains.len() > OWNED_BRAINS_MAX {
            return Err(BrainsError::TooMany);
        }
        let mut owners: BTreeMap<String, Vec<OwnedBrain>> = BTreeMap::new();
        for row in output.brains {
            let name_ok = !row.name.is_empty()
                && row.name.len() <= BRAIN_NAME_MAX
                && !row.name.chars().any(char::is_control);
            if !brain_id(&row.id) || !name_ok {
                return Err(BrainsError::Unusable(short(&row.id)));
            }
            owners.entry(row.owner).or_default().push(OwnedBrain {
                id: row.id,
                name: row.name,
            });
        }
        Ok(Self(owners))
    }

    /// The brains `npub` owns; none for an agent whose key Core hasn't
    /// pinned.
    pub fn of(&self, npub: Option<&str>) -> Vec<OwnedBrain> {
        let Some(npub) = npub else {
            return Vec::new();
        };
        let mut brains = self.0.get(npub).cloned().unwrap_or_default();
        brains.truncate(ROSTER_LIST_MAX);
        brains
    }
}

/// The command that prints finite-brain's owned brains: on lat-2, sudo of
/// the module's wrapper, which runs `finite-mover-brains` as finite-brain's
/// own user.
#[derive(Debug, Clone)]
pub struct BrainsReader {
    pub program: Vec<String>,
    pub deadline: Duration,
}

impl BrainsReader {
    pub fn new(program: &[String]) -> Self {
        Self {
            program: program.to_vec(),
            deadline: READER_DEADLINE,
        }
    }

    /// Every owned brain, now. Any failure is the fleet's: a stage never
    /// goes without the brains it should carry.
    pub fn read(&self) -> Result<BrainOwners, FleetError> {
        let unreadable = |why: String| FleetError::Unreadable(format!("finite-brain: {why}"));
        let (program, args) = self
            .program
            .split_first()
            .ok_or_else(|| unreadable("no reader is configured".into()))?;
        let mut command = Command::new(program);
        command.args(args);
        let (ended, stdout, stderr) = run_within(&mut command, self.deadline)
            .map_err(|error| unreadable(error.to_string()))?;
        if ended.fired {
            return Err(unreadable(format!(
                "the reader had not ended after {} s",
                self.deadline.as_secs()
            )));
        }
        if !ended.status.success() {
            let stderr = String::from_utf8_lossy(&stderr);
            let last = stderr.lines().rev().find(|line| !line.trim().is_empty());
            return Err(unreadable(short(&format!(
                "{}: {}",
                ended.status,
                last.unwrap_or("").trim()
            ))));
        }
        let output: ReaderOutput = serde_json::from_slice(&stdout)
            .map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
        BrainOwners::from_output(output).map_err(|error| unreadable(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, name: &str, owner: &str) -> OwnedBrainRow {
        OwnedBrainRow {
            id: id.into(),
            name: name.into(),
            owner: owner.into(),
        }
    }

    #[test]
    fn each_agent_gets_only_the_brains_its_own_key_owns() {
        let owners = BrainOwners::from_output(ReaderOutput {
            brains: vec![
                row("brain-a", "A's notes", "npub1a"),
                row("brain-b", "B's notes", "npub1b"),
            ],
        })
        .unwrap();
        assert_eq!(
            owners.of(Some("npub1a")),
            [OwnedBrain {
                id: "brain-a".into(),
                name: "A's notes".into()
            }]
        );
        assert!(owners.of(Some("npub1c")).is_empty());
        assert!(owners.of(None).is_empty(), "no pinned key, no brains");
    }

    #[test]
    fn a_brain_the_host_reader_cant_carry_refuses_the_answer() {
        for bad in [
            row("a/b", "n", "npub1a"),
            row("a:b", "n", "npub1a"),
            row("", "n", "npub1a"),
            row("a", "", "npub1a"),
            row("a", "line\nbreak", "npub1a"),
            row("a", &"x".repeat(BRAIN_NAME_MAX + 1), "npub1a"),
        ] {
            assert!(
                BrainOwners::from_output(ReaderOutput {
                    brains: vec![bad.clone()]
                })
                .is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_database_another_user_owns_is_never_opened() {
        // /etc/passwd is root's; these tests never run as root.
        if rustix::process::getuid().is_root() {
            return;
        }
        assert!(matches!(
            read_database(Path::new("/etc/passwd")),
            Err(BrainsError::NotOwner(..))
        ));
    }
}
