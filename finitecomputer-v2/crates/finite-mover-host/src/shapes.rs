//! Credential-shaped text: a port of the export's `TOKEN_SHAPES`,
//! `is_secret` and `secret_in_file`
//! (finite-agentd/integrations/hermes/finite-export/dashboard/export.py:225-249,
//! 386-403, 476-503), over bytes and without a regex engine.
//!
//! It is at least as strict as the export: every alternative is tried at
//! every position, where the export's `finditer` keeps only the first
//! match at each.

use std::io::{self, Read};

/// What reading a file for credentials found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextVerdict {
    /// Text with no credential-shaped string.
    Clean,
    /// Binary (a NUL in the first 8 KiB, without a UTF-16 byte order
    /// mark): not read, as the export does not.
    Binary,
    /// Holds a credential-shaped string.
    Secret,
    /// Holds an unbroken run too long to carry between chunks.
    Uncheckable,
}

const CHUNK: usize = 1024 * 1024;
const BINARY_PROBE: usize = 8192;
/// The scan always carries at least this much text into the next chunk.
const CARRY_MIN: usize = 4096;
/// The longest unbroken run carried between chunks (`SCAN_CARRY_MAX`).
const CARRY_MAX: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
enum Encoding {
    Bytes,
    Utf16Le,
    Utf16Be,
}

/// The largest text file held whole while it is read for credentials:
/// fragment's blob limit, past which the import leaves a file behind
/// anyway.
pub const TEXT_SNAPSHOT_MAX: u64 = 256 * 1024 * 1024;

/// What reading a file once found, with the bytes a text file is staged
/// from.
#[derive(Debug, PartialEq, Eq)]
pub enum Snapshot {
    /// Binary: its contents are not read for credentials, as the export's
    /// are not.
    Binary,
    /// Clean text, every byte of it: the stage holds exactly what was
    /// checked, whatever the file holds a moment later.
    Text(Vec<u8>),
    /// Text holding a credential-shaped string.
    Secret,
    /// Text too large to hold (`TEXT_SNAPSHOT_MAX`), or with a run too long
    /// to check.
    Uncheckable,
}

/// Read a file once: decide whether it is binary from its first bytes, and
/// hold a text file whole to check it and to stage it from.
pub fn snapshot(mut reader: impl Read, size: u64) -> io::Result<Snapshot> {
    let mut first = vec![0; BINARY_PROBE];
    let probed = read_full(&mut reader, &mut first)?;
    first.truncate(probed);
    if !first.starts_with(b"\xff\xfe") && !first.starts_with(b"\xfe\xff") && first.contains(&0) {
        return Ok(Snapshot::Binary);
    }
    if size > TEXT_SNAPSHOT_MAX {
        return Ok(Snapshot::Uncheckable);
    }
    let mut text = first;
    reader
        .take(TEXT_SNAPSHOT_MAX + 1 - text.len() as u64)
        .read_to_end(&mut text)?;
    if text.len() as u64 > TEXT_SNAPSHOT_MAX {
        return Ok(Snapshot::Uncheckable);
    }
    Ok(match scan(&text[..])? {
        TextVerdict::Clean => Snapshot::Text(text),
        // A NUL past the first 8 KiB: as text it was read, and it is clean.
        TextVerdict::Binary => Snapshot::Text(text),
        TextVerdict::Secret => Snapshot::Secret,
        TextVerdict::Uncheckable => Snapshot::Uncheckable,
    })
}

/// Read `reader` to its end, or until a credential is found.
pub fn scan(mut reader: impl Read) -> io::Result<TextVerdict> {
    let mut chunk = vec![0; CHUNK];
    let mut filled = read_full(&mut reader, &mut chunk)?;
    let encoding = if chunk[..filled].starts_with(b"\xff\xfe") {
        Encoding::Utf16Le
    } else if chunk[..filled].starts_with(b"\xfe\xff") {
        Encoding::Utf16Be
    } else if chunk[..filled.min(BINARY_PROBE)].contains(&0) {
        return Ok(TextVerdict::Binary);
    } else {
        Encoding::Bytes
    };
    let mut text = Vec::new();
    let mut odd: Option<u8> = None;
    while filled > 0 {
        decode(encoding, &chunk[..filled], &mut odd, &mut text);
        if holds_secret(&text) {
            return Ok(TextVerdict::Secret);
        }
        let start = carry_start(&text);
        text.drain(..start);
        if text.len() > CARRY_MAX {
            return Ok(TextVerdict::Uncheckable);
        }
        filled = read_full(&mut reader, &mut chunk)?;
    }
    Ok(TextVerdict::Clean)
}

