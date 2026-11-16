//! The one walk over a durable tree. `sizes` and `stream` both run it, with
//! the one classifier, so `sizes` estimates exactly the rules `stream`
//! stages by.
//!
//! For each entry it asks [`classify`]. Staging (`stream`), it then reads a
//! file the path rules would stage for credentials ([`shapes::snapshot`]):
//! a `history` text file that holds one is staged redacted (as phase 0's
//! export did the SOUL and memories); any other is left.
//! Measuring (`sizes`), it opens no regular file at all: its numbers come
//! from `lstat` and the path rules alone, so they are an estimate of what
//! `stream` stages. It never follows a link, never leaves the tree's
//! filesystem and never opens anything for writing.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::OwnedFd;
use std::path::PathBuf;

use rustix::fs::{Dir, FileType, Stat};
use rustix::io::Errno;

use crate::brains::{self, BRAIN_ROOT, Manifest};
use crate::classify::{self, BUILD_DIRS, CONFIG_COPY_PATH, Class, EntryKind, Verdict, Why};
use crate::config_copy::{CONFIG_MAX_BYTES, public_config};
use crate::error::HostError;
use crate::overrides::Overrides;
use crate::shapes::{self, Snapshot};
use crate::tree::{Tree, open_directory, open_regular, stat_entry};

/// The deepest directory the walk enters; deeper is refused.
pub const DEPTH_MAX: usize = 128;
/// `config.yaml`, whose allow-listed copy the walk makes.
const HERMES_CONFIG: [&str; 3] = ["agent", "hermes-home", "config.yaml"];

/// Each brain working tree in `workspace/finitebrain/`, by its folder's
/// name, with its manifest when it was read ([`crate::brains`]).
pub type Brains = BTreeMap<String, Option<Manifest>>;

/// How the walk treats a tree that changes under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `sizes`: an estimate from metadata, while the agent may be running.
    /// No regular file is opened: an entry that vanishes is skipped, a text
    /// file holding a credential counts in its path's class, and there is
    /// no config copy and no brain's manifest.
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
    /// A file measured, not read: its size from `lstat`.
    Measured(u64),
}

