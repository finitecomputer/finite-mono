//! The stage read as it streams past, entry by entry. The host's writer
//! puts each entry's size in its local header, so the uploader learns each
//! name before that entry's bytes and refuses a secret path *before* any of
//! the file's bytes leave the mover. It also checks every entry's CRC and
//! data descriptor, and hands the entries over to be matched against the
//! central directory at the end ([`crate::zip_tail`]).
//!
//! It accepts exactly `finite-mover-host`'s writer: stored entries, a local
//! header with the writer's zip64 field holding the size, the data, and a
//! zip64 data descriptor, one after another from byte 0 up to the central
//! directory.

use std::collections::BTreeSet;

use finite_mover_host::classify::{BrainScope, Class, EntryKind, classify};
use finite_mover_host::stage::SUMMARY_ENTRY;

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const DESCRIPTOR: u32 = 0x0807_4b50;
const ZIP64_EXTRA: u16 = 0x0001;
const FLAGS: u16 = 0x0808;
const LOCAL_BYTES: usize = 30;
const LOCAL_EXTRA_BYTES: usize = 20;
const DESCRIPTOR_BYTES: usize = 24;

/// One entry as it streamed past.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scanned {
    pub name: String,
    /// Where its local header starts.
    pub offset: u64,
    pub size: u64,
    pub crc: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScanError {
    #[error("the stage was refused whole: {0} is a secret path")]
    Secret(String),
    #[error("the stage was refused whole: {0} is not a safe name")]
    Unsafe(String),
    #[error("the stage is malformed at byte {offset}: {why}")]
    Malformed { offset: u64, why: &'static str },
    #[error("the stage ended before its central directory")]
    Truncated,
}

enum State {
    /// Waiting for the next local header, or the central directory.
    Header,
    Data {
        remaining: u64,
        crc: crc32fast::Hasher,
    },
    Descriptor {
        crc: u32,
    },
    /// The central directory has begun; the rest is the tail's to check.
    Directory,
}

pub struct Scanner {
    state: State,
    /// Bytes of a header or descriptor not yet whole.
    pending: Vec<u8>,
    /// Where `pending` starts in the stream.
    offset: u64,
    entries: Vec<Scanned>,
    names: BTreeSet<String>,
    directory_offset: Option<u64>,
}

impl Default for Scanner {
    fn default() -> Self {
        Self {
            state: State::Header,
            pending: Vec::new(),
            offset: 0,
            entries: Vec::new(),
            names: BTreeSet::new(),
            directory_offset: None,
        }
    }
}

impl Scanner {
    /// Read the next bytes of the stream. An error refuses the whole stage.
    pub fn feed(&mut self, mut bytes: &[u8]) -> Result<(), ScanError> {
        while !bytes.is_empty() {
            match &mut self.state {
                State::Directory => return Ok(()),
                State::Data { remaining, crc } => {
                    let take =
                        usize::try_from(*remaining).map_or(bytes.len(), |r| r.min(bytes.len()));
                    crc.update(&bytes[..take]);
                    *remaining -= take as u64;
                    self.offset += take as u64;
                    bytes = &bytes[take..];
                    if *remaining == 0 {
                        let State::Data { crc, .. } =
                            std::mem::replace(&mut self.state, State::Header)
                        else {
                            unreachable!()
                        };
                        self.state = State::Descriptor {
                            crc: crc.finalize(),
                        };
                    }
                }
                State::Header | State::Descriptor { .. } => {
                    let need = self.need();
                    let take = need.saturating_sub(self.pending.len()).min(bytes.len());
                    self.pending.extend_from_slice(&bytes[..take]);
                    bytes = &bytes[take..];
                    if self.pending.len() == self.need() {
                        self.step()?;
                    }
                }
            }
        }
        Ok(())
    }

    /// How many pending bytes the current state needs to decide.
    fn need(&self) -> usize {
        match self.state {
            State::Descriptor { .. } => DESCRIPTOR_BYTES,
            _ if self.pending.len() < LOCAL_BYTES => LOCAL_BYTES,
            _ if read_u32(&self.pending, 0) != Some(LOCAL) => LOCAL_BYTES,
            _ => {
                let name = usize::from(read_u16(&self.pending, 26).unwrap_or_default());
                let extra = usize::from(read_u16(&self.pending, 28).unwrap_or_default());
                LOCAL_BYTES + name + extra
            }
        }
    }

    fn malformed(&self, why: &'static str) -> ScanError {
        ScanError::Malformed {
            offset: self.offset,
            why,
        }
    }

    /// Act on a whole header or descriptor.
    fn step(&mut self) -> Result<(), ScanError> {
        let pending = std::mem::take(&mut self.pending);
        if let State::Descriptor { crc } = self.state {
            let entry = self
                .entries
                .last()
                .ok_or_else(|| self.malformed("a descriptor without an entry"))?;
            if read_u32(&pending, 0) != Some(DESCRIPTOR)
                || read_u32(&pending, 4) != Some(crc)
                || read_u64(&pending, 8) != Some(entry.size)
                || read_u64(&pending, 16) != Some(entry.size)
            {
                return Err(self.malformed("a data descriptor does not match its entry"));
            }
            if let Some(last) = self.entries.last_mut() {
                last.crc = crc;
            }
            self.offset += DESCRIPTOR_BYTES as u64;
            self.state = State::Header;
            return Ok(());
        }
        match read_u32(&pending, 0) {
            Some(CENTRAL) => {
                self.directory_offset = Some(self.offset);
                self.state = State::Directory;
                return Ok(());
            }
            Some(LOCAL) => {}
            _ => return Err(self.malformed("no local header where one belongs")),
        }
        let name_length = usize::from(read_u16(&pending, 26).unwrap_or_default());
        let extra = &pending[LOCAL_BYTES + name_length..];
        let writers = read_u16(&pending, 6) == Some(FLAGS)
            && read_u16(&pending, 8) == Some(0)
            && read_u32(&pending, 14) == Some(0)
            && read_u32(&pending, 18) == Some(u32::MAX)
            && read_u32(&pending, 22) == Some(u32::MAX)
            && extra.len() == LOCAL_EXTRA_BYTES
            && read_u16(extra, 0) == Some(ZIP64_EXTRA)
            && read_u16(extra, 2) == Some(16)
            && read_u64(extra, 4) == read_u64(extra, 12);
        if !writers {
            return Err(self.malformed("a local header is not the writer's"));
        }
        let size = read_u64(extra, 4).unwrap_or_default();
        let name = std::str::from_utf8(&pending[LOCAL_BYTES..LOCAL_BYTES + name_length])
            .map_err(|_| self.malformed("a name is not UTF-8"))?
            .to_string();
        check_name(&name)?;
        if !self.names.insert(name.clone()) {
            // Readers disagree on which of two same-named entries wins.
            return Err(ScanError::Unsafe(name));
        }
        self.entries.push(Scanned {
            name,
            offset: self.offset,
            size,
            crc: 0,
        });
        self.offset += pending.len() as u64;
        self.state = if size == 0 {
            State::Descriptor {
                crc: crc32fast::Hasher::new().finalize(),
            }
        } else {
            State::Data {
                remaining: size,
                crc: crc32fast::Hasher::new(),
            }
        };
        Ok(())
    }

    /// The entries, once the stream has reached its central directory.
    pub fn finish(self) -> Result<(Vec<Scanned>, u64), ScanError> {
        match (self.state, self.directory_offset) {
            (State::Directory, Some(offset)) => Ok((self.entries, offset)),
            _ => Err(ScanError::Truncated),
        }
    }
}

/// The second check, on a name: the host's own classifier, and a name a
/// reader can't misplace.
fn check_name(name: &str) -> Result<(), ScanError> {
    if name == SUMMARY_ENTRY {
        return Ok(());
    }
    let parts: Vec<&str> = name.split('/').collect();
    let unsafe_name = name.starts_with('/')
        || parts.iter().any(|part| {
            part.is_empty() || *part == "." || *part == ".." || part.contains(['\\', '\0'])
        });
    if unsafe_name {
        return Err(ScanError::Unsafe(name.to_string()));
    }
    let bytes: Vec<&[u8]> = parts.iter().map(|part| part.as_bytes()).collect();
    if classify(&bytes, EntryKind::File, BrainScope::None).class == Class::Secret {
        return Err(ScanError::Secret(name.to_string()));
    }
    Ok(())
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use finite_mover_host::zip64::ZipStream;

    use super::*;

    fn stage(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut stream = ZipStream::new(Vec::new());
        for (name, body) in entries {
            stream
                .entry(name, 0o644, 0, body.len() as u64, &mut &body[..])
                .unwrap();
        }
        stream.finish().unwrap()
    }

    fn scan(bytes: &[u8], chunk: usize) -> Result<(Vec<Scanned>, u64), ScanError> {
        let mut scanner = Scanner::default();
        for piece in bytes.chunks(chunk) {
            scanner.feed(piece)?;
        }
        scanner.finish()
    }

    #[test]
    fn every_entry_is_read_in_any_chunking() {
        let bytes = stage(&[
            ("workspace/a.md", b"alpha"),
            ("workspace/empty", b""),
            (SUMMARY_ENTRY, b"{}"),
        ]);
        for chunk in [1, 7, 64, bytes.len()] {
            let (entries, directory) = scan(&bytes, chunk).unwrap();
            let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
            assert_eq!(
                names,
                ["workspace/a.md", "workspace/empty", SUMMARY_ENTRY],
                "{chunk}"
            );
            assert_eq!(entries[0].crc, crc32fast::hash(b"alpha"));
            assert_eq!(
                &bytes[directory as usize..directory as usize + 4],
                &CENTRAL.to_le_bytes()
            );
        }
    }

    #[test]
    fn a_secret_name_is_refused_before_its_bytes() {
        let bytes = stage(&[
            ("workspace/a.md", b"alpha"),
            ("workspace/.env", b"TOKEN=SECRET-BYTES"),
        ]);
        let mut scanner = Scanner::default();
        let data = bytes
            .windows(12)
            .position(|w| w == b"SECRET-BYTES")
            .unwrap();
        // Everything up to the secret's data is enough to refuse it.
        assert_eq!(
            scanner.feed(&bytes[..data]),
            Err(ScanError::Secret("workspace/.env".into()))
        );
    }

    #[test]
    fn a_changed_byte_a_duplicate_or_a_cut_is_refused() {
        let good = stage(&[("workspace/a.md", b"alpha"), (SUMMARY_ENTRY, b"{}")]);
        let mut flipped = good.clone();
        let data = good.windows(5).position(|w| w == b"alpha").unwrap();
        flipped[data] ^= 1;
        assert!(matches!(
            scan(&flipped, 16),
            Err(ScanError::Malformed { .. })
        ));
        let twice = stage(&[("workspace/a.md", b"1"), ("workspace/a.md", b"2")]);
        assert_eq!(
            scan(&twice, 16),
            Err(ScanError::Unsafe("workspace/a.md".into()))
        );
        // Cut before the directory; a cut inside it is the tail's to find.
        assert_eq!(scan(&good[..40], 16), Err(ScanError::Truncated));
        let unsafe_name = stage(&[("workspace/../x", b"1")]);
        assert_eq!(
            scan(&unsafe_name, 16),
            Err(ScanError::Unsafe("workspace/../x".into()))
        );
    }

    /// The scanner as the uploader feeds it, over a stage with a 4.5 GiB
    /// entry (sizes past 32 bits) and the central directory read from the
    /// kept tail. Run by name with `--ignored`, in `--release`.
    #[test]
    #[ignore = "streams a 4.5 GiB stage"]
    fn a_stage_over_4_gib_streams_past() {
        use std::io::{self, Read, Write};

        use crate::zip_tail::{Tail, entries};

        struct Feed {
            scanner: Scanner,
            tail: Tail,
        }
        impl Write for Feed {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.scanner
                    .feed(bytes)
                    .map_err(|error| io::Error::other(error.to_string()))?;
                self.tail.push(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        const BIG: u64 = 4 * 1024 * 1024 * 1024 + 512 * 1024 * 1024;
        let feed = Feed {
            scanner: Scanner::default(),
            tail: Tail::new(1 << 20),
        };
        let mut stream = ZipStream::new(io::BufWriter::with_capacity(1 << 20, feed));
        let mut big = io::repeat(7).take(BIG);
        stream
            .entry("workspace/video.mov", 0o644, 0, BIG, &mut big)
            .unwrap();
        stream
            .entry(SUMMARY_ENTRY, 0o644, 0, 2, &mut &b"{}"[..])
            .unwrap();
        let feed = stream.finish().unwrap().into_inner().ok().unwrap();
        let (scanned, _) = feed.scanner.finish().unwrap();
        assert_eq!(scanned[0].size, BIG);
        let listed = entries(&feed.tail).unwrap();
        assert_eq!(listed[0].size, BIG);
        assert_eq!(listed[0].crc, scanned[0].crc);
        assert_eq!(listed[1].offset, scanned[1].offset);
    }
}
