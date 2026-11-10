//! Typed failures, each with its exit status. No message ever holds a
//! file's contents: parse errors of files that may hold a credential are
//! dropped, never wrapped.

use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("usage: {0}")]
    Usage(String),
    #[error("that is not a durable state id")]
    BadId,
    #[error("no durable tree has that id")]
    NoTree,
    #[error("the chat store's writer lease is held: the agent is still running")]
    LeaseHeld,
    #[error("{} changed while it was read: stop the agent and try again", .0.display())]
    TreeChanged(PathBuf),
    #[error("{} is more than {} directories deep", .0.display(), crate::walk::DEPTH_MAX)]
    TooDeep(PathBuf),
    #[error("{}: {source}", .path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("writing the stage: {0}")]
    Write(String),
}

impl HostError {
    pub fn io(path: PathBuf, source: io::Error) -> Self {
        Self::Io { path, source }
    }

    /// The process exit status (sysexits.h): usage 64, bad input 65,
    /// I/O 74, try again later 75.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 64,
            Self::BadId | Self::NoTree | Self::TooDeep(_) => 65,
            Self::Io { .. } | Self::Write(_) => 74,
            Self::LeaseHeld | Self::TreeChanged(_) => 75,
        }
    }
}