impl Source {
    pub fn bytes(&self) -> u64 {
        match self {
            Source::File { stat, .. } => file_bytes(stat),
            Source::Text { bytes, .. } | Source::Bytes(bytes) => bytes.len() as u64,
            Source::Measured(bytes) => *bytes,
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

/// Walk `tree` by the rules and the operator's `overrides`, and return
/// the brain working trees it found, whose name files the stage writes. A
/// file where one goes is left behind, replaced by the stage's own.
pub fn walk(
    tree: &Tree,
    overrides: &Overrides,
    mode: Mode,
    visit: &mut impl Visit,
) -> Result<Brains, HostError> {
    let mut walk = Walk {
        tree,
        overrides,
        mode,
        brains: Brains::new(),
    };
    let root = rustix::io::dup(tree.root()).map_err(|error| walk.io(&[], error))?;
    walk.directory(&root, &mut Vec::new(), None, visit)?;
    Ok(walk.brains)
}

struct Walk<'a> {
    tree: &'a Tree,
    overrides: &'a Overrides,
    mode: Mode,
    brains: Brains,
}

impl Walk<'_> {
    /// `build_output`: the number of components of the build folder the
    /// directory is in, if any.
    fn directory(
        &mut self,
        directory: &OwnedFd,
        path: &mut Vec<Vec<u8>>,
        build_output: Option<usize>,
        visit: &mut impl Visit,
    ) -> Result<(), HostError> {
        if path.len() >= DEPTH_MAX {
            return Err(HostError::TooDeep(display(path)));
        }
        let names = self.names(directory, path)?;
        // A build folder is build output only beside the project file that
        // builds it (export.py's `beside_project_file`).
        let project_here = names.iter().any(|name| {
            classify::project_file(name.to_bytes())
                && stat_entry(directory, name).is_ok_and(|stat| {
                    FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile
                })
        });
        for name in names {
            path.push(name.to_bytes().to_vec());
            self.entry(directory, &name, path, build_output, project_here, visit)?;
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
        &mut self,
        directory: &OwnedFd,
        name: &CStr,
        path: &mut Vec<Vec<u8>>,
        build_output: Option<usize>,
        project_here: bool,
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
            let build_output = build_output.or_else(|| {
                let in_workspace = path.first().map(Vec::as_slice) == Some(b"workspace");
                let build_folder = BUILD_DIRS
                    .iter()
                    .any(|dir| path[path.len() - 1] == dir.as_bytes());
                (in_workspace && project_here && build_folder).then_some(path.len())
            });
            self.find_brain(&child, path);
            return self.directory(&child, path, build_output, visit);
        }
        // A file with another name may be outside the tree: it is never
        // read, like a symbolic link.
        let kind = if file_type == FileType::RegularFile && stat.st_nlink == 1 {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        let parts: Vec<&[u8]> = path.iter().map(Vec::as_slice).collect();
        if self.replaced_by_name_file(&parts) {
            let replaced = Verdict {
                class: Class::Runtime,
                why: Why::ReplacedByCopy,
                depth: parts.len(),
            };
            return visit.left(&parts, replaced, file_bytes(&stat));
        }
        let verdict =
            self.overrides
                .apply(&parts, kind, classify::classify(&parts, kind, build_output));
        if verdict.class.staged() {
            if self.mode == Mode::Measure {
                return visit.staged("", verdict.class, Source::Measured(file_bytes(&stat)));
            }
            return self.staged(directory, name, &parts, verdict, stat, visit);
        }
        visit.left(&parts, verdict, file_bytes(&stat))?;
        let config = parts[..] == HERMES_CONFIG.map(str::as_bytes)[..];
        if kind == EntryKind::File && config && self.mode == Mode::Stage {
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
        let redact = verdict.class == Class::History;
        let read = shapes::snapshot(&mut file, size, redact)
            .map_err(|error| HostError::io(display(&path()), error))?;
        let why = match &read {
            Snapshot::Binary | Snapshot::Text(_) | Snapshot::Redacted { .. } => None,
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
            Snapshot::Redacted { text, read } => {
                if read != size && self.mode == Mode::Stage {
                    return Err(HostError::TreeChanged(display(&path())));
                }
                Source::Text { bytes: text, stat }
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

    /// A file at the path of a brain's name file, which the stage writes
    /// in its place. Names are walked in order, so `<folder>/` has been
    /// found before `<folder>.json` comes.
    fn replaced_by_name_file(&self, parts: &[&[u8]]) -> bool {
        let [workspace, root, file] = parts else {
            return false;
        };
        [*workspace, *root] == BRAIN_ROOT.map(str::as_bytes)
            && file
                .strip_suffix(b".json")
                .and_then(|folder| std::str::from_utf8(folder).ok())
                .is_some_and(|folder| self.brains.contains_key(folder))
    }

    /// Entering `workspace/finitebrain/<folder>/`: a brain working tree if
    /// it holds `.finitebrain/`, whose manifest is read when staging.
    fn find_brain(&mut self, directory: &OwnedFd, path: &[Vec<u8>]) {
        let [workspace, root, folder] = path else {
            return;
        };
        let is_brain = [workspace.as_slice(), root.as_slice()] == BRAIN_ROOT.map(str::as_bytes)
            && stat_entry(directory, c".finitebrain")
                .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Directory);
        let Some(folder) = std::str::from_utf8(folder).ok().filter(|_| is_brain) else {
            return;
        };
        let manifest = match self.mode {
            Mode::Measure => None,
            Mode::Stage => brains::read_manifest(self.tree, directory),
        };
        self.brains.insert(folder.to_owned(), manifest);
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