fn read_full(reader: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

/// Append `chunk` to `text` as bytes the shapes can match. UTF-16 code
/// units outside ASCII become 0x80, which no shape contains.
fn decode(encoding: Encoding, chunk: &[u8], odd: &mut Option<u8>, text: &mut Vec<u8>) {
    let unit = |pair: [u8; 2]| match encoding {
        Encoding::Utf16Le => u16::from_le_bytes(pair),
        _ => u16::from_be_bytes(pair),
    };
    match encoding {
        Encoding::Bytes => text.extend_from_slice(chunk),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let mut bytes = chunk.iter().copied();
            if let Some(first) = odd.take() {
                match bytes.next() {
                    Some(second) => text.push(ascii(unit([first, second]))),
                    None => *odd = Some(first),
                }
            }
            let rest: Vec<u8> = bytes.collect();
            let mut pairs = rest.chunks_exact(2);
            for pair in &mut pairs {
                text.push(ascii(unit([pair[0], pair[1]])));
            }
            if let [last] = pairs.remainder() {
                *odd = Some(*last);
            }
        }
    }
}

fn ascii(unit: u16) -> u8 {
    u8::try_from(unit).ok().filter(u8::is_ascii).unwrap_or(0x80)
}

/// Where the next chunk's carried text starts: after the last whitespace,
/// but never less than the last 4 KiB (export.py:496-498).
fn carry_start(text: &[u8]) -> usize {
    let after_space = text
        .iter()
        .rposition(|byte| b" \n\r\t\x0c\x0b".contains(byte))
        .map_or(0, |space| space + 1);
    after_space.min(text.len().saturating_sub(CARRY_MIN))
}

/// Whether `text` holds a credential-shaped string that is no placeholder.
pub fn holds_secret(text: &[u8]) -> bool {
    (0..text.len()).any(|at| {
        let rest = &text[at..];
        // Only the first byte of a run of word bytes can start a token, so
        // each byte is looked at a bounded number of times.
        let word_start = at == 0 || !word(text[at - 1]);
        match rest[0] {
            b'-' => private_key_header(rest),
            b'd' => discord_webhook(rest),
            b'h' => url_password(rest) || (word_start && token(rest)),
            b's' | b'r' | b'g' | b'x' | b'A' | b'n' | b'e' | b'0'..=b'9' => {
                word_start && token(rest)
            }
            _ => false,
        }
    })
}

