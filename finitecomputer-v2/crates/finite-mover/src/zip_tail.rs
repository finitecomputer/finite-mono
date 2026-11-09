//! The names in a streamed stage, read from its central directory, which
//! sits at the end: the uploader keeps the stream's last bytes and reads
//! the directory there once the stream ends, before the upload completes.
//!
//! It reads what `finite-mover-host`'s writer produces, and any zip that
//! keeps its directory, zip64 records included, within the kept tail.

const EOCD: u32 = 0x0605_4b50;
const ZIP64_EOCD: u32 = 0x0606_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;
const ZIP64_EXTRA: u16 = 0x0001;

/// The last bytes of a stream, up to a bound.
pub struct Tail {
    max: usize,
    kept: Vec<u8>,
    total: u64,
}

impl Tail {
    pub fn new(max: usize) -> Self {
        Self {
            max,
            kept: Vec::new(),
            total: 0,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len() as u64;
        self.kept.extend_from_slice(bytes);
        if self.kept.len() > 2 * self.max {
            let excess = self.kept.len() - self.max;
            self.kept.drain(..excess);
        }
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    /// The bytes at absolute `offset..offset + length`, if still kept.
    fn at(&self, offset: u64, length: u64) -> Option<&[u8]> {
        let start = self.total - self.kept.len() as u64;
        let end = offset.checked_add(length)?;
        if offset < start || end > self.total {
            return None;
        }
        let from = usize::try_from(offset - start).ok()?;
        let to = usize::try_from(end - start).ok()?;
        Some(&self.kept[from..to])
    }
}

/// One entry of the central directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// Where its local header starts.
    pub offset: u64,
    pub size: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TailError {
    #[error("the stage has no end of central directory: it is not a whole zip")]
    NoEnd,
    #[error("the stage's central directory is not within the last bytes kept")]
    OutOfReach,
    #[error("the stage's central directory is malformed: {0}")]
    Malformed(&'static str),
}

/// The central directory's entries.
pub fn entries(tail: &Tail) -> Result<Vec<Entry>, TailError> {
    let (count, size, offset) = directory_location(tail)?;
    let directory = tail.at(offset, size).ok_or(TailError::OutOfReach)?;
    let mut entries = Vec::new();
    let mut at = 0usize;
    while at < directory.len() {
        let record = directory
            .get(at..)
            .ok_or(TailError::Malformed("a record runs past the end"))?;
        if read_u32(record, 0) != Some(CENTRAL) {
            return Err(TailError::Malformed("a record has no signature"));
        }
        let field16 =
            |position| read_u16(record, position).ok_or(TailError::Malformed("a record is short"));
        let field32 =
            |position| read_u32(record, position).ok_or(TailError::Malformed("a record is short"));
        let flags = field16(8)?;
        let method = field16(10)?;
        let mut compressed = u64::from(field32(20)?);
        let mut uncompressed = u64::from(field32(24)?);
        let name_length = usize::from(field16(28)?);
        let extra_length = usize::from(field16(30)?);
        let comment_length = usize::from(field16(32)?);
        let mut local = u64::from(field32(42)?);
        let name = record
            .get(46..46 + name_length)
            .ok_or(TailError::Malformed("a name runs past the end"))?;
        let extra = record
            .get(46 + name_length..46 + name_length + extra_length)
            .ok_or(TailError::Malformed("an extra field runs past the end"))?;
        if flags & 0x0800 == 0 && !name.is_ascii() {
            return Err(TailError::Malformed(
                "a name is neither ASCII nor flagged UTF-8",
            ));
        }
        let name = std::str::from_utf8(name)
            .map_err(|_| TailError::Malformed("a name is not UTF-8"))?
            .to_string();
        let mut zip64 = zip64_values(extra);
        for value in [&mut uncompressed, &mut compressed, &mut local] {
            if *value == u64::from(u32::MAX) {
                *value = zip64
                    .next()
                    .ok_or(TailError::Malformed("a zip64 value is missing"))?;
            }
        }
        if method != 0 || compressed != uncompressed {
            return Err(TailError::Malformed("an entry is not stored"));
        }
        if local.checked_add(compressed).is_none_or(|end| end > offset) {
            return Err(TailError::Malformed(
                "an entry runs into the central directory",
            ));
        }
        entries.push(Entry {
            name,
            offset: local,
            size: uncompressed,
        });
        at += 46 + name_length + extra_length + comment_length;
    }
    if entries.len() as u64 != count {
        return Err(TailError::Malformed("the entry count does not match"));
    }
    Ok(entries)
}

/// The data of `entry`, if it is kept in the tail.
pub fn data<'a>(tail: &'a Tail, entry: &Entry) -> Result<&'a [u8], TailError> {
    let header = tail.at(entry.offset, 30).ok_or(TailError::OutOfReach)?;
    if read_u32(header, 0) != Some(LOCAL) {
        return Err(TailError::Malformed("a local header has no signature"));
    }
    let name_length = u64::from(read_u16(header, 26).unwrap_or_default());
    let extra_length = u64::from(read_u16(header, 28).unwrap_or_default());
    tail.at(entry.offset + 30 + name_length + extra_length, entry.size)
        .ok_or(TailError::OutOfReach)
}

/// (entries, directory size, directory offset), zip64 records preferred.
fn directory_location(tail: &Tail) -> Result<(u64, u64, u64), TailError> {
    let end = tail.total;
    // This writer adds no comment, so the record is the last 22 bytes; a
    // comment would follow it, so look back as far as one could reach.
    let window = 22 + u64::from(u16::MAX);
    let search_from = end.saturating_sub(window).max(end - tail.kept.len() as u64);
    let searched = tail
        .at(search_from, end - search_from)
        .ok_or(TailError::NoEnd)?;
    let position = (0..searched.len().saturating_sub(21))
        .rev()
        .find(|&at| read_u32(searched, at) == Some(EOCD))
        .ok_or(TailError::NoEnd)?;
    let record = &searched[position..];
    let eocd_offset = search_from + position as u64;
    let count = u64::from(read_u16(record, 10).ok_or(TailError::NoEnd)?);
    let size = u64::from(read_u32(record, 12).ok_or(TailError::NoEnd)?);
    let offset = u64::from(read_u32(record, 16).ok_or(TailError::NoEnd)?);
    let locator = eocd_offset.checked_sub(20).and_then(|at| tail.at(at, 20));
    if let Some(locator) = locator.filter(|locator| read_u32(locator, 0) == Some(ZIP64_LOCATOR)) {
        let zip64_offset =
            read_u64(locator, 8).ok_or(TailError::Malformed("short zip64 locator"))?;
        let record = tail.at(zip64_offset, 56).ok_or(TailError::OutOfReach)?;
        if read_u32(record, 0) != Some(ZIP64_EOCD) {
            return Err(TailError::Malformed(
                "the zip64 end record has no signature",
            ));
        }
        let read = |position| {
            read_u64(record, position).ok_or(TailError::Malformed("short zip64 end record"))
        };
        return Ok((read(32)?, read(40)?, read(48)?));
    }
    if count == u64::from(u16::MAX) || size == u64::from(u32::MAX) || offset == u64::from(u32::MAX)
    {
        return Err(TailError::Malformed("zip64 values without a zip64 locator"));
    }
    Ok((count, size, offset))
}

/// The 8-byte values of a zip64 extra field, in order.
fn zip64_values(extra: &[u8]) -> impl Iterator<Item = u64> + '_ {
    let mut at = 0;
    let mut field: &[u8] = &[];
    while at + 4 <= extra.len() {
        let id = read_u16(extra, at).unwrap_or_default();
        let length = usize::from(read_u16(extra, at + 2).unwrap_or_default());
        let body = extra.get(at + 4..at + 4 + length).unwrap_or_default();
        if id == ZIP64_EXTRA {
            field = body;
            break;
        }
        at += 4 + length;
    }
    field
        .chunks_exact(8)
        .map(|chunk| u64::from_le_bytes(chunk.try_into().unwrap_or_default()))
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

