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
//! `COHORT_MAX` entries, isn't this shape (an object, its one key `old`),
//! or holds an entry that isn't a project id is an error, so the run fails
//! and the next tries again. What it says of the file never quotes it,
//! except an entry shaped as Core mints project ids (`core_shaped`): a slip
//! could have pasted a person's email or phone number.
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
    /// The entry's place in the list, counted from 1, and never its text.
    #[error("{}: entry {place} is not an old agent's project id", .path.display())]
    NotAnId { path: PathBuf, place: usize },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CohortFile {
    old: Vec<String>,
}

/// The old agents a cohort lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cohort {
    /// As the file lists them, for saying where an entry is.
    entries: Vec<String>,
    old: BTreeSet<String>,
}

/// Whether `id` is shaped as Core mints a project's id (`project_` and 20
/// hex, its `generate_surrogate_id`): then it is no person's data, and may
/// be said as it is.
pub fn core_shaped(id: &str) -> bool {
    id.strip_prefix("project_").is_some_and(|hex| {
        hex.len() == 20
            && hex
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// What the roster says of an entry it has no live old agent for: by its
/// id if Core could have minted it, else by its place (counted from 1).
pub fn not_on_the_roster(place: usize, old: &str) -> String {
    let which = if core_shaped(old) {
        old.to_string()
    } else {
        format!("entry {place}")
    };
    format!(
        "finite-mover: the cohort's {which} is not on the roster: Core has no live old agent by that id that the mover can move"
    )
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
        // An object alone: a derived struct would also take an array of its
        // fields in order, so `[["proj_a"]]` would list proj_a.
        let start = bytes
            .iter()
            .position(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'));
        if let Some(at) = start.filter(|at| bytes[*at] != b'{') {
            let line = 1 + bytes[..at].iter().filter(|byte| **byte == b'\n').count();
            let column = at
                - bytes[..at]
                    .iter()
                    .rposition(|byte| *byte == b'\n')
                    .map_or(0, |nl| nl + 1)
                + 1;
            return Err(CohortError::Malformed {
                path: path.to_path_buf(),
                line,
                column,
            });
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
                place: index + 1,
            });
        }
        Ok(Self {
            old: listed.old.iter().cloned().collect(),
            entries: listed.old,
        })
    }

    /// Whether it lists old agent `old`.
    pub fn lists(&self, old: &str) -> bool {
        self.old.contains(old)
    }

    /// Every entry, as the file lists them, with its place (from 1).
    pub fn entries(&self) -> impl Iterator<Item = (usize, &str)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(index, old)| (index + 1, old.as_str()))
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
    fn it_lists_its_ids_alone() {
        let cohort = read(r#" {"old": ["proj_b", "proj_a", "proj_b"]}"#).unwrap();
        assert_eq!(
            cohort.entries().collect::<Vec<_>>(),
            [(1, "proj_b"), (2, "proj_a"), (3, "proj_b")]
        );
        assert!(cohort.lists("proj_a") && cohort.lists("proj_b") && !cohort.lists("proj_c"));
        assert_eq!(read(r#"{"old": []}"#).unwrap(), Cohort::default());
    }

    #[test]
    fn an_entry_off_the_roster_is_said_by_id_only_if_core_could_have_minted_it() {
        let minted = "project_0123456789abcdef0123";
        assert!(not_on_the_roster(2, minted).contains(&format!("the cohort's {minted} is not")));
        for slip in [
            "15551234567",
            "proj_live",
            "project_0123456789ABCDEF0123",
            "project_0123456789abcdef012",
        ] {
            let said = not_on_the_roster(7, slip);
            assert!(
                said.contains("the cohort's entry 7 is not") && !said.contains(slip),
                "{said}"
            );
        }
    }

    #[test]
    fn a_file_that_is_not_there_lists_no_one() {
        let dir = tempfile::tempdir().unwrap();
        let cohort = Cohort::read(&dir.path().join("cohort.json")).unwrap();
        assert_eq!(cohort, Cohort::default());
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
            // The fields in order, as an array: never a cohort.
            r#"[["proj_live"]]"#,
            r#" ["proj_live"]"#,
            "\n  [[]]",
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
            Err(CohortError::NotAnId { place: 2, .. })
        ));
        assert!(matches!(
            read("\n  [[]]"),
            Err(CohortError::Malformed {
                line: 2,
                column: 3,
                ..
            })
        ));
    }

    #[test]
    fn a_file_past_its_limits_is_an_error() {
        let most: Vec<String> = (0..COHORT_MAX)
            .map(|n| format!("{n:0>width$}", width = OLD_ID_MAX_BYTES))
            .collect();
        let pretty = serde_json::to_string_pretty(&serde_json::json!({ "old": most })).unwrap();
        assert_eq!(read(&pretty).unwrap().entries().count(), COHORT_MAX);

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
