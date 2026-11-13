//! Identity descriptions for one report page. Read-only and response-only:
//! nothing updates identity aliases, Core state or any cache. The store
//! mutex is never held across these awaits.
//!
//! Legacy policy: Brain decides eligibility: only keys with recorded participation by that
//! exact key are sent to Core or shown any stored alias. Every other key may
//! show only a stored Identity Directory (`finite.vip`) name, the key's public
//! Directory binding: for a managed Agent its Agent name, for a person their
//! Finite VIP Email. It is shown whatever kind of key it is, by decision.
//! Brain asks Core v2, which applies no per-Brain sharing scope (FIN-166); an
//! older Core answers v1 and still enforces the account/Brain sharing scope.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use finite_brain_store::AccessReportSnapshot;

use super::contracts::*;
use crate::core_descriptions::scoped::{
    OperatorAnnotation, OperatorRecord, ScopedCoreDescription, ScopedLookupReport,
    ScopedLookupRequest, validate_scoped_response,
};
use crate::core_descriptions::{CORE_BATCH_KEYS, CoreDescription, CoreLookupFailure};
use crate::{ServerState, authority_concurrency};

const CORE_OVERALL_TIMEOUT: Duration = Duration::from_secs(4);

pub(crate) const NOT_PARTICIPATING: &str = "noParticipation";

/// The Identity Directory's NIP-05 domain. Its well-known document is the
/// binding authority for names under it. Any other domain's document can
/// claim any key, and any caller's lookup stores that claim.
const DIRECTORY_NIP05_DOMAIN: &str = "finite.vip";

fn directory_name(name: &str) -> bool {
    name.rsplit_once('@').is_some_and(|(local, domain)| {
        !local.is_empty() && domain.eq_ignore_ascii_case(DIRECTORY_NIP05_DOMAIN)
    })
}

pub(crate) struct PageDescriptions {
    pub(crate) coverage: DescriptionCoverage,
    pub(crate) rows: BTreeMap<String, DescribedRow>,
}

pub(crate) struct DescribedRow {
    pub(crate) description: IdentityDescriptionView,
    pub(crate) stored_alias: Option<StoredAliasView>,
    pub(crate) annotation: Option<OperatorAnnotation>,
    pub(crate) record: Option<OperatorRecord>,
}

