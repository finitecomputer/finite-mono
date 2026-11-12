//! `sizes` and `stream`: the same walk, estimated from metadata or
//! written.
//!
//! The stage is a zip64 of stored (uncompressed) entries, streamed with
//! data descriptors, so it can be written to a pipe and any file in it
//! read later by one range request. Each staged file keeps its path in
//! the tree (`agent/hermes-home/state.db`, `workspace/...`), plus the
//! allow-listed config copy, each named owned brain's name file
//! (`workspace/finitebrain/<id>.json`, `{"name": …}`) and, when it can be
//! carried safely, the SimpleX conversation ([`crate::simplex`]). The last
//! entry is
//! [`SUMMARY_ENTRY`]: what `stream` staged and left, exactly, in the shape
//! `sizes` estimates.

use std::io::{self, Write};

use rustix::fs::Stat;

use crate::classify::{self, Class, EntryKind, Verdict};
use crate::error::HostError;
use crate::simplex::{self, Carried, Carry, Seal};
use crate::summary::{Summary, Tallies};
use crate::tree::Tree;
use crate::walk::{self, Mode, OwnedBrains, Source, Visit, brain_name_path, same_file};
use crate::zip64::ZipStream;

/// The summary's name inside the stage. No source path can take it: a
/// top-level name other than `agent` and `workspace` is never staged.
pub const SUMMARY_ENTRY: &str = ".finite-stage/summary.json";

/// Estimate every class from metadata, opening no regular file. The agent
/// may be running. The exact bytes are `stream`'s.
///
/// Name files aren't counted: their few bytes are written by `stream`.
pub fn sizes(tree: &Tree, owned_brains: &OwnedBrains) -> Result<Summary, HostError> {
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
/// the whole walk. Refuses a running agent. `stage` names the stage, for
/// the SimpleX seal ([`simplex`]). Returns the summary it wrote.
pub fn stream(
    tree: &Tree,
    owned_brains: &OwnedBrains,
    stage: Option<&str>,
    out: impl Write,
) -> Result<Summary, HostError> {
    let _lease = tree.take_writer_lease()?;
    let stage_id = stage;
    let mut stage = Stage {
        zip: ZipStream::new(io::BufWriter::with_capacity(1024 * 1024, out)),
        tallies: Tallies::default(),
        copied: Vec::new(),
    };
    walk::walk(tree, owned_brains, Mode::Stage, &mut stage)?;
    for (id, name) in owned_brains {
        if let Some(name) = name {
            stage.name_file(id, name)?;
        }
    }
    let carried = match simplex::decide(tree, stage_id)? {
        Ok(carried) => Ok(stage.carry(carried)?),
        Err(why) => Err(why),
    };
    let Stage {
        mut zip,
        tallies,
        copied,
    } = stage;
    // A runtime started while the walk ran (a restart from the old
    // dashboard) would leave files copied before it wrote them stale: a
    // session store without its newest log. Every staged file must still be
    // as it was copied. This comes last.
    for (name, stat) in &copied {
        let parts: Vec<&str> = name.split('/').collect();
        if !tree
            .stat_file(&parts)?
            .is_some_and(|now| same_file(stat, &now))
        {
            return Err(HostError::TreeChanged(name.into()));
        }
    }
    // Staged and checked, the agent still stopped and its lease held: seal
    // the old SimpleX where every later start sees it. The carried
    // databases' own last checks come after the seal, which keeps every
    // SimpleX writer from them ([`simplex`]'s "Writers").
    let carry = match carried {
        Ok(seal) => Carry::carried(simplex::seal(tree, seal)?),
        Err(why) => Carry::refused(why),
    };
    let mut summary = tallies.summary(tree.id().as_str());
    summary.simplex = Some(carry);
    let json = serde_json::to_vec_pretty(&summary).map_err(write_error)?;
    zip.entry(SUMMARY_ENTRY, 0o644, 0, json.len() as u64, &mut &json[..])
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

impl<W: Write> Stage<W> {
    /// Stage brain `id`'s name file, as phase 0's export wrote it. Its
    /// path must be one the uploader's second check takes (an id such as
    /// `identity` would make a credential's name), and its text no
    /// credential's shape; otherwise there is none, and the importer
    /// names the brain from its pages.
    fn name_file(&mut self, id: &str, name: &str) -> Result<(), HostError> {
        let path = brain_name_path(id);
        let parts: Vec<&[u8]> = path.split('/').map(str::as_bytes).collect();
        let body = serde_json::to_vec(&serde_json::json!({ "name": name })).map_err(write_error)?;
        let verdict = classify::classify(&parts, EntryKind::File, classify::BrainScope::None);
        if !verdict.class.staged() || crate::shapes::holds_secret(&body) {
            return Ok(());
        }
        self.staged(&path, Class::Work, Source::Bytes(body))
    }

    /// Stage the SimpleX conversation. Its databases are checked again by
    /// the seal, once sealed. Returns the seal to make.
    fn carry(&mut self, carried: Carried) -> Result<Seal, HostError> {
        let owner = carried.owner();
        let mut seal = carried.seal;
        for database in carried.databases {
            let bytes = u64::try_from(database.stat.st_size).unwrap_or(0);
            match self.zip.entry(
                database.entry,
                0o600,
                database.stat.st_mtime,
                bytes,
                &mut &database.file,
            ) {
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                    return Err(HostError::TreeChanged(database.source.into()));
                }
                other => other.map_err(write_error)?,
            }
            let after = rustix::fs::fstat(&database.file)
                .map_err(|error| HostError::io(database.source.clone().into(), error.into()))?;
            if !same_file(&database.stat, &after) {
                return Err(HostError::TreeChanged(database.source.into()));
            }
            seal.copied(&database, after);
            self.tallies.staged(Class::Simplex, bytes);
        }
        self.zip
            .entry(
                simplex::OWNER_ENTRY,
                0o600,
                0,
                owner.len() as u64,
                &mut &owner[..],
            )
            .map_err(write_error)?;
        self.tallies.staged(Class::Simplex, owner.len() as u64);
        Ok(seal)
    }
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
                    .entry(name, 0o644, 0, bytes, &mut &data[..])
                    .map_err(write_error)?;
            }
            Source::Text { bytes: data, stat } => {
                self.zip
                    .entry(name, stat.st_mode, stat.st_mtime, bytes, &mut &data[..])
                    .map_err(write_error)?;
                self.copied.push((name.to_owned(), *stat));
            }
            Source::File { file, stat } => {
                match self
                    .zip
                    .entry(name, stat.st_mode, stat.st_mtime, bytes, &mut &file)
                {
                    Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                        return Err(HostError::TreeChanged(name.into()));
                    }
                    other => other.map_err(write_error)?,
                }
                let after = rustix::fs::fstat(&file)
                    .map_err(|error| HostError::io(name.into(), error.into()))?;
                if !same_file(&stat, &after) {
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
