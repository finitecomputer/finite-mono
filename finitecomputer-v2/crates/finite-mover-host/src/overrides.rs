//! The operator's overrides (FIN-499): paths, relative to the durable
//! tree, that `carry` stages although a rule would leave them as runtime,
//! hidden, an earlier export or cache, and that `leave` leaves although a
//! rule would stage them. The mover reads them from its overrides file on
//! each run and names each, its UTF-8 in hex, on `sizes` and `stream`
//! (`--carry`, `--leave`).
//!
//! A path covers itself and everything under it. The longest path that
//! covers an entry decides it; the same path in both is left. The secret
//! and link rules always win: a credential, a link (or a file with a
//! second name) or a name a zip can't carry is never staged, inside a
//! carried cache or build folder too. The files the stage writes itself (the config
//! copy, a brain's name file, SimpleX) are not walked, so neither applies.

use crate::classify::{Class, EntryKind, Verdict, Why, uncarriable};
use crate::error::HostError;

/// The most overrides one operation may name.
pub const OVERRIDES_MAX: usize = 32;
/// The longest path, in bytes.
pub const OVERRIDE_PATH_MAX: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Override {
    Carry,
    Leave,
}

/// Each override, with its path's components.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Overrides(Vec<(Override, Vec<String>)>);

impl Overrides {
    /// Add the override whose path's UTF-8 is `hex_path`.
    pub fn push(&mut self, kind: Override, hex_path: &str) -> Result<(), HostError> {
        if self.0.len() >= OVERRIDES_MAX {
            return Err(HostError::Usage(format!(
                "at most {OVERRIDES_MAX} --carry and --leave"
            )));
        }
        let path = hex::decode(hex_path)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or_else(|| HostError::Usage("a path is the hex of its UTF-8".into()))?;
        self.0.push((kind, parse_path(&path)?));
        Ok(())
    }

    /// `verdict`, the rules' answer for `parts` (an entry of `kind`), as the
    /// overrides change it.
    pub fn apply(&self, parts: &[&[u8]], kind: EntryKind, verdict: Verdict) -> Verdict {
        let covering = self
            .0
            .iter()
            .filter(|(_, path)| {
                path.len() <= parts.len()
                    && path
                        .iter()
                        .zip(parts)
                        .all(|(part, name)| part.as_bytes() == *name)
            })
            .max_by_key(|(kind, path)| (path.len(), *kind == Override::Leave));
        match covering {
            Some((Override::Leave, path)) if verdict.class.staged() => Verdict {
                class: Class::Runtime,
                why: Why::LeftByOperator,
                depth: path.len(),
            },
            Some((Override::Carry, _)) if carriable(verdict.why) => {
                // The cache rules come before the link and name rules.
                if let Some(uncarriable) = uncarriable(parts, kind) {
                    return uncarriable;
                }
                let (class, why) = if parts[0] == b"agent" {
                    (Class::History, Why::History)
                } else {
                    (Class::Work, Why::Workspace)
                };
                Verdict {
                    class,
                    why,
                    depth: parts.len(),
                }
            }
            _ => verdict,
        }
    }
}

/// The rules `carry` overrides: never a secret's, a link's, a name a zip
/// can't carry, or a file the stage writes in its place.
fn carriable(why: Why) -> bool {
    matches!(
        why,
        Why::Runtime | Why::HiddenFolder | Why::EarlierExport | Why::CacheDir | Why::BuildOutput
    )
}

