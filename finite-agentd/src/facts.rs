//! Redacted Hermes facts from the helper's `inference-facts` subcommand
//! (§8.2.1), and the status cache in front of it.
//!
//! Every fact is a string enum that includes `unknown`; an unrecognized value
//! deserializes as `unknown`. Nothing here holds a secret.

use std::future::Future;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::AgentdError;

/// Facts are recomputed at least this often even when no file changed, so
/// time-dependent facts such as Codex cooldowns are never staler than this.
pub(crate) const FACTS_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct InferenceFacts {
    pub v: u32,
    pub saved_route: SavedRouteFact,
    pub fallback: FallbackFacts,
    pub finite_private: FinitePrivateFacts,
    pub openrouter: OpenRouterFacts,
    pub codex: CodexFacts,
    pub session_overrides: SessionOverrideFacts,
    pub alias_present: YesNo,
    pub codex_home_neutral: YesNo,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct FallbackFacts {
    pub fallback_providers: Tri,
    pub fallback_model: Tri,
    /// `None` means the effective chain is unknown.
    pub effective: Option<Vec<FallbackEntryFact>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct FallbackEntryFact {
    pub provider: String,
    pub model: String,
    pub owned_canonical: YesNo,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct FinitePrivateFacts {
    pub provider_entry: ProviderEntryFact,
    pub fp_key: Tri,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct OpenRouterFacts {
    pub hermes_key: Tri,
    pub hermes_key_fingerprint: Option<String>,
    pub dotenv_key: Tri,
    pub manual_pool_entries: PoolEntries,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct CodexFacts {
    pub state: CodexStateFact,
    /// Epoch seconds.
    pub quota_reset_at: Option<f64>,
    /// Epoch seconds.
    pub reported_quota_reset_at: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct SessionOverrideFacts {
    pub openrouter: Tri,
    pub openai_codex: Tri,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Tri {
    Present,
    Absent,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum YesNo {
    Yes,
    No,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SavedRouteFact {
    FinitePrivate,
    Openrouter,
    OpenaiCodex,
    Other,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderEntryFact {
    Canonical,
    Absent,
    Modified,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PoolEntries {
    None,
    Present,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CodexStateFact {
    NotSignedIn,
    SignedIn,
    QuotaLimited,
    SignInRequired,
    #[serde(other)]
    Unknown,
}

impl InferenceFacts {
    /// What status uses when the helper failed: every fact `unknown`, and an
    /// unknown effective fallback chain.
    pub(crate) fn unknown() -> Self {
        Self {
            v: 1,
            saved_route: SavedRouteFact::Unknown,
            fallback: FallbackFacts {
                fallback_providers: Tri::Unknown,
                fallback_model: Tri::Unknown,
                effective: None,
            },
            finite_private: FinitePrivateFacts {
                provider_entry: ProviderEntryFact::Unknown,
                fp_key: Tri::Unknown,
            },
            openrouter: OpenRouterFacts {
                hermes_key: Tri::Unknown,
                hermes_key_fingerprint: None,
                dotenv_key: Tri::Unknown,
                manual_pool_entries: PoolEntries::Unknown,
            },
            codex: CodexFacts {
                state: CodexStateFact::Unknown,
                quota_reset_at: None,
                reported_quota_reset_at: None,
            },
            session_overrides: SessionOverrideFacts {
                openrouter: Tri::Unknown,
                openai_codex: Tri::Unknown,
            },
            alias_present: YesNo::Unknown,
            codex_home_neutral: YesNo::Unknown,
        }
    }
}

/// `(mtime_ns, size)` of `config.yaml`, `.env`, and `auth.json`; `None` for a
/// missing or unreadable file.
type FileStamps = [Option<(i128, u64)>; 3];

struct CachedFacts {
    stamps: FileStamps,
    fetched_at: Instant,
    facts: InferenceFacts,
}

/// Caches helper facts keyed on the stats of the files they come from, with a
/// TTL. A failed fetch is not cached: status serves `unknown` facts and the
/// next call tries the helper again.
#[derive(Default)]
pub(crate) struct FactsCache {
    entry: Mutex<Option<CachedFacts>>,
}

impl FactsCache {
    #[cfg_attr(not(test), expect(dead_code, reason = "wired in A1c"))]
    pub(crate) async fn get_or_fetch<F, Fut>(&self, hermes_home: &Path, fetch: F) -> InferenceFacts
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<InferenceFacts, AgentdError>>,
    {
        self.get_or_fetch_at(hermes_home, Instant::now(), fetch)
            .await
    }

    async fn get_or_fetch_at<F, Fut>(
        &self,
        hermes_home: &Path,
        now: Instant,
        fetch: F,
    ) -> InferenceFacts
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<InferenceFacts, AgentdError>>,
    {
        let stamps = file_stamps(hermes_home);
        if let Some(facts) = self.lookup(&stamps, now) {
            return facts;
        }
        match fetch().await {
            Ok(facts) => {
                // Stamp with the stats taken before the fetch, so a write
                // racing the helper leaves a stale key and the next call refetches.
                *self.lock() = Some(CachedFacts {
                    stamps,
                    fetched_at: now,
                    facts: facts.clone(),
                });
                facts
            }
            Err(_) => InferenceFacts::unknown(),
        }
    }

    fn lookup(&self, stamps: &FileStamps, now: Instant) -> Option<InferenceFacts> {
        self.lock()
            .as_ref()
            .filter(|entry| {
                entry.stamps == *stamps
                    && now.saturating_duration_since(entry.fetched_at) < FACTS_TTL
            })
            .map(|entry| entry.facts.clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<CachedFacts>> {
        self.entry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn file_stamps(hermes_home: &Path) -> FileStamps {
    ["config.yaml", ".env", "auth.json"].map(|name| {
        std::fs::metadata(hermes_home.join(name))
            .ok()
            .map(|metadata| {
                (
                    i128::from(metadata.mtime()) * 1_000_000_000
                        + i128::from(metadata.mtime_nsec()),
                    metadata.size(),
                )
            })
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;

    use serde_json::json;

    use super::*;

    fn sample() -> serde_json::Value {
        json!({
            "v": 1,
            "saved_route": "openrouter",
            "fallback": {
                "fallback_providers": "present",
                "fallback_model": "absent",
                "effective": [
                    {"provider": "finite-private", "model": "glm-5-3-flash", "owned_canonical": "yes"},
                    {"provider": "anthropic", "model": "claude", "owned_canonical": "no"}
                ]
            },
            "finite_private": {"provider_entry": "canonical", "fp_key": "present"},
            "openrouter": {
                "hermes_key": "present",
                "hermes_key_fingerprint": "ab".repeat(32),
                "dotenv_key": "present",
                "manual_pool_entries": "none"
            },
            "codex": {"state": "quota_limited", "quota_reset_at": 1790000000.5, "reported_quota_reset_at": null},
            "session_overrides": {"openrouter": "absent", "openai_codex": "present"},
            "alias_present": "no",
            "codex_home_neutral": "yes"
        })
    }

    #[test]
    fn helper_output_parses_with_the_documented_names() {
        let facts: InferenceFacts = serde_json::from_value(sample()).unwrap();
        assert_eq!(facts.saved_route, SavedRouteFact::Openrouter);
        assert_eq!(facts.fallback.fallback_providers, Tri::Present);
        let effective = facts.fallback.effective.as_ref().unwrap();
        assert_eq!(effective[0].owned_canonical, YesNo::Yes);
        assert_eq!(effective[1].provider, "anthropic");
        assert_eq!(
            facts.finite_private.provider_entry,
            ProviderEntryFact::Canonical
        );
        assert_eq!(facts.openrouter.manual_pool_entries, PoolEntries::None);
        assert_eq!(facts.codex.state, CodexStateFact::QuotaLimited);
        assert_eq!(facts.codex.quota_reset_at, Some(1_790_000_000.5));
        assert_eq!(facts.session_overrides.openai_codex, Tri::Present);
        assert_eq!(facts.alias_present, YesNo::No);
        assert_eq!(facts.codex_home_neutral, YesNo::Yes);
    }

    #[test]
    fn unrecognized_enum_values_become_unknown() {
        let mut value = sample();
        value["saved_route"] = json!("anthropic");
        value["fallback"]["fallback_model"] = json!("maybe");
        value["fallback"]["effective"] = json!(null);
        value["finite_private"]["provider_entry"] = json!("rewritten");
        value["openrouter"]["manual_pool_entries"] = json!("three");
        value["codex"]["state"] = json!("frozen");
        value["alias_present"] = json!("perhaps");
        let facts: InferenceFacts = serde_json::from_value(value).unwrap();
        assert_eq!(facts.saved_route, SavedRouteFact::Unknown);
        assert_eq!(facts.fallback.fallback_model, Tri::Unknown);
        assert_eq!(facts.fallback.effective, None);
        assert_eq!(
            facts.finite_private.provider_entry,
            ProviderEntryFact::Unknown
        );
        assert_eq!(facts.openrouter.manual_pool_entries, PoolEntries::Unknown);
        assert_eq!(facts.codex.state, CodexStateFact::Unknown);
        assert_eq!(facts.alias_present, YesNo::Unknown);
    }

    fn fetched(label: &str) -> InferenceFacts {
        let mut facts = InferenceFacts::unknown();
        facts.openrouter.hermes_key_fingerprint = Some(label.to_owned());
        facts
    }

    fn label(facts: &InferenceFacts) -> Option<&str> {
        facts.openrouter.hermes_key_fingerprint.as_deref()
    }

    #[tokio::test]
    async fn the_cache_is_keyed_on_file_stats_and_expires_after_the_ttl() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        fs::write(home.join("config.yaml"), "model: {}\n").unwrap();
        let cache = FactsCache::default();
        let calls = Cell::new(0);
        let fetch = |name: &'static str| {
            calls.set(calls.get() + 1);
            async move { Ok(fetched(name)) }
        };
        let start = Instant::now();

        let first = cache.get_or_fetch_at(home, start, || fetch("one")).await;
        assert_eq!(label(&first), Some("one"));
        let hit = cache
            .get_or_fetch_at(home, start + Duration::from_secs(29), || fetch("two"))
            .await;
        assert_eq!(label(&hit), Some("one"));
        assert_eq!(calls.get(), 1);

        // No file changed, but the TTL passed.
        let expired = cache
            .get_or_fetch_at(home, start + FACTS_TTL, || fetch("three"))
            .await;
        assert_eq!(label(&expired), Some("three"));
        assert_eq!(calls.get(), 2);

        // A watched file appears.
        fs::write(home.join(".env"), "OPENROUTER_API_KEY=x\n").unwrap();
        let changed = cache
            .get_or_fetch_at(home, start + FACTS_TTL, || fetch("four"))
            .await;
        assert_eq!(label(&changed), Some("four"));
        assert_eq!(calls.get(), 3);

        // A watched file changes size.
        fs::write(home.join("auth.json"), "{}").unwrap();
        let changed = cache
            .get_or_fetch_at(home, start + FACTS_TTL, || fetch("five"))
            .await;
        assert_eq!(label(&changed), Some("five"));
        assert_eq!(calls.get(), 4);
    }

    #[tokio::test]
    async fn a_failed_fetch_serves_unknown_facts_and_is_not_cached() {
        let temp = tempfile::tempdir().unwrap();
        let cache = FactsCache::default();
        let now = Instant::now();
        let failed = cache
            .get_or_fetch_at(temp.path(), now, || async {
                Err(AgentdError::ProviderUnavailable("helper failed".to_owned()))
            })
            .await;
        assert_eq!(failed, InferenceFacts::unknown());
        let next = cache
            .get_or_fetch(temp.path(), || async { Ok(fetched("fresh")) })
            .await;
        assert_eq!(label(&next), Some("fresh"));
    }
}
