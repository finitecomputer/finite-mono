//! The names in a streamed stage, read from its central directory, which
//! sits at the end: the uploader keeps the stream's last bytes and reads
//! the directory there once the stream ends, before the upload completes.
//!
//! It accepts exactly what `finite-mover-host`'s writer produces
//! (`finite_mover_host::zip64`), and nothing looser:
//!
//! - the end records are the stream's last 98 bytes: the zip64 end record,
//!   its locator, and an end record with no comment;
//! - the central directory ends where the zip64 end record starts, and each
//!   of its records is whole, with the writer's zip64 field and no comment;
//! - the entries tile the stream from byte 0 to the directory, each a local
//!   header, its stored data and a zip64 data descriptor, with no gap.
//!
//! Every entry whose bytes are still kept (all of them, for a stage under
//! the kept tail) is checked against its directory record: the local name
//! and flags, the descriptor, and the data's CRC.

const EOCD: u32 = 0x0605_4b50;
const ZIP64_EOCD: u32 = 0x0606_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;
const DESCRIPTOR: u32 = 0x0807_4b50;
const ZIP64_EXTRA: u16 = 0x0001;
/// Data descriptor and UTF-8 names.
const FLAGS: u16 = 0x0808;

/// The writer's fixed lengths.
const LOCAL_BYTES: u64 = 30;
const LOCAL_EXTRA_BYTES: u64 = 20;
const DESCRIPTOR_BYTES: u64 = 24;
const CENTRAL_BYTES: usize = 46;
const CENTRAL_EXTRA_BYTES: usize = 28;
const EOCD_BYTES: u64 = 22;
const LOCATOR_BYTES: u64 = 20;
const ZIP64_EOCD_BYTES: u64 = 56;

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
    pub crc: u32,
}

impl Entry {
    fn data_offset(&self) -> u64 {
        self.offset + LOCAL_BYTES + self.name.len() as u64 + LOCAL_EXTRA_BYTES
    }

