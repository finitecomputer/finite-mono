//! Totals by class and what was left behind. `stream` writes the exact
//! summary as the stage's last entry; `sizes` prints the same shape,
//! estimated from metadata.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::classify::{Class, Verdict, Why};
use crate::simplex::Carry;

pub const FORMAT: &str = "finite-mover-host/summary/1";
/// The most `left` lines a summary lists; the rest are counted.
pub const LEFT_MAX: usize = 1000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tally {
    pub files: u64,
    pub bytes: u64,
}

impl Tally {
    fn add(&mut self, files: u64, bytes: u64) {
        self.files += files;
        self.bytes += bytes;
    }
}

/// One line of what stayed behind: a path (a directory when a whole
/// directory stayed for one reason), its class and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Left {
    pub path: String,
    pub class: Class,
    pub why: Why,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub format: &'static str,
    pub id: String,
    /// Every class, staged or not.
    pub classes: BTreeMap<Class, Tally>,
    /// The classes that go into the stage, together.
    pub staged: Tally,
    /// What stayed behind, largest first, at most [`LEFT_MAX`] lines.
    pub left: Vec<Left>,
    /// What the `left` lines past [`LEFT_MAX`] hold.
    pub left_more: Tally,
    /// Whether the SimpleX conversation was carried, and if not why
    /// (`stream` only: `sizes` reads no file to decide it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simplex: Option<Carry>,
}

/// The running totals of a walk.
#[derive(Default)]
pub struct Tallies {
    classes: BTreeMap<Class, Tally>,
    left: BTreeMap<(String, Class, Why), Tally>,
}

impl Tallies {
    pub fn staged(&mut self, class: Class, bytes: u64) {
        self.classes.entry(class).or_default().add(1, bytes);
    }

    pub fn left(&mut self, parts: &[&[u8]], verdict: Verdict, bytes: u64) {
        self.classes.entry(verdict.class).or_default().add(1, bytes);
        let prefix = parts[..verdict.depth.clamp(1, parts.len())]
            .iter()
            .map(|part| String::from_utf8_lossy(part))
            .collect::<Vec<_>>()
            .join("/");
        self.left
            .entry((prefix, verdict.class, verdict.why))
            .or_default()
            .add(1, bytes);
    }

    pub fn summary(self, id: &str) -> Summary {
        let mut classes = self.classes;
        for class in Class::ALL {
            classes.entry(class).or_default();
        }
        let mut staged = Tally::default();
        for (class, tally) in &classes {
            if class.staged() {
                staged.add(tally.files, tally.bytes);
            }
        }
        let mut left: Vec<Left> = self
            .left
            .into_iter()
            .map(|((path, class, why), tally)| Left {
                path,
                class,
                why,
                files: tally.files,
                bytes: tally.bytes,
            })
            .collect();
        left.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
        let mut left_more = Tally::default();
        for line in left.iter().skip(LEFT_MAX) {
            left_more.add(line.files, line.bytes);
        }
        left.truncate(LEFT_MAX);
        Summary {
            format: FORMAT,
            id: id.to_owned(),
            classes,
            staged,
            left,
            left_more,
            simplex: None,
        }
    }
}