/// `[A-Za-z0-9_-]`, the class the export's lookbehind refuses before a token.
fn word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn alnum(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

fn alnum_underscore(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn alnum_dash(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}

fn upper_digit(byte: u8) -> bool {
    byte.is_ascii_uppercase() || byte.is_ascii_digit()
}

fn lower_digit(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

/// The length of the longest run of `class` at the start of `text`.
fn run(text: &[u8], class: fn(u8) -> bool) -> usize {
    text.iter().take_while(|byte| class(**byte)).count()
}

/// `prefix` then at least `min` of `class`: the whole match, greedily.
fn prefixed<'a>(
    text: &'a [u8],
    prefix: &[u8],
    class: fn(u8) -> bool,
    min: usize,
) -> Option<&'a [u8]> {
    let rest = text.strip_prefix(prefix)?;
    let length = run(rest, class);
    (length >= min).then(|| &text[..prefix.len() + length])
}

/// The `token` group of `TOKEN_SHAPES`, at a word start.
fn token(text: &[u8]) -> bool {
    let candidates = [
        prefixed(text, b"sk-", word, 16),
        prefixed(text, b"sk_live_", alnum, 16),
        prefixed(text, b"rk_live_", alnum, 16),
        prefixed(text, b"ghp_", alnum, 20),
        prefixed(text, b"github_pat_", alnum_underscore, 20),
        prefixed(text, b"glpat-", word, 20),
        slack(text),
        prefixed(text, b"AIza", word, 30),
        prefixed(text, b"AKIA", upper_digit, 16).map(|found| &found[..20]),
        prefixed(text, b"ntn_", alnum, 20),
        prefixed(text, b"hf_", alnum, 30),
        telegram(text),
        prefixed(text, b"nsec1", lower_digit, 20),
        jwt(text),
    ];
    candidates.into_iter().flatten().any(secret_token)
}

/// `xox[abpr]-[A-Za-z0-9-]{10,}`.
fn slack(text: &[u8]) -> Option<&[u8]> {
    let rest = text.strip_prefix(b"xox")?;
    let (kind, rest) = rest.split_first()?;
    if !b"abpr".contains(kind) {
        return None;
    }
    prefixed(rest, b"-", alnum_dash, 10).map(|found| &text[..4 + found.len()])
}

/// `\d{6,}:[A-Za-z0-9_-]{30,}`, a Telegram bot token.
fn telegram(text: &[u8]) -> Option<&[u8]> {
    let digits = run(text, |byte| byte.is_ascii_digit());
    if digits < 6 {
        return None;
    }
    prefixed(&text[digits..], b":", word, 30).map(|found| &text[..digits + found.len()])
}

/// `eyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{10,}`, a JWT.
fn jwt(text: &[u8]) -> Option<&[u8]> {
    let header = prefixed(text, b"eyJ", word, 20)?;
    let payload = prefixed(&text[header.len()..], b".", word, 20)?;
    let signature = prefixed(&text[header.len() + payload.len()..], b".", word, 10)?;
    Some(&text[..header.len() + payload.len() + signature.len()])
}

/// `is_secret` for a token: random-looking (a digit, or long) and no
/// placeholder.
fn secret_token(token: &[u8]) -> bool {
    !placeholder(token) && (token.iter().any(u8::is_ascii_digit) || token.len() >= 32)
}

/// `PLACEHOLDER` (export.py:245-247), ignoring case.
fn placeholder(text: &[u8]) -> bool {
    let lower = text.to_ascii_lowercase();
    let contains = |needle: &[u8]| lower.windows(needle.len()).any(|window| window == needle);
    contains(b"xxxx")
        || contains(b"****")
        || contains(b"...")
        || lower.iter().any(|byte| b"<>${}".contains(byte))
        || [
            &b"your"[..],
            b"example",
            b"placeholder",
            b"redacted",
            b"changeme",
        ]
        .iter()
        .any(|word| contains(word))
}

/// `-----BEGIN [A-Z ]*PRIVATE KEY-----`.
fn private_key_header(text: &[u8]) -> bool {
    let Some(rest) = text.strip_prefix(b"-----BEGIN ") else {
        return false;
    };
    let length = run(rest, |byte| byte.is_ascii_uppercase() || byte == b' ');
    rest[..length].ends_with(b"PRIVATE KEY") && rest[length..].starts_with(b"-----")
}

/// `discord(?:app)?\.com/api/webhooks/\d+/[\w-]+`.
fn discord_webhook(text: &[u8]) -> bool {
    let Some(rest) = text
        .strip_prefix(b"discord.com/api/webhooks/")
        .or_else(|| text.strip_prefix(b"discordapp.com/api/webhooks/"))
    else {
        return false;
    };
    let digits = run(rest, |byte| byte.is_ascii_digit());
    digits > 0
        && rest[digits..]
            .strip_prefix(b"/")
            .is_some_and(|id| run(id, |byte| word(byte) || byte >= 0x80) > 0)
}

/// `https?://[^\s/:@]+:(?P<password>[^\s/@]+)@`, with the export's rule for
/// a password: six characters or more, no placeholder, not an
/// environment-variable name.
fn url_password(text: &[u8]) -> bool {
    let Some(rest) = text
        .strip_prefix(b"https://")
        .or_else(|| text.strip_prefix(b"http://"))
    else {
        return false;
    };
    let user = run(rest, |byte| {
        !byte.is_ascii_whitespace() && !b"/:@".contains(&byte)
    });
    let Some(after_colon) = rest[user..].strip_prefix(b":").filter(|_| user > 0) else {
        return false;
    };
    let length = run(after_colon, |byte| {
        !byte.is_ascii_whitespace() && !b"/@".contains(&byte)
    });
    let password = &after_colon[..length];
    length > 0
        && after_colon[length..].starts_with(b"@")
        && password.len() >= 6
        && !placeholder(password)
        && !password
            .iter()
            .all(|byte| upper_digit(*byte) || *byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(text: &str) -> bool {
        holds_secret(text.as_bytes())
    }

    #[test]
    fn each_token_shape_is_found() {
        for text in [
            "key: sk-proj4abcdefghijklmnop",
            "sk_live_abcdefgh12345678",
            "rk_live_abcdefgh12345678",
            "ghp_abcdefghij0123456789",
            "github_pat_abcdefghij0123456789",
            "glpat-abcdefghij0123456789",
            "xoxb-123456789012-abc",
            "AIzaSyA1234567890abcdefghijklmnopqrs",
            "AKIA1234567890ABCDEF",
            "ntn_abcdefghij0123456789",
            "hf_abcdefghij0123456789abcdefghij",
            "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw1",
            "nsec1abcdefghij0123456789",
            "eyJhbGciOiJIUzI1NiIsInR5c.eyJzdWIiOiIxMjM0NTY3ODkwIiwib.SflKxwRJSMeKKF2QT4fw",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "-----BEGIN PRIVATE KEY-----",
            "https://discord.com/api/webhooks/123/abcDEF",
            "clone https://alice:hunter22secret@example.org/repo",
        ] {
            assert!(secret(text), "{text}");
        }
    }

    #[test]
    fn placeholders_and_words_are_not_secrets() {
        for text in [
            "sk-xxxxxxxxxxxxxxxxxxxxxxxx",
            "sk-your-key-goes-here-please",
            "coindesk-dl0123456789abcdefghij",
            "sk-is-a-slug-for-everything",
            "AKIAIOSFODNN7EXAMPLE",
            "https://USER:TOKEN@host",
            "https://user:${PASSWORD}@host",
            "https://user:pw@host",
            "-----BEGIN PUBLIC KEY-----",
            "nsec1short",
            "plain prose with no credential in it",
        ] {
            assert!(!secret(text), "{text}");
        }
    }

    #[test]
    fn a_token_split_across_chunks_is_found() {
        let mut text = vec![b'a'; CHUNK - 10];
        text.push(b' ');
        text.extend_from_slice(b"ghp_abcdefghij0123456789 tail");
        assert_eq!(scan(&text[..]).unwrap(), TextVerdict::Secret);
    }

    #[test]
    fn binary_is_not_read_and_utf16_is() {
        let mut binary = b"SQLite format 3\0".to_vec();
        binary.extend_from_slice(b"ghp_abcdefghij0123456789");
        assert_eq!(scan(&binary[..]).unwrap(), TextVerdict::Binary);
        let mut utf16 = vec![0xff, 0xfe];
        for unit in "token ghp_abcdefghij0123456789".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(scan(&utf16[..]).unwrap(), TextVerdict::Secret);
    }

    #[test]
    fn a_snapshot_holds_text_and_skips_binary() {
        let text = b"notes, and no credential\n".to_vec();
        assert_eq!(
            snapshot(&text[..], text.len() as u64).unwrap(),
            Snapshot::Text(text.clone())
        );
        let token = b"key ghp_abcdefghij0123456789\n";
        assert_eq!(snapshot(&token[..], 29).unwrap(), Snapshot::Secret);
        let binary = b"SQLite format 3\0ghp_abcdefghij0123456789";
        assert_eq!(snapshot(&binary[..], 40).unwrap(), Snapshot::Binary);
        assert_eq!(
            snapshot(&text[..], TEXT_SNAPSHOT_MAX + 1).unwrap(),
            Snapshot::Uncheckable,
            "a text file too large to hold is never read past its start"
        );
    }

    #[test]
    fn an_endless_run_is_uncheckable() {
        let text = vec![b'a'; CARRY_MAX + 2 * CHUNK];
        assert_eq!(scan(&text[..]).unwrap(), TextVerdict::Uncheckable);
    }
}
