//! A streaming writer of stored (uncompressed) zip64 archives, the stage's
//! format (PKWARE APPNOTE 6.3.10).
//!
//! Bespoke because the workspace's zip 4.6.1 refuses, when streaming, any
//! entry over 4 GiB even with `large_file` set (its `write_data_descriptor`
//! checks the size the wrong way round), and zip 9, which fixes it, needs
//! a newer `libc` for the whole workspace. Every entry is stored and
//! carries zip64 sizes, so one rule covers a 1-byte file and a 99 GB one,
//! and any file's bytes sit contiguously where one range request reaches
//! them. The local header carries the size, which the caller knows up
//! front, so a reader of the stream (the mover's uploader) learns each
//! entry's name and length before its bytes; the CRC follows the data in a
//! zip64 data descriptor.

use std::io::{self, Read, Write};

/// Version 4.5: zip64.
const VERSION: u16 = 45;
/// Made on Unix, so readers honour the permissions in the external
/// attributes.
const MADE_BY: u16 = (3 << 8) | VERSION;
/// Bit 3: sizes and CRC follow the data. Bit 11: names are UTF-8.
const FLAGS: u16 = 0x0008 | 0x0800;
const ZIP64_EXTRA: u16 = 0x0001;
const SENTINEL_32: u32 = u32::MAX;
const SENTINEL_16: u16 = u16::MAX;
const S_IFREG: u32 = 0o100_000;

/// One entry's line in the central directory.
struct Central {
    name: Vec<u8>,
    mode: u32,
    time: u16,
    date: u16,
    crc: u32,
    size: u64,
    offset: u64,
}

/// Writes entries one after another to `out`, which need not seek.
pub struct ZipStream<W: Write> {
    out: Counted<W>,
    central: Vec<Central>,
}

impl<W: Write> ZipStream<W> {
    pub fn new(out: W) -> Self {
        Self {
            out: Counted {
                inner: out,
                written: 0,
            },
            central: Vec::new(),
        }
    }

    /// Write one entry from `data`, read to its end, with Unix `mode` and
    /// modification time `mtime` (seconds since 1970). Returns its size.
    pub fn entry(
        &mut self,
        name: &str,
        mode: u32,
        mtime: i64,
        size: u64,
        data: &mut impl Read,
    ) -> io::Result<()> {
        let name = name.as_bytes().to_vec();
        if name.is_empty() || name.len() > usize::from(u16::MAX) || name.contains(&0) {
            return Err(io::Error::other(
                "a zip entry name is 1 to 65535 bytes without NUL",
            ));
        }
        let offset = self.out.written;
        let (time, date) = dos_time(mtime);
        let mut header = Vec::with_capacity(30 + name.len() + 20);
        put32(&mut header, 0x0403_4b50);
        put16(&mut header, VERSION);
        put16(&mut header, FLAGS);
        put16(&mut header, 0);
        put16(&mut header, time);
        put16(&mut header, date);
        put32(&mut header, 0);
        put32(&mut header, SENTINEL_32);
        put32(&mut header, SENTINEL_32);
        put16(&mut header, name.len() as u16);
        put16(&mut header, 20);
        header.extend_from_slice(&name);
        // The size up front, so a reader of the stream (the mover's
        // uploader) knows each entry's name before its bytes and where it
        // ends; the CRC follows the data, in the descriptor.
        put16(&mut header, ZIP64_EXTRA);
        put16(&mut header, 16);
        put64(&mut header, size);
        put64(&mut header, size);
        self.out.write_all(&header)?;

        let mut hasher = crc32fast::Hasher::new();
        let mut written = 0u64;
        let mut buffer = vec![0; 1024 * 1024];
        let mut data = data.take(size);
        loop {
            let read = match data.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            hasher.update(&buffer[..read]);
            self.out.write_all(&buffer[..read])?;
            written += read as u64;
        }
        if written != size {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "an entry ended before its declared size",
            ));
        }
        let crc = hasher.finalize();
        let mut descriptor = Vec::with_capacity(24);
        put32(&mut descriptor, 0x0807_4b50);
        put32(&mut descriptor, crc);
        put64(&mut descriptor, size);
        put64(&mut descriptor, size);
        self.out.write_all(&descriptor)?;
        self.central.push(Central {
            name,
            mode,
            time,
            date,
            crc,
            size,
            offset,
        });
        Ok(())
    }

    /// Write the central directory and its zip64 end records; return `out`.
    pub fn finish(mut self) -> io::Result<W> {
        let start = self.out.written;
        for entry in &self.central {
            let mut record = Vec::with_capacity(46 + entry.name.len() + 28);
            put32(&mut record, 0x0201_4b50);
            put16(&mut record, MADE_BY);
            put16(&mut record, VERSION);
            put16(&mut record, FLAGS);
            put16(&mut record, 0);
            put16(&mut record, entry.time);
            put16(&mut record, entry.date);
            put32(&mut record, entry.crc);
            put32(&mut record, SENTINEL_32);
            put32(&mut record, SENTINEL_32);
            put16(&mut record, entry.name.len() as u16);
            put16(&mut record, 28);
            put16(&mut record, 0);
            put16(&mut record, 0);
            put16(&mut record, 0);
            put32(&mut record, (S_IFREG | (entry.mode & 0o777)) << 16);
            put32(&mut record, SENTINEL_32);
            record.extend_from_slice(&entry.name);
            put16(&mut record, ZIP64_EXTRA);
            put16(&mut record, 24);
            put64(&mut record, entry.size);
            put64(&mut record, entry.size);
            put64(&mut record, entry.offset);
            self.out.write_all(&record)?;
        }
        let end = self.out.written;
        let count = self.central.len() as u64;
        let mut tail = Vec::with_capacity(56 + 20 + 22);
        // Zip64 end of central directory record.
        put32(&mut tail, 0x0606_4b50);
        put64(&mut tail, 44);
        put16(&mut tail, MADE_BY);
        put16(&mut tail, VERSION);
        put32(&mut tail, 0);
        put32(&mut tail, 0);
        put64(&mut tail, count);
        put64(&mut tail, count);
        put64(&mut tail, end - start);
        put64(&mut tail, start);
        // Its locator.
        put32(&mut tail, 0x0706_4b50);
        put32(&mut tail, 0);
        put64(&mut tail, end);
        put32(&mut tail, 1);
        // End of central directory, with sentinels wherever a value does
        // not fit.
        put32(&mut tail, 0x0605_4b50);
        put16(&mut tail, 0);
        put16(&mut tail, 0);
        let entries = u16::try_from(count).unwrap_or(SENTINEL_16);
        put16(&mut tail, entries);
        put16(&mut tail, entries);
        put32(&mut tail, u32::try_from(end - start).unwrap_or(SENTINEL_32));
        put32(&mut tail, u32::try_from(start).unwrap_or(SENTINEL_32));
        put16(&mut tail, 0);
        self.out.write_all(&tail)?;
        self.out.flush()?;
        Ok(self.out.inner)
    }
}

