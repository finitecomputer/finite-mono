//! Identity descriptions for one report page. Read-only and response-only:
//! nothing updates identity aliases, Core state or any cache. The store
//! mutex is never held across these awaits.
//!
//! Brain decides eligibility: only keys with recorded participation by that
//! exact key are sent to Core or shown a stored alias. Brain asks Core v2,
//! which applies no per-Brain sharing scope (FIN-166); an older Core answers
//! v1 and still enforces the account/Brain sharing scope.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use finite_brain_store::AccessReportSnapshot;

use super::contracts::*;
use crate::core_descriptions::{
    CORE_BATCH_KEYS, CoreDescription, CoreLookupFailure, CoreLookupRequest, validate_core_response,
};
use crate::{ServerState, authority_concurrency};

const CORE_OVERALL_TIMEOUT: Duration = Duration::from_secs(4);

pub(crate) const NOT_PARTICIPATING: &str = "noParticipation";

pub(crate) struct PageDescriptions {
    pub(crate) coverage: DescriptionCoverage,
    pub(crate) rows: BTreeMap<String, (IdentityDescriptionView, Option<StoredAliasView>)>,
}

/// Describe the page's rows. Keys without recorded participation get
/// `notShared`/`noParticipation` and no alias, whatever Core holds.
pub(crate) async fn describe_page(
    state: &ServerState,
    snapshot: &AccessReportSnapshot,
    rows: &[AccessIdentityRow],
    actor_hex: &str,
) -> PageDescriptions {
    let eligible = rows
        .iter()
        .filter(|row| row.participation.is_some() && !row.hex.is_empty())
        .map(|row| row.hex.clone())
        .collect::<Vec<_>>();
    let (coverage, by_hex) = core_descriptions(state, snapshot, &eligible, actor_hex).await;
    let asked = eligible.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let mut described = BTreeMap::new();
    for row in rows {
        if row.participation.is_none() || row.hex.is_empty() {
            described.insert(
                row.npub.clone(),
                (IdentityDescriptionView::withheld(NOT_PARTICIPATING), None),
            );
            continue;
        }
        let stored_alias = snapshot
            .aliases
            .iter()
            .find(|(npub, _)| npub.as_str() == row.npub)
            .and_then(|(_, alias)| {
                alias.preferred_nip05.clone().map(|name| StoredAliasView {
                    name,
                    stored_at: alias.nip05_verified_at.clone(),
                })
            });
        let description = match by_hex.get(&row.hex) {
            Some(core) => view(core, &asked),
            None => IdentityDescriptionView::bare(
                "unavailable",
                Some(
                    coverage
                        .reason
                        .as_deref()
                        .unwrap_or("identity source unavailable"),
                ),
            ),
        };
        described.insert(row.npub.clone(), (description, stored_alias));
    }
    PageDescriptions {
        coverage,
        rows: described,
    }
}

/// `asked` is the page's eligible keys. An owner's human keys are shown only
/// when they are among them: without a sharing scope, a key outside the
/// request would link the owner across Brains (older Core does not filter).
fn view(core: &CoreDescription, asked: &BTreeSet<&str>) -> IdentityDescriptionView {
    IdentityDescriptionView {
        state: core.state.clone(),
        reason: None,
        kind: core.kind.clone(),
        display_name: core.display_name.clone(),
        account_email: core.account_email.clone(),
        lifecycle: core.lifecycle.clone(),
        responsible_account: core.responsible_account.as_ref().map(|account| {
            ResponsibleAccountView {
                email: account.email.clone(),
                source: account.source.clone(),
                observed_at: account.observed_at.clone(),
                human_public_keys_hex: account
                    .human_public_keys_hex
                    .iter()
                    .filter(|key| asked.contains(key.as_str()))
                    .cloned()
                    .collect(),
            }
        }),
        source: core.source.as_ref().map(|source| DescriptionSourceView {
            kind: source.kind.clone(),
            observed_at: source.observed_at.clone(),
            revision: source.revision.clone(),
        }),
    }
}

/// Query Core for the eligible page keys. Any failure leaves every access
/// row intact and marks descriptions unavailable, never `unknown`.
async fn core_descriptions(
    state: &ServerState,
    snapshot: &AccessReportSnapshot,
    eligible: &[String],
    actor_hex: &str,
) -> (DescriptionCoverage, BTreeMap<String, CoreDescription>) {
    let mut by_hex = BTreeMap::new();
    if eligible.is_empty() {
        return (
            coverage(
                "notNeeded",
                Some("no key on this page has recorded participation by that exact key"),
                None,
                0,
            ),
            by_hex,
        );
    }
    let Some(client) = state.core_descriptions.clone() else {
        return (
            coverage(
                "notConfigured",
                Some("this Brain server has no Core identity descriptions configured"),
                None,
                0,
            ),
            by_hex,
        );
    };
    let mut checked_at = None;
    for chunk in eligible.chunks(CORE_BATCH_KEYS) {
        let request = CoreLookupRequest {
            brain_server: client.brain_server.clone(),
            brain_id: snapshot.authority.brain.id.to_string(),
            requested_by_hex: actor_hex.to_owned(),
            keys: chunk.to_vec(),
        };
        let lookup = client.lookup.clone();
        let result = tokio::time::timeout(CORE_OVERALL_TIMEOUT, async move {
            let permit = authority_concurrency()
                .acquire()
                .await
                .map_err(|_| CoreLookupFailure::Unavailable("worker".to_owned()))?;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                lookup(&request).and_then(|response| validate_core_response(&request, response))
            })
            .await
            .map_err(|_| CoreLookupFailure::Unavailable("worker".to_owned()))?
        })
        .await
        .unwrap_or_else(|_| Err(CoreLookupFailure::Unavailable("timeout".to_owned())));
        match result {
            Ok(response) => {
                checked_at.get_or_insert(response.checked_at);
                for row in response.results {
                    by_hex.insert(row.public_key_hex.clone(), row);
                }
            }
            Err(failure) => {
                // A partial page is not trusted: drop what earlier chunks said.
                return (
                    coverage(failure.state(), Some(&failure.reason()), None, 0),
                    BTreeMap::new(),
                );
            }
        }
    }
    let checked = by_hex.len();
    (coverage("checked", None, checked_at, checked), by_hex)
}

fn coverage(
    state: &str,
    reason: Option<&str>,
    checked_at: Option<String>,
    checked_keys: usize,
) -> DescriptionCoverage {
    DescriptionCoverage {
        state: state.to_owned(),
        reason: reason.map(ToOwned::to_owned),
        checked_at,
        checked_keys,
    }
}
