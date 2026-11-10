//! Each live old agent's sizes, measured once a day (Paul, 11-09) by the
//! host reader's `sizes`, which reads metadata alone, and kept in a cache
//! the roster reads. An agent not measured yet is listed with zeros.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::contract::Sizes;
use crate::fleet::Binding;
use crate::hosts::HostCommands;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizesCache {
    pub measured_ms: i64,
    pub agents: BTreeMap<String, Sizes>,
}

impl SizesCache {
    /// The cache, or an empty one if it isn't there or can't be read.
    pub fn read(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Write it whole, replacing the last by a rename.
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        let fresh = path.with_extension("json.new");
        std::fs::write(&fresh, serde_json::to_vec(self).expect("sizes serialize"))?;
        std::fs::rename(fresh, path)
    }
}

/// Measure every live agent through its host's reader. An agent whose host
/// can't be read keeps its last sizes (with why); one no longer live is
/// dropped.
pub fn measure(
    live: &[Binding],
    hosts: &HostCommands,
    last: &SizesCache,
    measured_ms: i64,
) -> (SizesCache, Vec<(String, String)>) {
    let mut fresh = SizesCache {
        measured_ms,
        agents: BTreeMap::new(),
    };
    let mut failed = Vec::new();
    for binding in live {
        match hosts.sizes(binding) {
            Ok(sizes) => {
                fresh.agents.insert(binding.old.clone(), sizes);
            }
            Err(error) => {
                if let Some(sizes) = last.agents.get(&binding.old) {
                    fresh.agents.insert(binding.old.clone(), *sizes);
                }
                failed.push((binding.old.clone(), error.to_string()));
            }
        }
    }
    (fresh, failed)
}

/// The host reader's `sizes` summary, as the roster counts it: its
/// classes' bytes, and what is staged.
pub fn sizes_of(summary: &serde_json::Value) -> Option<Sizes> {
    let classes = summary.get("classes")?.as_object()?;
    let bytes = |class: &str| {
        classes
            .get(class)
            .and_then(|tally| tally.get("bytes")?.as_u64())
            .unwrap_or(0)
    };
    let total = classes
        .values()
        .map(|tally| {
            tally
                .get("bytes")
                .and_then(|bytes| bytes.as_u64())
                .unwrap_or(0)
        })
        .sum();
    Some(Sizes {
        cache: bytes("cache"),
        history: bytes("history"),
        work: bytes("work"),
        staged: summary.get("staged")?.get("bytes")?.as_u64()?,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_counts_as_the_roster_lists_it() {
        let summary = serde_json::json!({
            "classes": {
                "secret": {"files": 1, "bytes": 10}, "config": {"files": 1, "bytes": 0},
                "cache": {"files": 2, "bytes": 200}, "runtime": {"files": 1, "bytes": 5},
                "history": {"files": 3, "bytes": 300}, "work": {"files": 4, "bytes": 4000},
            },
            "staged": {"files": 7, "bytes": 4300},
        });
        assert_eq!(
            sizes_of(&summary),
            Some(Sizes {
                cache: 200,
                history: 300,
                work: 4000,
                staged: 4300,
                total: 4515
            })
        );
        assert_eq!(
            sizes_of(&serde_json::json!({"classes": {}})),
            None,
            "no staged tally"
        );
    }
}