/// Core receives all page keys in v3, but may describe a non-participating key
/// only through an exact scoped record. Brain alone supplies participation.
pub(crate) async fn describe_page(
    state: &ServerState,
    snapshot: &AccessReportSnapshot,
    rows: &[AccessIdentityRow],
    actor_hex: &str,
) -> PageDescriptions {
    let page = rows
        .iter()
        .filter(|row| !row.hex.is_empty())
        .map(|row| row.hex.clone())
        .collect::<Vec<_>>();
    let participating = rows
        .iter()
        .filter(|row| row.participation.is_some() && !row.hex.is_empty())
        .map(|row| row.hex.clone())
        .collect::<Vec<_>>();
    let (coverage, by_hex, legacy) =
        core_descriptions(state, snapshot, &page, &participating, actor_hex).await;
    let asked = participating
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut described = BTreeMap::new();
    for row in rows {
        let non_participating = row.participation.is_none() || row.hex.is_empty();
        let stored_alias = snapshot
            .aliases
            .iter()
            .find(|(npub, _)| npub.as_str() == row.npub)
            .and_then(|(_, alias)| {
                alias.preferred_nip05.clone().map(|name| StoredAliasView {
                    name,
                    stored_at: alias.nip05_verified_at.clone(),
                })
            })
            .filter(|alias| !non_participating || directory_name(&alias.name));
        let core = by_hex.get(&row.hex);
        let mut description = if non_participating && (legacy || row.hex.is_empty()) {
            IdentityDescriptionView::withheld(NOT_PARTICIPATING)
        } else if let Some(core) = core {
            view(&core.legacy(), &asked)
        } else {
            IdentityDescriptionView::bare(
                "unavailable",
                Some(
                    coverage
                        .reason
                        .as_deref()
                        .unwrap_or("identity source unavailable"),
                ),
            )
        };
        if non_participating && description.state == "notShared" {
            description.reason = Some(NOT_PARTICIPATING.to_owned());
        }
        described.insert(
            row.npub.clone(),
            DescribedRow {
                description,
                stored_alias,
                annotation: core.and_then(|r| r.operator_annotation.clone()),
                record: core.and_then(|r| r.operator_record.clone()),
            },
        );
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

/// A partial page is never trusted. All attempts share the existing permit
/// and bounded outer wait; each HTTP chunk has its own three-attempt deadline.
async fn core_descriptions(
    state: &ServerState,
    snapshot: &AccessReportSnapshot,
    page: &[String],
    participating: &[String],
    actor_hex: &str,
) -> (
    DescriptionCoverage,
    BTreeMap<String, ScopedCoreDescription>,
    bool,
) {
    let mut by_hex = BTreeMap::new();
    if page.is_empty() {
        return (
            coverage(
                "notNeeded",
                Some("this page has no canonical keys"),
                None,
                0,
            ),
            by_hex,
            true,
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
            true,
        );
    };
    let mut checked_at = None;
    let mut protocol = None;
    let mut negotiation = None;
    let mut all_legacy = true;
    for chunk in page.chunks(CORE_BATCH_KEYS) {
        let request = ScopedLookupRequest {
            brain_server: client.brain_server.clone(),
            brain_id: snapshot.authority.brain.id.to_string(),
            requested_by_hex: actor_hex.to_owned(),
            keys: chunk.to_vec(),
            participating_keys: chunk
                .iter()
                .filter(|key| participating.contains(key))
                .cloned()
                .collect(),
        };
        let lookup = client.lookup.clone();
        let failure = |reason: &str| ScopedLookupReport {
            result: Err(CoreLookupFailure::Unavailable(reason.to_owned())),
            legacy: false,
            negotiation: "direct",
        };
        let report = tokio::time::timeout(CORE_OVERALL_TIMEOUT, async move {
            let permit = authority_concurrency()
                .acquire()
                .await
                .map_err(|_| "worker")?;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let mut report = lookup(&request);
                if !report.legacy {
                    report.result = report.result.and_then(|r| {
                        r.map(|response| validate_scoped_response(&request, response))
                            .transpose()
                    });
                }
                report
            })
            .await
            .map_err(|_| "worker")
        })
        .await
        .map(|r| r.unwrap_or_else(failure))
        .unwrap_or_else(|_| failure("timeout"));
        all_legacy &= report.legacy;
        negotiation.get_or_insert(report.negotiation.to_owned());
        match report.result {
            Ok(Some(response)) => {
                if protocol.as_ref().is_some_and(|v| v != &response.version)
                    || negotiation.as_deref() != Some(report.negotiation)
                {
                    let mut failed = coverage(
                        "unavailable",
                        Some("Core protocol changed within this page"),
                        None,
                        0,
                    );
                    failed.core_negotiation = negotiation;
                    return (failed, BTreeMap::new(), false);
                }
                protocol.get_or_insert(response.version);
                checked_at.get_or_insert(response.checked_at);
                for row in response.results {
                    by_hex.insert(row.public_key_hex.clone(), row);
                }
            }
            Ok(None) => {} // Empty subset: do not issue a legacy request.
            Err(error) => {
                let mut failed = coverage(error.state(), Some(&error.reason()), None, 0);
                failed.core_negotiation = negotiation;
                return (failed, BTreeMap::new(), all_legacy);
            }
        }
    }
    let mut result = if by_hex.is_empty() && all_legacy {
        coverage(
            "notNeeded",
            Some("no participating keys for the legacy Core"),
            None,
            0,
        )
    } else {
        coverage("checked", None, checked_at, by_hex.len())
    };
    result.core_protocol_version = protocol;
    result.core_negotiation = negotiation;
    (result, by_hex, all_legacy)
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
        core_protocol_version: None,
        core_negotiation: None,
    }
}

#[cfg(test)]
mod tests {
    use super::directory_name;

    #[test]
    fn only_exact_directory_domain_names_count_as_directory_names() {
        for name in ["ada@finite.vip", "ada-1a2b@FINITE.VIP", "_@finite.vip"] {
            assert!(directory_name(name), "{name}");
        }
        for name in [
            "ada@evil.example",
            "ada@sub.finite.vip",
            "ada@finite.vip.evil.example",
            "ada@notfinite.vip",
            "@finite.vip",
            "finite.vip",
            "",
        ] {
            assert!(!directory_name(name), "{name}");
        }
    }
}
