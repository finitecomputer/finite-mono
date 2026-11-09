//! The one walk over a durable tree. `sizes` and `stream` both run it, so
//! what `sizes` counts in a class is exactly what `stream` stages.
//!
//! For each entry it asks [`classify`], then, for a file the path rules
//! would stage, reads the file for credentials ([`shapes::scan`]). It
//! never follows a link, never leaves the tree's filesystem and never
//! opens anything for writing.

use std::collections::BTreeSet;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::{AsFd, OwnedFd};
use std::path::PathBuf;

use rustix::fs::{Dir, FileType, Stat};
use rustix::io::Errno;

use crate::classify::{self, BrainScope, CONFIG_COPY_PATH, Class, EntryKind, Verdict, Why};
use crate::config_copy::{CONFIG_MAX_BYTES, public_config};
use crate::error::HostError;
use crate::shapes::{self, Snapshot};
use crate::tree::{Tree, open_directory, open_regular, stat_entry};

/// The deepest directory the walk enters; deeper is refused.
pub const DEPTH_MAX: usize = 128;
/// The largest `brain-directory.json` read.
const BRAIN_DIRECTORY_MAX_BYTES: u64 = 1024 * 1024;
/// `config.yaml`, whose allow-listed copy the walk makes.
const HERMES_CONFIG: [&str; 3] = ["agent", "hermes-home", "config.yaml"];

/// How the walk treats a tree that changes under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `sizes`: the agent may be running, so an entry that vanishes is
    /// skipped and the numbers are approximate.
    Measure,
    /// `stream`: the agent is stopped and its lease held, so any change is
    /// an error and the stage is abandoned.
    Stage,
}

/// A file going into the stage.
pub enum Source {
    /// A binary file in the tree, positioned at its start, with the
    /// `fstat` it was classified by.
    File { file: File, stat: Box<Stat> },
    /// A text file, as it was when it was checked: the stage holds exactly
    /// these bytes. `stat` is the file's `fstat` when it was read.
    Text { bytes: Vec<u8>, stat: Box<Stat> },
    /// The allow-listed copy of `config.yaml`.
    Bytes(Vec<u8>),
}

impl Source {
    pub fn bytes(&self) -> u64 {
        match self {
            Source::File { stat, .. } => file_bytes(stat),
            Source::Text { bytes, .. } | Source::Bytes(bytes) => bytes.len() as u64,
        }
    }
}

/// What the walk hands each entry to.
pub trait Visit {
    /// An entry that stays behind.
    fn left(&mut self, parts: &[&[u8]], verdict: Verdict, bytes: u64) -> Result<(), HostError>;
    /// An entry that goes into the stage, under `name`.
    fn staged(&mut self, name: &str, class: Class, source: Source) -> Result<(), HostError>;
}

/// Walk `tree`, staging only the brain working trees whose ids are in
/// `owned_brains`.
pub fn walk(
    tree: &Tree,
    owned_brains: &BTreeSet<String>,
    mode: Mode,
    visit: &mut impl Visit,
) -> Result<(), HostError> {
    let walk = Walk {
        tree,
        owned_brains,
        mode,
    };
    let root = rustix::io::dup(tree.root()).map_err(|error| walk.io(&[], error))?;
    walk.directory(&root, &mut Vec::new(), BrainScope::None, visit)
}

struct Walk<'a> {
    tree: &'a Tree,
    owned_brains: &'a BTreeSet<String>,
    mode: Mode,
}

