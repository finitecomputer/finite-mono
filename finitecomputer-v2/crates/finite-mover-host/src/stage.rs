//! `sizes` and `stream`: the same walk, estimated from metadata or
//! written.
//!
//! The stage is a zip64 of stored (uncompressed) entries, streamed with
//! data descriptors, so it can be written to a pipe and any file in it
//! read later by one range request. Each staged file keeps its path in
//! the tree (`agent/hermes-home/state.db`, `workspace/...`), plus the
//! allow-listed config copy. The last entry is [`SUMMARY_ENTRY`]: what
//! `stream` staged and left, exactly, in the shape `sizes` estimates.

use std::collections::BTreeSet;
use std::io::{self, Read, Write};

use rustix::fs::Stat;

use crate::classify::{Class, Verdict};
use crate::error::HostError;
use crate::summary::{Summary, Tallies};
use crate::tree::Tree;
use crate::walk::{self, Mode, Source, Visit, same_file};
use crate::zip64::ZipStream;

/// The summary's name inside the stage. No source path can take it: a
/// top-level name other than `agent` and `workspace` is never staged.
pub const SUMMARY_ENTRY: &str = ".finite-stage/summary.json";

/// Estimate every class from metadata, opening no regular file. The agent
/// may be running. The exact bytes are `stream`'s.
pub fn sizes(tree: &Tree, owned_brains: &BTreeSet<String>) -> Result<Summary, HostError> {
    let mut count = Count(Tallies::default());
    walk::walk(tree, owned_brains, Mode::Measure, &mut count)?;
    Ok(count.0.summary(tree.id().as_str()))
}

struct Count(Tallies);

impl Visit for Count {
    fn left(&mut self, parts: &[&[u8]], verdict: Verdict, bytes: u64) -> Result<(), HostError> {
        self.0.left(parts, verdict, bytes);
        Ok(())
    }

    fn staged(&mut self, _name: &str, class: Class, source: Source) -> Result<(), HostError> {
        self.0.staged(class, source.bytes());
        Ok(())
    }
}

/// Write the stage to `out`, holding the chat store's writer lease for
/// the whole walk. Refuses a running agent. Returns the summary it wrote.
pub fn stream(
    tree: &Tree,
    owned_brains: &BTreeSet<String>,
    out: impl Write,
) -> Result<Summary, HostError> {
    let _lease = tree.take_writer_lease()?;
    let mut stage = Stage {
        zip: ZipStream::new(io::BufWriter::with_capacity(1024 * 1024, out)),
        tallies: Tallies::default(),
        copied: Vec::new(),
    };
    walk::walk(tree, owned_brains, Mode::Stage, &mut stage)?;
    let Stage {
        mut zip,
        tallies,
        copied,
    } = stage;
    // A runtime started while the walk ran (a restart from the old
    // dashboard) would leave files copied before it wrote them stale: a
    // session store without its newest log. Every staged file must still be
    // as it was copied.
    for (name, stat) in &copied {
        let parts: Vec<&str> = name.split('/').collect();
        if !tree
            .stat_file(&parts)?
            .is_some_and(|now| same_file(stat, &now))
        {
            return Err(HostError::TreeChanged(name.into()));
        }
    }
    let summary = tallies.summary(tree.id().as_str());
    let json = serde_json::to_vec_pretty(&summary).map_err(write_error)?;
    zip.entry(SUMMARY_ENTRY, 0o644, 0, &mut &json[..])
        .map_err(write_error)?;
    zip.finish().map_err(write_error)?;
    Ok(summary)
}

struct Stage<W: Write> {
    zip: ZipStream<W>,
    tallies: Tallies,
    /// Each staged file and its `fstat` when copied.
    copied: Vec<(String, Stat)>,
}

impl<W: Write> Visit for Stage<W> {
    fn left(&mut self, parts: &[&[u8]], verdict: Verdict, bytes: u64) -> Result<(), HostError> {
        self.tallies.left(parts, verdict, bytes);
        Ok(())
    }

    fn staged(&mut self, name: &str, class: Class, source: Source) -> Result<(), HostError> {
        let bytes = source.bytes();
        match source {
            Source::Measured(_) => {
                return Err(HostError::Write("a measured file can't be staged".into()));
            }
            Source::Bytes(data) => {
                self.zip
                    .entry(name, 0o644, 0, &mut &data[..])
                    .map_err(write_error)?;
            }
            Source::Text { bytes: data, stat } => {
                self.zip
                    .entry(name, stat.st_mode, stat.st_mtime, &mut &data[..])
                    .map_err(write_error)?;
                self.copied.push((name.to_owned(), *stat));
            }
            Source::File { file, stat } => {
                let copied = self
                    .zip
                    .entry(name, stat.st_mode, stat.st_mtime, &mut (&file).take(bytes))
                    .map_err(write_error)?;
                let after = rustix::fs::fstat(&file)
                    .map_err(|error| HostError::io(name.into(), error.into()))?;
                if copied != bytes || !same_file(&stat, &after) {
                    return Err(HostError::TreeChanged(name.into()));
                }
                self.copied.push((name.to_owned(), after));
            }
        }
        self.tallies.staged(class, bytes);
        Ok(())
    }
}

fn write_error(error: impl std::fmt::Display) -> HostError {
    HostError::Write(error.to_string())
}
