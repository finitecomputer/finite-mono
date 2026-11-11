//! Opening one durable tree, `<work root>/kata/<durable_state_id>`, and
//! reading inside it without following a link.
//!
//! The reader runs with `CAP_DAC_READ_SEARCH` on the runner hosts, so it
//! could read any file a link points at. Every open below is relative to
//! a directory descriptor, with `O_NOFOLLOW`, and checked to be the kind
//! it should be; nothing is ever opened for writing or created.

use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, FileType, FlockOperation, Mode, OFlags, Stat};
use rustix::io::Errno;

use crate::error::HostError;

/// The longest durable state id: the Runner's `sanitize_sandbox_name`
/// truncates to 63 (finite-saas-runner/src/lib.rs:4399-4412).
const ID_MAX: usize = 63;

/// The chat store's single-writer lease, relative to the tree
/// (finite-saas-runner/src/kata.rs:36-40).
pub const WRITER_LEASE: [&str; 2] = ["agent", "client.sqlite3.writer-lease"];

/// A validated durable state id: what the Runner's `sanitize_sandbox_name`
/// can produce, `[A-Za-z0-9_-]`, not starting or ending with `-`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableId(String);

impl DurableId {
    pub fn parse(value: &str) -> Result<Self, HostError> {
        let valid = !value.is_empty()
            && value.len() <= ID_MAX
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            && !value.starts_with('-')
            && !value.ends_with('-');
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(HostError::BadId)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// [`Tree::read_whole`]'s answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Whole {
    Absent,
    Bytes(Vec<u8>),
    Unreadable,
}

/// One open durable tree.
pub struct Tree {
    id: DurableId,
    root: OwnedFd,
    device: u64,
}

impl Tree {
    /// Open `<work_root>/kata/<id>`. Neither `kata` nor `<id>` may be a
    /// link.
    pub fn open(work_root: &Path, id: DurableId) -> Result<Self, HostError> {
        if !work_root.is_absolute() {
            return Err(HostError::Usage(
                "--work-root must be an absolute path".into(),
            ));
        }
        let io = |error: Errno| HostError::io(work_root.to_path_buf(), error.into());
        let work = rustix::fs::open(work_root, directory_flags(), Mode::empty()).map_err(io)?;
        let missing = |error: Errno| match error {
            Errno::NOENT | Errno::NOTDIR | Errno::LOOP => HostError::NoTree,
            other => HostError::io(work_root.join("kata").join(id.as_str()), other.into()),
        };
        let kata = open_directory(&work, c"kata").map_err(missing)?;
        let name = CString::new(id.as_str()).map_err(|_| HostError::BadId)?;
        let root = open_directory(&kata, &name).map_err(missing)?;
        let device = rustix::fs::fstat(&root).map_err(io)?.st_dev;
        Ok(Self { id, root, device })
    }

    pub fn id(&self) -> &DurableId {
        &self.id
    }

    pub fn root(&self) -> BorrowedFd<'_> {
        self.root.as_fd()
    }

    /// The filesystem the tree is on; the walk does not leave it.
    pub fn device(&self) -> u64 {
        self.device
    }

    /// Whether the open `fd` is on the tree's filesystem. Nothing read
    /// beside the walk leaves it either: a folder or file mounted from
    /// another is never read.
    pub fn holds(&self, fd: impl AsFd) -> Result<bool, Errno> {
        Ok(rustix::fs::fstat(fd)?.st_dev == self.device)
    }

    /// Open a regular file with one name at `parts`, on the tree's
    /// filesystem, or `None` if any part is missing or on another.
    pub fn open_file(&self, parts: &[&str]) -> Result<Option<File>, HostError> {
        self.open_path(parts, false)
    }