struct Counted<W> {
    inner: W,
    written: u64,
}

impl<W: Write> Write for Counted<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// A Unix time as an MS-DOS time and date in UTC, clamped to what they
/// hold (1980 to 2107, two-second steps).
pub fn dos_time(seconds: i64) -> (u16, u16) {
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    if year < 1980 {
        return (0, (1 << 5) | 1);
    }
    if year > 2107 {
        return ((23 << 11) | (59 << 5) | 29, (127 << 9) | (12 << 5) | 31);
    }
    let second_of_day = seconds.rem_euclid(86_400);
    let time = ((second_of_day / 3600) << 11)
        | ((second_of_day / 60 % 60) << 5)
        | ((second_of_day % 60) / 2);
    let date = ((year - 1980) << 9) | (month << 5) | day;
    (time as u16, date as u16)
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use super::*;

    #[test]
    fn unix_times_become_dos_times() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        let (time, date) = dos_time(1_704_067_200 + 13 * 3600 + 5 * 60 + 9);
        assert_eq!((date >> 9, (date >> 5) & 15, date & 31), (44, 1, 1));
        assert_eq!((time >> 11, (time >> 5) & 63, (time & 31) * 2), (13, 5, 8));
        assert_eq!(dos_time(0), (0, 33));
        assert_eq!(dos_time(i64::from(u32::MAX) * 4).1 >> 9, 127);
    }

    #[test]
    fn a_reader_opens_what_it_writes() {
        let mut stream = ZipStream::new(Vec::new());
        stream
            .entry("a/one.txt", 0o640, 1_704_067_200, 3, &mut &b"one"[..])
            .unwrap();
        stream.entry("b/empty", 0o600, 0, 0, &mut &b""[..]).unwrap();
        // A source shorter than its declared size is an error, never a
        // short entry.
        let mut short = ZipStream::new(Vec::new());
        let error = short
            .entry("c/short", 0o600, 0, 5, &mut &b"abc"[..])
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
        let bytes = stream.finish().unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 2);
        let mut one = archive.by_name("a/one.txt").unwrap();
        assert_eq!(one.unix_mode(), Some(S_IFREG | 0o640));
        let mut text = String::new();
        one.read_to_string(&mut text).unwrap();
        assert_eq!(text, "one");
        drop(one);
        assert_eq!(archive.by_name("b/empty").unwrap().size(), 0);
    }
}
