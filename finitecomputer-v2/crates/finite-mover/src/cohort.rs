//! The operator's cohort: the old agents the move is offered to while it
//! is offered to a few people at a time (phase 1's start), not everyone.
//! Root edits one file on the mover's host, which each run reads again,
//! as the overrides file is, so it changes with no release:
//!
//! ```json
//! { "old": ["proj_123", "proj_456"] }
//! ```
//!
//! Each entry is an old agent's Core project id (`valid_old`). With a
//! cohort, the roster lists only those, and the mover stops or retires no
//! other (`PgFleet`). With none configured, everyone is offered.
//!
//! It fails closed, never open to everyone: a file that isn't there lists
//! no one; one that can't be read, is over `COHORT_FILE_MAX` bytes or
//! `COHORT_MAX` entries, isn't this shape, or holds an entry that isn't a
//! project id is an error, so the run fails and the next tries again.
//!
//! A cohort only grows in practice. finite.chat withdraws the offer of an
//! old agent a roster leaves out only while its person hasn't said Yes; a
//! move under way goes on, but this mover refuses its stop and retirement
//! once it is out, so it fails or waits for staff. Taking someone out
//! takes back an offer not yet answered Yes; putting them back offers it
//! again.

use std::collections::BTreeSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::contract::{OLD_ID_MAX_BYTES, valid_old};

/// The most old agents a cohort lists. A cohort is a few people at a time:
/// one this large is everyone, which is no cohort at all.
pub const COHORT_MAX: usize = 2_000;

/// The largest cohort file read: `COHORT_MAX` ids at their longest, each
/// quoted, on its own indented line.
pub const COHORT_FILE_MAX: u64 = (COHORT_MAX * (OLD_ID_MAX_BYTES + 8) + 64) as u64;

#[derive(Debug, thiserror::Error)]
pub enum CohortError {
    #[error("{}: {why}", .path.display())]
    Unreadable { path: PathBuf, why: String },
    #[error("{}: over {COHORT_FILE_MAX} bytes", .0.display())]
    TooLarge(PathBuf),
    /// Where it stops being `{"old": [<project id>, ...]}`, and never what
    /// it holds there: a slip could have pasted a person's email.
    #[error("{}: not {{\"old\": [<project id>, ...]}} (line {line}, column {column})", .path.display())]
    Malformed {
        path: PathBuf,
        line: usize,
        column: usize,
    },
    #[error("{}: more than {COHORT_MAX} old agents", .0.display())]
    TooMany(PathBuf),
    /// The entry's place in the list, and never its text, as above.
    #[error("{}: entry {index} is not an old agent's project id", .path.display())]
    NotAnId { path: PathBuf, index: usize },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CohortFile {
    old: Vec<String>,
}

/// The old agents a cohort lists. The same id twice is listed once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cohort {
    old: BTreeSet<String>,
}

impl Cohort {
    /// The cohort in `path` now. No file lists no one.
    pub fn read(path: &Path) -> Result<Self, CohortError> {
        let file = match std::fs::File::open(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            other => other.map_err(|error| CohortError::Unreadable {
                path: path.to_path_buf(),
                why: error.to_string(),
            })?,
        };
        let mut bytes = Vec::new();
        file.take(COHORT_FILE_MAX + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| CohortError::Unreadable {
                path: path.to_path_buf(),
                why: error.to_string(),
            })?;
        if bytes.len() as u64 > COHORT_FILE_MAX {
            return Err(CohortError::TooLarge(path.to_path_buf()));
        }
        let listed: CohortFile =
            serde_json::from_slice(&bytes).map_err(|error| CohortError::Malformed {
                path: path.to_path_buf(),
                line: error.line(),
                column: error.column(),
            })?;
        if listed.old.len() > COHORT_MAX {
            return Err(CohortError::TooMany(path.to_path_buf()));
        }
        if let Some(index) = listed.old.iter().position(|old| !valid_old(old)) {
            return Err(CohortError::NotAnId {
                path: path.to_path_buf(),
                index,
            });
        }
        Ok(Self {
            old: listed.old.into_iter().collect(),
        })
    }

    /// Whether it lists old agent `old`.
    pub fn lists(&self, old: &str) -> bool {
        self.old.contains(old)
    }

    /// Every old agent it lists, in order.
    pub fn old(&self) -> impl Iterator<Item = &str> {
        self.old.iter().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<Cohort, CohortError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cohort.json");
        std::fs::write(&path, text).unwrap();
        Cohort::read(&path)
    }

    #[test]
    fn it_lists_its_ids_alone_each_once() {
        let cohort = read(r#"{"old": ["proj_b", "proj_a", "proj_b"]}"#).unwrap();
        assert_eq!(cohort.old().collect::<Vec<_>>(), ["proj_a", "proj_b"]);
        assert!(cohort.lists("proj_a") && !cohort.lists("proj_c"));
        assert_eq!(read(r#"{"old": []}"#).unwrap(), Cohort::default());
    }

    #[test]
    fn a_file_that_is_not_there_lists_no_one() {
        let dir = tempfile::tempdir().unwrap();
        let cohort = Cohort::read(&dir.path().join("cohort.json")).unwrap();
        assert_eq!(cohort.old().count(), 0);
    }

    #[test]
    fn a_file_not_of_its_shape_is_an_error_that_never_quotes_it() {
        for text in [
            "",
            "{",
            "[]",
            "{}",
            r#"{"old": "proj_a"}"#,
            r#"{"old": ["proj_a"], "everyone": true}"#,
            r#"{"old": [1]}"#,
            r#"{"old": ["someone@example.test"]}"#,
        ] {
            let error = read(text).unwrap_err();
            assert!(
                matches!(
                    error,
                    CohortError::Malformed { .. } | CohortError::NotAnId { .. }
                ),
                "{text}: {error}"
            );
            assert!(!error.to_string().contains("example"), "{error}");
        }
        assert!(matches!(
            read(r#"{"old": ["proj_a", "proj b"]}"#),
            Err(CohortError::NotAnId { index: 1, .. })
        ));
    }

    #[test]
    fn a_file_past_its_limits_is_an_error() {
        let most: Vec<String> = (0..COHORT_MAX)
            .map(|n| format!("{n:0>width$}", width = OLD_ID_MAX_BYTES))
            .collect();
        let pretty = serde_json::to_string_pretty(&serde_json::json!({ "old": most })).unwrap();
        assert_eq!(read(&pretty).unwrap().old().count(), COHORT_MAX);

        let short: Vec<String> = (0..=COHORT_MAX).map(|n| format!("p{n}")).collect();
        assert!(matches!(
            read(&serde_json::json!({ "old": short }).to_string()),
            Err(CohortError::TooMany(_))
        ));
        let padded = format!(r#"{{"old": []}}{}"#, " ".repeat(COHORT_FILE_MAX as usize));
        assert!(matches!(read(&padded), Err(CohortError::TooLarge(_))));
    }

    #[test]
    fn a_file_that_cannot_be_read_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the file should be: there, but not readable as one.
        assert!(matches!(
            Cohort::read(dir.path()),
            Err(CohortError::Unreadable { .. })
        ));
    }
}