    fn stage(names: &[&str]) -> Vec<u8> {
        let mut stream = ZipStream::new(Vec::new());
        for name in names {
            stream.entry(name, 0o644, 0, &mut name.as_bytes()).unwrap();
        }
        stream.finish().unwrap()
    }

    fn tail_of(bytes: &[u8], max: usize, chunk: usize) -> Tail {
        let mut tail = Tail::new(max);
        for piece in bytes.chunks(chunk) {
            tail.push(piece);
        }
        tail
    }

    #[test]
    fn reads_every_name_and_its_data() {
        let bytes = stage(&[
            "agent/hermes-home/SOUL.md",
            "workspace/a b.md",
            ".finite-stage/summary.json",
        ]);
        let tail = tail_of(&bytes, 1 << 20, 7);
        let entries = entries(&tail).unwrap();
        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "agent/hermes-home/SOUL.md",
                "workspace/a b.md",
                ".finite-stage/summary.json"
            ]
        );
        assert_eq!(data(&tail, &entries[1]).unwrap(), b"workspace/a b.md");
    }

    #[test]
    fn a_directory_beyond_the_kept_tail_is_out_of_reach() {
        let names: Vec<String> = (0..2000)
            .map(|index| format!("workspace/{index:05}.md"))
            .collect();
        let bytes = stage(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let tail = tail_of(&bytes, 1024, 4096);
        assert_eq!(entries(&tail), Err(TailError::OutOfReach));
    }

    #[test]
    fn a_truncated_stream_has_no_end() {
        let bytes = stage(&["workspace/a.md"]);
        let tail = tail_of(&bytes[..bytes.len() - 30], 1 << 20, 64);
        assert!(entries(&tail).is_err());
    }
}