impl Walk<'_> {
    fn directory(
        &self,
        directory: &OwnedFd,
        path: &mut Vec<Vec<u8>>,
        brain: BrainScope,
        visit: &mut impl Visit,
    ) -> Result<(), HostError> {
        if path.len() >= DEPTH_MAX {
            return Err(HostError::TooDeep(display(path)));
        }
        for name in self.names(directory, path)? {
            path.push(name.to_bytes().to_vec());
            self.entry(directory, &name, path, brain, visit)?;
            path.pop();
        }
        Ok(())
    }

    /// A directory's entries, sorted by name, so every walk of an
    /// unchanged tree is the same.
    fn names(&self, directory: &OwnedFd, path: &[Vec<u8>]) -> Result<Vec<CString>, HostError> {
        let mut names = Vec::new();
        let dir = Dir::read_from(directory).map_err(|error| self.io(path, error))?;
        for entry in dir {
            let entry = entry.map_err(|error| self.io(path, error))?;
            let name = entry.file_name();
            if name != c"." && name != c".." {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    fn entry(
        &self,
        directory: &OwnedFd,
        name: &CStr,
        path: &mut Vec<Vec<u8>>,
        brain: BrainScope,
        visit: &mut impl Visit,
    ) -> Result<(), HostError> {
        let stat = match stat_entry(directory, name) {
            Ok(stat) => stat,
            Err(Errno::NOENT) => return self.vanished(path),
            Err(error) => return Err(self.io(path, error)),
        };
        let file_type = FileType::from_raw_mode(stat.st_mode);
        if file_type == FileType::Directory && stat.st_dev == self.tree.device() {
            let child = match open_directory(directory, name) {
                Ok(child) => child,
                Err(Errno::NOENT | Errno::NOTDIR | Errno::LOOP) => return self.vanished(path),
                Err(error) => return Err(self.io(path, error)),
            };
            let brain = self.brain_scope(&child, path, brain);
            return self.directory(&child, path, brain, visit);
        }
        // A file with another name may be outside the tree: it is never
        // read, like a symbolic link.
        let kind = if file_type == FileType::RegularFile && stat.st_nlink == 1 {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        let parts: Vec<&[u8]> = path.iter().map(Vec::as_slice).collect();
        let verdict = classify::classify(&parts, kind, brain);
        if verdict.class.staged() {
            return self.staged(directory, name, &parts, verdict, stat, visit);
        }
        visit.left(&parts, verdict, file_bytes(&stat))?;
        if kind == EntryKind::File && parts[..] == HERMES_CONFIG.map(str::as_bytes)[..] {
            self.config_copy(directory, name, path, visit)?;
        }
        Ok(())
    }

    /// A file the path rules stage: read it for credentials first. A text
    /// file is read once and staged from what was read, so nothing written
    /// to it after the check can reach the stage.
    fn staged(
        &self,
        directory: &OwnedFd,
        name: &CStr,
        parts: &[&[u8]],
        verdict: Verdict,
        stat: Stat,
        visit: &mut impl Visit,
    ) -> Result<(), HostError> {
        let path = || parts.iter().map(|part| part.to_vec()).collect::<Vec<_>>();
        let mut file = match open_regular(directory, name) {
            Ok(Some(file)) => file,
            Ok(None) | Err(Errno::NOENT | Errno::LOOP) => return self.vanished(&path()),
            Err(error) => return Err(self.io(&path(), error)),
        };
        let opened = rustix::fs::fstat(&file).map_err(|error| self.io(&path(), error))?;
        if !same_file(&stat, &opened) && self.mode == Mode::Stage {
            return Err(HostError::TreeChanged(display(&path())));
        }
        let size = file_bytes(&opened);
        let read = shapes::snapshot(&mut file, size)
            .map_err(|error| HostError::io(display(&path()), error))?;
        let why = match &read {
            Snapshot::Binary | Snapshot::Text(_) => None,
            Snapshot::Secret => Some(Why::TokenInText),
            Snapshot::Uncheckable => Some(Why::UncheckableText),
        };
        if let Some(why) = why {
            let secret = Verdict {
                class: Class::Secret,
                why,
                depth: parts.len(),
            };
            return visit.left(parts, secret, size);
        }
        let name = parts
            .iter()
            .map(|part| std::str::from_utf8(part))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| HostError::TreeChanged(display(&path())))?
            .join("/");
        let stat = Box::new(opened);
        let source = match read {
            Snapshot::Text(bytes) => {
                if bytes.len() as u64 != size && self.mode == Mode::Stage {
                    return Err(HostError::TreeChanged(display(&path())));
                }
                Source::Text { bytes, stat }
            }
            _ => {
                file.seek(SeekFrom::Start(0))
                    .map_err(|error| HostError::io(display(&path()), error))?;
                Source::File { file, stat }
            }
        };
        visit.staged(&name, verdict.class, source)
    }

    /// Stage the allow-listed copy of `config.yaml` beside it.
    fn config_copy(
        &self,
        directory: &OwnedFd,
        name: &CStr,
        path: &[Vec<u8>],
        visit: &mut impl Visit,
    ) -> Result<(), HostError> {
        let file = match open_regular(directory, name) {
            Ok(Some(file)) => file,
            Ok(None) | Err(Errno::NOENT | Errno::LOOP) => return self.vanished(path),
            Err(error) => return Err(self.io(path, error)),
        };
        let mut original = Vec::new();
        file.take(CONFIG_MAX_BYTES + 1)
            .read_to_end(&mut original)
            .map_err(|error| HostError::io(display(path), error))?;
        if original.len() as u64 > CONFIG_MAX_BYTES {
            return Ok(());
        }
        match public_config(&original) {
            Some(copy) => visit.staged(
                &CONFIG_COPY_PATH.join("/"),
                Class::Config,
                Source::Bytes(copy),
            ),
            None => Ok(()),
        }
    }

    /// Entering a directory that holds `.finitebrain/`: a brain working
    /// tree, wherever `fbrain open` put it, owned only if the mover named
    /// its id. An unowned tree's whole subtree stays unowned.
    fn brain_scope(&self, directory: &OwnedFd, path: &[Vec<u8>], outer: BrainScope) -> BrainScope {
        if matches!(outer, BrainScope::Brain { owned: false, .. }) {
            return outer;
        }
        let is_brain = stat_entry(directory, c".finitebrain")
            .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Directory);
        if !is_brain {
            return outer;
        }
        BrainScope::Brain {
            depth: path.len(),
            owned: brain_id(directory).is_some_and(|id| self.owned_brains.contains(&id)),
        }
    }

    fn vanished(&self, path: &[Vec<u8>]) -> Result<(), HostError> {
        match self.mode {
            Mode::Measure => Ok(()),
            Mode::Stage => Err(HostError::TreeChanged(display(path))),
        }
    }

    fn io(&self, path: &[Vec<u8>], error: Errno) -> HostError {
        HostError::io(display(path), error.into())
    }
}

/// The brain id in `.finitebrain/brain-directory.json`
/// (finite-brain-core/src/portability.rs:355-386), if it can be read.
fn brain_id(directory: impl AsFd) -> Option<String> {
    let control = open_directory(directory, c".finitebrain").ok()?;
    let file = open_regular(&control, c"brain-directory.json").ok()??;
    let mut bytes = Vec::new();
    file.take(BRAIN_DIRECTORY_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > BRAIN_DIRECTORY_MAX_BYTES {
        return None;
    }
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    manifest
        .get("brain")?
        .get("id")?
        .as_str()
        .map(str::to_owned)
}

/// A regular file's size; nothing else holds bytes the move counts.
fn file_bytes(stat: &Stat) -> u64 {
    if FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile {
        u64::try_from(stat.st_size).unwrap_or(0)
    } else {
        0
    }
}

/// The same inode, size and modification time.
pub fn same_file(a: &Stat, b: &Stat) -> bool {
    a.st_dev == b.st_dev
        && a.st_ino == b.st_ino
        && a.st_size == b.st_size
        && a.st_mtime == b.st_mtime
        && a.st_mtime_nsec == b.st_mtime_nsec
}

fn display(path: &[Vec<u8>]) -> PathBuf {
    path.iter()
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect()
}