/// A path as an operator writes it: relative, `/` between its parts, none
/// of them empty, `.` or `..`, with no control character or backslash, at
/// most `OVERRIDE_PATH_MAX` bytes, and outside `.finite-stage/`, the
/// stage's own entries.
pub fn parse_path(path: &str) -> Result<Vec<String>, HostError> {
    let parts: Vec<String> = path.split('/').map(str::to_owned).collect();
    let valid = !path.is_empty()
        && path.len() <= OVERRIDE_PATH_MAX
        && !path.chars().any(|c| c.is_control() || c == '\\')
        && parts
            .iter()
            .all(|part| !["", ".", ".."].contains(&part.as_str()))
        && parts[0] != ".finite-stage";
    if !valid {
        return Err(HostError::Usage(format!(
            "an override is a relative path of at most {OVERRIDE_PATH_MAX} bytes, \
             with no empty, . or .. part, outside .finite-stage"
        )));
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{EntryKind, classify};

    fn overrides(entries: &[(Override, &str)]) -> Overrides {
        let mut overrides = Overrides::default();
        for (kind, path) in entries {
            overrides.push(*kind, &hex::encode(path)).unwrap();
        }
        overrides
    }

    fn class(overrides: &Overrides, path: &str) -> (Class, Why) {
        let parts: Vec<&[u8]> = path.split('/').map(str::as_bytes).collect();
        let verdict = overrides.apply(
            &parts,
            EntryKind::File,
            classify(&parts, EntryKind::File, None),
        );
        (verdict.class, verdict.why)
    }

    #[test]
    fn carry_stages_what_a_rule_leaves_and_leave_leaves_what_it_stages() {
        let set = overrides(&[
            (Override::Carry, "agent/hermes-home/sessions"),
            (Override::Carry, "workspace/.config/tool"),
            (Override::Carry, "workspace/app/node_modules"),
            (Override::Leave, "workspace/big"),
            (Override::Carry, "workspace/big/keep.md"),
            (Override::Leave, "workspace/app/node_modules/huge"),
            (Override::Carry, "agent/hermes-home/.env"),
            (Override::Carry, "workspace/keys"),
            (Override::Carry, "workspace/notes/today.md"),
            (Override::Leave, "workspace/notes/today.md"),
        ]);
        for (path, expected) in [
            (
                "agent/hermes-home/sessions/s.json",
                (Class::History, Why::History),
            ),
            (
                "workspace/.config/tool/settings.json",
                (Class::Work, Why::Workspace),
            ),
            (
                "workspace/app/node_modules/pkg/index.js",
                (Class::Work, Why::Workspace),
            ),
            (
                "workspace/big/photo.jpg",
                (Class::Runtime, Why::LeftByOperator),
            ),
            ("workspace/big/keep.md", (Class::Work, Why::Workspace)),
            (
                "workspace/app/node_modules/huge/x.bin",
                (Class::Cache, Why::CacheDir),
            ),
            // The same path in both is left.
            (
                "workspace/notes/today.md",
                (Class::Runtime, Why::LeftByOperator),
            ),
            // Secrets win over carry.
            ("agent/hermes-home/.env", (Class::Secret, Why::HermesEnv)),
            (
                "workspace/keys/id_rsa",
                (Class::Secret, Why::CredentialName),
            ),
            // Uncovered paths keep their rule.
            (
                "agent/hermes-home/logs/gateway.log",
                (Class::Runtime, Why::Runtime),
            ),
            ("workspace/bigger/a.md", (Class::Work, Why::Workspace)),
        ] {
            assert_eq!(class(&set, path), expected, "{path}");
        }
        // A link, or a name a zip can't carry, is never carried: neither
        // under a carried folder of the runtime nor of a cache.
        for (path, kind, why) in [
            (
                &[&b"agent"[..], b"hermes-home", b"sessions", b"link"][..],
                EntryKind::Other,
                Why::NotAFile,
            ),
            (
                &[b"workspace", b"app", b"node_modules", b".bin", b"tsc"],
                EntryKind::Other,
                Why::NotAFile,
            ),
            (
                &[b"workspace", b"app", b"node_modules", b"bad\xffname.js"],
                EntryKind::File,
                Why::UncarriedName,
            ),
            (
                &[b"workspace", b"app", b"node_modules", b"new\nline.js"],
                EntryKind::File,
                Why::UncarriedName,
            ),
        ] {
            let verdict = set.apply(path, kind, classify(path, kind, None));
            assert_eq!(
                (verdict.class, verdict.why),
                (Class::Runtime, why),
                "{path:?}"
            );
        }
        let output: Vec<&[u8]> = vec![b"workspace", b"app", b"dist", b"link.js"];
        let carried = overrides(&[(Override::Carry, "workspace/app/dist")]);
        let verdict = carried.apply(
            &output,
            EntryKind::Other,
            classify(&output, EntryKind::Other, Some(3)),
        );
        assert_eq!(
            verdict.why,
            Why::NotAFile,
            "nor under a carried build folder"
        );
    }

    #[test]
    fn paths_are_relative_bounded_and_outside_the_stages_own() {
        for good in [
            "workspace",
            "agent/hermes-home/sessions",
            "a b/c.md",
            "été/x",
        ] {
            assert!(parse_path(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "workspace/../agent",
            "./workspace",
            "workspace//x",
            "workspace/",
            "a\nb",
            "a\\b",
            ".finite-stage/summary.json",
            &"x".repeat(OVERRIDE_PATH_MAX + 1),
        ] {
            assert!(parse_path(bad).is_err(), "{bad:?}");
        }
        let mut set = Overrides::default();
        assert!(set.push(Override::Carry, "zz").is_err(), "not hex");
        assert!(
            set.push(Override::Carry, &hex::encode([0xff, 0xfe]))
                .is_err()
        );
        for _ in 0..OVERRIDES_MAX {
            set.push(Override::Leave, &hex::encode("workspace"))
                .unwrap();
        }
        assert!(
            set.push(Override::Leave, &hex::encode("workspace"))
                .is_err()
        );
    }
}