    /// `any_links` is for a file whose contents are never read (the
    /// writer lease): any link count, on any filesystem.
    fn open_path(&self, parts: &[&str], any_links: bool) -> Result<Option<File>, HostError> {
        let Some((name, directories)) = parts.split_last() else {
            return Ok(None);
        };
        let held = |fd: BorrowedFd<'_>| -> Result<bool, HostError> {
            Ok(any_links || self.holds(fd).map_err(|error| self.error(parts, error))?)
        };
        let mut directory =
            rustix::io::dup(&self.root).map_err(|error| self.error(parts, error))?;
        for part in directories {
            let part = CString::new(*part).map_err(|_| HostError::BadId)?;
            directory = match open_directory(&directory, &part) {
                Ok(next) => next,
                Err(Errno::NOENT | Errno::NOTDIR | Errno::LOOP) => return Ok(None),
                Err(error) => return Err(self.error(parts, error)),
            };
            if !held(directory.as_fd())? {
                return Ok(None);
            }
        }
        let name = CString::new(*name).map_err(|_| HostError::BadId)?;
        let opened = if any_links {
            open_any_regular(&directory, &name).map(|file| file.map(|(file, _)| file))
        } else {
            open_regular(&directory, &name)
        };
        match opened {
            Ok(Some(file)) if !held(file.as_fd())? => Ok(None),
            Ok(file) => Ok(file),
            Err(Errno::NOENT | Errno::LOOP) => Ok(None),
            Err(error) => Err(self.error(parts, error)),
        }
    }

    /// The `fstat` of the regular file at `parts`, or `None` if it is gone
    /// or is no longer a regular file.
    pub fn stat_file(&self, parts: &[&str]) -> Result<Option<Stat>, HostError> {
        let Some(file) = self.open_file(parts)? else {
            return Ok(None);
        };
        rustix::fs::fstat(&file)
            .map(Some)
            .map_err(|error| self.error(parts, error))
    }

    /// What is at `parts`, for a file whose absence means something (the
    /// owner notice's pairing): nothing at all, a plain file with one name
    /// read whole (at most `max` bytes), or something else that must not
    /// be taken for nothing (a link, another kind of file, a second name,
    /// a file over `max`, a file where a folder belongs, a folder or file
    /// on another filesystem).
    pub fn read_whole(&self, parts: &[&str], max: u64) -> Result<Whole, HostError> {
        let Some((name, directories)) = parts.split_last() else {
            return Ok(Whole::Absent);
        };
        let held = |fd: BorrowedFd<'_>| self.holds(fd).map_err(|error| self.error(parts, error));
        let mut directory =
            rustix::io::dup(&self.root).map_err(|error| self.error(parts, error))?;
        for part in directories {
            let part = CString::new(*part).map_err(|_| HostError::BadId)?;
            directory = match open_directory(&directory, &part) {
                Ok(next) => next,
                Err(Errno::NOENT) => return Ok(Whole::Absent),
                Err(Errno::NOTDIR | Errno::LOOP) => return Ok(Whole::Unreadable),
                Err(error) => return Err(self.error(parts, error)),
            };
            if !held(directory.as_fd())? {
                return Ok(Whole::Unreadable);
            }
        }
        let name = CString::new(*name).map_err(|_| HostError::BadId)?;
        let file = match open_regular(&directory, &name) {
            Ok(Some(file)) if !held(file.as_fd())? => return Ok(Whole::Unreadable),
            Ok(Some(file)) => file,
            Ok(None) | Err(Errno::LOOP) => return Ok(Whole::Unreadable),
            Err(Errno::NOENT) => return Ok(Whole::Absent),
            Err(error) => return Err(self.error(parts, error)),
        };
        let mut bytes = Vec::new();
        file.take(max + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| HostError::io(self.display(parts), error))?;
        if bytes.len() as u64 > max {
            return Ok(Whole::Unreadable);
        }
        Ok(Whole::Bytes(bytes))
    }

    /// Read a small regular file at `parts`, refusing one over `max`
    /// bytes.
    pub fn read_small(&self, parts: &[&str], max: u64) -> Result<Option<Vec<u8>>, HostError> {
        let Some(file) = self.open_file(parts)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        file.take(max + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| HostError::io(self.display(parts), error))?;
        if bytes.len() as u64 > max {
            return Ok(None);
        }
        Ok(Some(bytes))
    }

    /// Take the chat store's writer lease and keep it while the returned
    /// file is open: no agent can open its store for writing meanwhile.
    /// A tree without the lease file has no store a writer could hold.
    pub fn take_writer_lease(&self) -> Result<Option<File>, HostError> {
        let Some(lease) = self.open_path(&WRITER_LEASE, true)? else {
            return Ok(None);
        };
        match rustix::fs::flock(&lease, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => Ok(Some(lease)),
            Err(Errno::WOULDBLOCK) => Err(HostError::LeaseHeld),
            Err(error) => Err(self.error(&WRITER_LEASE, error)),
        }
    }

    /// Check that no process holds the writer lease, without keeping it.
    pub fn check_writer_lease(&self) -> Result<(), HostError> {
        drop(self.take_writer_lease()?);
        Ok(())
    }

    fn display(&self, parts: &[&str]) -> PathBuf {
        parts.iter().collect()
    }

    fn error(&self, parts: &[&str], error: Errno) -> HostError {
        HostError::io(self.display(parts), error.into())
    }
}

fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

/// Open the directory `name` in `parent`, never through a link.
pub fn open_directory(parent: impl AsFd, name: &CStr) -> Result<OwnedFd, Errno> {
    rustix::fs::openat(parent, name, directory_flags(), Mode::empty())
}

/// Open the regular file `name` in `parent`, never through a link and
/// never blocking on a pipe; anything else is `None`. A file with more
/// than one name is `None` too: a hard link can be a file outside the
/// tree, and `O_NOFOLLOW` does not see it.
pub fn open_regular(parent: impl AsFd, name: &CStr) -> Result<Option<File>, Errno> {
    let Some((file, stat)) = open_any_regular(parent, name)? else {
        return Ok(None);
    };
    Ok((stat.st_nlink == 1).then_some(file))
}

/// `open_regular`, whatever the file's link count: only for a file whose
/// contents are never read, such as the writer lease.
fn open_any_regular(parent: impl AsFd, name: &CStr) -> Result<Option<(File, Stat)>, Errno> {
    let flags =
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC;
    let fd = rustix::fs::openat(parent, name, flags, Mode::empty())?;
    let stat = rustix::fs::fstat(&fd)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Ok(None);
    }
    Ok(Some((File::from(fd), stat)))
}

/// `lstat` of `name` in `parent`.
pub fn stat_entry(parent: impl AsFd, name: &CStr) -> Result<Stat, Errno> {
    rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_what_the_runner_can_make() {
        for good in ["runtime_123", "a", "state-test", &"x".repeat(63)] {
            assert!(DurableId::parse(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "-a",
            "a-",
            "../a",
            "a/b",
            "a b",
            ".",
            "..",
            &"x".repeat(64),
            "a\0b",
        ] {
            assert!(DurableId::parse(bad).is_err(), "{bad:?}");
        }
    }
}