    /// Where the next entry's local header starts.
    fn end(&self) -> Option<u64> {
        self.data_offset()
            .checked_add(self.size)?
            .checked_add(DESCRIPTOR_BYTES)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TailError {
    #[error("the stage does not end with its end records: it is not a whole zip")]
    NoEnd,
    #[error("the stage's central directory is not within the last bytes kept")]
    OutOfReach,
    #[error("the stage is malformed: {0}")]
    Malformed(&'static str),
}

fn malformed(why: &'static str) -> TailError {
    TailError::Malformed(why)
}

/// The central directory's entries, after every check this module makes.
pub fn entries(tail: &Tail) -> Result<Vec<Entry>, TailError> {
    let (count, size, offset) = directory_location(tail)?;
    let directory = tail.at(offset, size).ok_or(TailError::OutOfReach)?;
    let mut entries: Vec<Entry> = Vec::new();
    let mut at = 0usize;
    let mut next_local = 0u64;
    while at < directory.len() {
        let record = &directory[at..];
        if record.len() < CENTRAL_BYTES || read_u32(record, 0) != Some(CENTRAL) {
            return Err(malformed("a directory record is short or unsigned"));
        }
        let field16 = |position| read_u16(record, position).unwrap_or_default();
        let field32 = |position| read_u32(record, position).unwrap_or_default();
        let name_length = usize::from(field16(28));
        let extra_length = usize::from(field16(30));
        let comment_length = usize::from(field16(32));
        let length = CENTRAL_BYTES + name_length + extra_length + comment_length;
        if record.len() < length {
            return Err(malformed("a directory record runs past the directory"));
        }
        if extra_length != CENTRAL_EXTRA_BYTES || comment_length != 0 {
            return Err(malformed("a directory record is not the writer's"));
        }
        if field16(8) != FLAGS || field16(10) != 0 {
            return Err(malformed("an entry is not stored with a data descriptor"));
        }
        if [field32(20), field32(24), field32(42)] != [u32::MAX; 3] {
            return Err(malformed("an entry's sizes are not in its zip64 field"));
        }
        let extra =
            &record[CENTRAL_BYTES + name_length..CENTRAL_BYTES + name_length + extra_length];
        if read_u16(extra, 0) != Some(ZIP64_EXTRA) || read_u16(extra, 2) != Some(24) {
            return Err(malformed("an entry's zip64 field is not the writer's"));
        }
        let [uncompressed, compressed, local] =
            [4, 12, 20].map(|position| read_u64(extra, position).unwrap_or_default());
        if uncompressed != compressed {
            return Err(malformed("an entry is not stored"));
        }
        if local != next_local {
            return Err(malformed("the entries do not follow one another"));
        }
        let name = std::str::from_utf8(&record[CENTRAL_BYTES..CENTRAL_BYTES + name_length])
            .map_err(|_| malformed("a name is not UTF-8"))?
            .to_string();
        let entry = Entry {
            name,
            offset: local,
            size: uncompressed,
            crc: field32(16),
        };
        next_local = entry.end().ok_or(malformed("an entry is too large"))?;
        entries.push(entry);
        at += length;
    }
    if next_local != offset {
        return Err(malformed("the entries do not reach the central directory"));
    }
    if entries.len() as u64 != count {
        return Err(malformed("the entry count does not match"));
    }
    for entry in &entries {
        if tail
            .at(entry.offset, entry.end().unwrap_or(u64::MAX) - entry.offset)
            .is_some()
        {
            check_entry(tail, entry)?;
        }
    }
    Ok(entries)
}

/// An entry whose bytes are kept, against its directory record.
fn check_entry(tail: &Tail, entry: &Entry) -> Result<(), TailError> {
    let name_length = entry.name.len() as u64;
    let header = tail
        .at(entry.offset, LOCAL_BYTES + name_length + LOCAL_EXTRA_BYTES)
        .ok_or(TailError::OutOfReach)?;
    let local_name = &header[30..30 + entry.name.len()];
    if read_u32(header, 0) != Some(LOCAL)
        || read_u16(header, 6) != Some(FLAGS)
        || read_u16(header, 8) != Some(0)
        || read_u16(header, 26).map(u64::from) != Some(name_length)
        || read_u16(header, 28).map(u64::from) != Some(LOCAL_EXTRA_BYTES)
        || local_name != entry.name.as_bytes()
    {
        return Err(malformed(
            "a local header does not match its directory record",
        ));
    }
    let data = tail
        .at(entry.data_offset(), entry.size)
        .ok_or(TailError::OutOfReach)?;
    if crc32fast::hash(data) != entry.crc {
        return Err(malformed("an entry's data does not match its CRC"));
    }
    let descriptor = tail
        .at(entry.data_offset() + entry.size, DESCRIPTOR_BYTES)
        .ok_or(TailError::OutOfReach)?;
    if read_u32(descriptor, 0) != Some(DESCRIPTOR)
        || read_u32(descriptor, 4) != Some(entry.crc)
        || read_u64(descriptor, 8) != Some(entry.size)
        || read_u64(descriptor, 16) != Some(entry.size)
    {
        return Err(malformed(
            "a data descriptor does not match its directory record",
        ));
    }
    Ok(())
}

/// The data of `entry`, checked, if it is kept in the tail.
pub fn data<'a>(tail: &'a Tail, entry: &Entry) -> Result<&'a [u8], TailError> {
    check_entry(tail, entry)?;
    tail.at(entry.data_offset(), entry.size)
        .ok_or(TailError::OutOfReach)
}

/// (entries, directory size, directory offset), from end records that are
/// exactly the stream's last bytes.
fn directory_location(tail: &Tail) -> Result<(u64, u64, u64), TailError> {
    let end = tail.total;
    let zip64_offset = end
        .checked_sub(EOCD_BYTES + LOCATOR_BYTES + ZIP64_EOCD_BYTES)
        .ok_or(TailError::NoEnd)?;
    let ends = tail
        .at(zip64_offset, EOCD_BYTES + LOCATOR_BYTES + ZIP64_EOCD_BYTES)
        .ok_or(TailError::NoEnd)?;
    let (record, rest) = ends.split_at(56);
    let (locator, eocd) = rest.split_at(20);
    if read_u32(eocd, 0) != Some(EOCD)
        || read_u32(locator, 0) != Some(ZIP64_LOCATOR)
        || read_u32(record, 0) != Some(ZIP64_EOCD)
    {
        return Err(TailError::NoEnd);
    }
    let read = |bytes: &[u8], position| read_u64(bytes, position).unwrap_or(u64::MAX);
    let (count, size, offset) = (read(record, 32), read(record, 40), read(record, 48));
    let whole = read_u16(eocd, 4) == Some(0)
        && read_u16(eocd, 8) == read_u16(eocd, 10)
        && read_u16(eocd, 6) == Some(0)
        && read_u16(eocd, 20) == Some(0)
        && read_u32(locator, 4) == Some(0)
        && read(locator, 8) == zip64_offset
        && read_u32(locator, 16) == Some(1)
        && read(record, 4) == ZIP64_EOCD_BYTES - 12
        && read_u32(record, 16) == Some(0)
        && read_u32(record, 20) == Some(0)
        && read(record, 24) == count
        && offset.checked_add(size) == Some(zip64_offset)
        && read_u16(eocd, 10).map(u64::from) == Some(count.min(u64::from(u16::MAX)))
        && read_u32(eocd, 12).map(u64::from) == Some(size.min(u64::from(u32::MAX)))
        && read_u32(eocd, 16).map(u64::from) == Some(offset.min(u64::from(u32::MAX)));
    if !whole {
        return Err(malformed("the end records do not describe this stream"));
    }
    Ok((count, size, offset))
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

    /// The offset of the first `needle` in `bytes` at or after `from`.
    fn find(bytes: &[u8], needle: &[u8], from: usize) -> usize {
        from + bytes[from..]
            .windows(needle.len())
            .position(|window| window == needle)
            .unwrap()
    }

    fn check(bytes: &[u8]) -> Result<Vec<Entry>, TailError> {
        entries(&tail_of(bytes, 1 << 20, 4096))
    }

    #[test]
    fn a_stage_that_is_not_exactly_the_writers_is_refused() {
        let good = stage(&["workspace/notes.md", "workspace/report.md"]);
        assert!(check(&good).is_ok());
        let directory = find(&good, &CENTRAL.to_le_bytes(), 0);
        let eocd = good.len() - 22;
        let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
        // An end record that claims a comment the stream does not hold.
        let mut comment = good.clone();
        comment[eocd + 20] = 100;
        cases.push(("comment", comment));
        // Bytes after the end record.
        let mut trailing = good.clone();
        trailing.extend_from_slice(b"more");
        cases.push(("trailing", trailing));
        // A directory record whose comment would reach past the directory.
        let mut record = good.clone();
        let last = find(&good, &CENTRAL.to_le_bytes(), directory + 1);
        record[last + 32..last + 34].copy_from_slice(&u16::MAX.to_le_bytes());
        cases.push(("record comment", record));
        // A local name that differs from the directory's, same length.
        let mut local = good.clone();
        let name = find(&good, b"workspace/report.md", 0);
        local[name..name + 19].copy_from_slice(b"workspace//.env.txt");
        cases.push(("local name", local));
        // A payload byte changed without its CRC.
        let mut payload = good.clone();
        let data = find(
            &good,
            b"workspace/notes.md",
            find(&good, b"workspace/notes.md", 0) + 1,
        );
        payload[data] ^= 1;
        cases.push(("payload", payload));
        // A gap between two entries.
        let mut gap = good.clone();
        gap.insert(find(&good, &LOCAL.to_le_bytes(), 1), 0);
        cases.push(("gap", gap));
        for (label, bytes) in cases {
            assert!(check(&bytes).is_err(), "{label} must be refused");
        }
    }

    #[test]
    fn a_stream_cut_after_a_zip_inside_it_is_refused() {
        // A workspace zip, staged whole, holds its own end records.
        let inner = stage(&["inner.md"]);
        let mut stream = ZipStream::new(Vec::new());
        stream
            .entry("workspace/archive.zip", 0o644, 0, &mut &inner[..])
            .unwrap();
        stream
            .entry("workspace/after.md", 0o644, 0, &mut &b"after"[..])
            .unwrap();
        let outer = stream.finish().unwrap();
        let cut = find(&outer, &inner[inner.len() - 22..], 0) + 22;
        assert!(check(&outer[..cut]).is_err());
        assert!(check(&outer).is_ok());
    }

    #[test]
    fn a_truncated_stream_has_no_end() {
        let bytes = stage(&["workspace/a.md"]);
        let tail = tail_of(&bytes[..bytes.len() - 30], 1 << 20, 64);
        assert!(entries(&tail).is_err());
    }
}
