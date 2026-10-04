//! Name evidence for one report page. Every lookup here is read-only and
//! response-only: nothing updates identity aliases or adds a cache. The
//! store mutex is never held across these awaits.

use std::collections::BTreeMap;
use std::time::Duration;

use axum::http::StatusCode;
use finite_brain_store::{AccessReportSnapshot, IdentityAlias};

use super::contracts::*;
use crate::directory_names::{
    DIRECTORY_BATCH_KEYS, DirectoryKeyNames, DirectoryLookupFailure, DirectoryLookupResponse,
    validate_directory_response,
};
use crate::{
    Nip05Identifier, ServerState, authority_concurrency, format_unix_timestamp,
    resolve_identity_input,
};

/// Forward NIP-05 rechecks of stored aliases per page.
pub(crate) const MAX_FORWARD_CHECKS_PER_PAGE: usize = 4;
const DIRECTORY_OVERALL_TIMEOUT: Duration = Duration::from_secs(5);
const FORWARD_CHECKS_OVERALL_TIMEOUT: Duration = Duration::from_secs(6);

pub(crate) struct PageNames {
    pub(crate) directory: DirectoryCoverage,
    pub(crate) evidence: BTreeMap<String, (NameEvidence, IdentityTypeEvidence)>,
}

enum ForwardCheck {
    Matched,
    Mismatched,
    NotRegistered,
    Invalid,
    Unavailable,
}

const NOT_PARTICIPATING: &str =
    "no recorded participation by this exact key; stored and Directory names were not consulted";

/// Resolve names for the page's rows. Only keys with recorded participation
/// by that exact key get any name: they alone are sent to the Directory, and
/// they alone may show or recheck a stored alias. Aliases are server-global
/// (any caller's lookup may have written one), so showing one for an
/// admin-added key would turn adding a key into reverse name discovery.
pub(crate) async fn resolve_page_names(
    state: &ServerState,
    snapshot: &AccessReportSnapshot,
    rows: &[AccessIdentityRow],
    checked_at: &str,
) -> PageNames {
    let eligible = rows
        .iter()
        .filter(|row| row.participation.is_some() && !row.hex.is_empty())
        .map(|row| row.hex.clone())
        .collect::<Vec<_>>();
    let (directory, by_hex) = directory_names(state, &eligible).await;

    let mut evidence = BTreeMap::new();
    let mut forward = Vec::new();
    for row in rows {
        if row.participation.is_none() {
            evidence.insert(
                row.npub.clone(),
                (
                    NameEvidence::unknown(NOT_PARTICIPATING),
                    IdentityTypeEvidence::not_confirmed(NOT_PARTICIPATING),
                ),
            );
            continue;
        }
        let alias = snapshot
            .aliases
            .iter()
            .find(|(npub, _)| npub.as_str() == row.npub)
            .map(|(_, alias)| alias);
        let directory_result = by_hex.get(&row.hex);
        if let Some(found) =
            directory_result.filter(|result| result.status == "found" && !result.names.is_empty())
        {
            evidence.insert(
                row.npub.clone(),
                directory_evidence(row, found, alias, &directory, checked_at),
            );
            continue;
        }
        let directory_note = if directory_result.is_some() {
            "Directory has no active name for this exact key"
        } else {
            "Directory name lookup unavailable for this key"
        };
        match alias.and_then(|alias| alias.preferred_nip05.clone()) {
            Some(name) if forward.len() < MAX_FORWARD_CHECKS_PER_PAGE => {
                forward.push((row.npub.clone(), name));
            }
            Some(_) => {
                evidence.insert(
                    row.npub.clone(),
                    (
                        stored_not_rechecked(alias, "recheck budget for this page was used"),
                        IdentityTypeEvidence::not_confirmed(directory_note),
                    ),
                );
            }
            None => {
                evidence.insert(
                    row.npub.clone(),
                    (
                        NameEvidence::unknown("no stored or Directory name for this exact key"),
                        IdentityTypeEvidence::not_confirmed(directory_note),
                    ),
                );
            }
        }
    }

    let outcomes = forward_checks(state, &forward).await;
    for (npub, name) in forward {
        let alias = snapshot
            .aliases
            .iter()
            .find(|(key, _)| key.as_str() == npub)
            .map(|(_, alias)| alias);
        let row = rows.iter().find(|row| row.npub == npub);
        let type_note = "no managed-agent binding confirmed for this exact key";
        let name_evidence = match outcomes.get(&npub) {
            Some(ForwardCheck::Matched) => NameEvidence {
                state: "domainClaimed".to_owned(),
                display: format!("Domain claim: {name}"),
                label: Some(name.clone()),
                source: Some("nip05Forward".to_owned()),
                kind: None,
                matched_key: row.map(|row| row.hex.clone()),
                checked_at: Some(checked_at.to_owned()),
                stored_verified_at: alias.and_then(|alias| alias.nip05_verified_at.clone()),
                additional_names: Vec::new(),
                more_names: false,
                reason: Some("The domain publishes this name-to-key claim; the key holder has not confirmed the label.".to_owned()),
            },
            Some(ForwardCheck::Mismatched) => {
                NameEvidence::unknown("the stored name now resolves to a different key")
            }
            Some(ForwardCheck::NotRegistered) => {
                NameEvidence::unknown("the stored name is no longer registered")
            }
            Some(ForwardCheck::Invalid) => {
                NameEvidence::unknown("the stored name is not a checkable NIP-05 name")
            }
            Some(ForwardCheck::Unavailable) | None => {
                stored_not_rechecked(alias, "name recheck was unavailable")
            }
        };
        evidence.insert(
            npub,
            (
                name_evidence,
                IdentityTypeEvidence::not_confirmed(type_note),
            ),
        );
    }
    PageNames {
        directory,
        evidence,
    }
}

fn directory_evidence(
    row: &AccessIdentityRow,
    found: &DirectoryKeyNames,
    alias: Option<&IdentityAlias>,
    directory: &DirectoryCoverage,
    checked_at: &str,
) -> (NameEvidence, IdentityTypeEvidence) {
    let preferred = alias.and_then(|alias| alias.preferred_nip05.as_deref());
    let primary = found
        .names
        .iter()
        .find(|name| Some(name.name.as_str()) == preferred)
        .unwrap_or(&found.names[0]);
    let checked = directory.checked_at.as_deref().unwrap_or(checked_at);
    let name = NameEvidence {
        state: "verified".to_owned(),
        display: primary.name.clone(),
        label: Some(primary.name.clone()),
        source: Some(format!(
            "identityDirectory:{}",
            primary.source.as_deref().unwrap_or("binding")
        )),
        kind: Some(primary.kind.clone()),
        matched_key: Some(row.hex.clone()),
        checked_at: Some(checked.to_owned()),
        stored_verified_at: primary.bound_at.and_then(format_unix_timestamp),
        additional_names: found
            .names
            .iter()
            .filter(|name| name.name != primary.name)
            .map(|name| name.name.clone())
            .collect(),
        more_names: found.more_names,
        reason: None,
    };
    let managed = found.names.iter().find(|name| name.kind == "managed_agent");
    let identity_type = match managed {
        Some(binding) => IdentityTypeEvidence::managed_agent(format!(
            "Identity Directory managed-agent binding {} for this exact key",
            binding.name
        )),
        None => IdentityTypeEvidence::not_confirmed(
            "Directory binding is a mailbox name; no managed-agent binding for this exact key",
        ),
    };
    (name, identity_type)
}

fn stored_not_rechecked(alias: Option<&IdentityAlias>, reason: &str) -> NameEvidence {
    let Some(label) = alias.and_then(|alias| alias.preferred_nip05.clone()) else {
        return NameEvidence::unknown(reason);
    };
    if Nip05Identifier::parse(&label).is_err() {
        return NameEvidence::unknown("stored label is not a valid NIP-05 name");
    }
    NameEvidence {
        state: "storedNotRechecked".to_owned(),
        display: format!("Stored name, not rechecked: {label}"),
        label: Some(label),
        source: Some("storedAlias".to_owned()),
        kind: None,
        matched_key: None,
        checked_at: None,
        stored_verified_at: alias.and_then(|alias| alias.nip05_verified_at.clone()),
        additional_names: Vec::new(),
        more_names: false,
        reason: Some(format!(
            "{reason}; stored domain claim, not a label confirmed by the key holder"
        )),
    }
}

/// Query the Directory in bounded batches. The first failure stops further
/// batches; affected keys fall back to stored-name evidence.
async fn directory_names(
    state: &ServerState,
    eligible: &[String],
) -> (DirectoryCoverage, BTreeMap<String, DirectoryKeyNames>) {
    let mut by_hex = BTreeMap::new();
    if eligible.is_empty() {
        return (
            directory_coverage(
                "notNeeded",
                Some("no key on this page has recorded participation by that exact key".to_owned()),
                0,
            ),
            by_hex,
        );
    }
    let Some(lookup) = state.directory_names.clone() else {
        return (
            directory_coverage(
                "notConfigured",
                Some(
                    "this Brain server has no Identity Directory name lookup configured".to_owned(),
                ),
                0,
            ),
            by_hex,
        );
    };
    let mut checked_at = None;
    for chunk in eligible.chunks(DIRECTORY_BATCH_KEYS) {
        let keys = chunk.to_vec();
        let lookup = lookup.clone();
        let result = tokio::time::timeout(DIRECTORY_OVERALL_TIMEOUT, async move {
            let permit = authority_concurrency()
                .acquire()
                .await
                .map_err(|_| DirectoryLookupFailure::Unavailable("worker".to_owned()))?;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                lookup(keys.as_slice())
                    .and_then(|response| validate_directory_response(&keys, response))
            })
            .await
            .map_err(|_| DirectoryLookupFailure::Unavailable("worker".to_owned()))?
        })
        .await
        .unwrap_or_else(|_| Err(DirectoryLookupFailure::Unavailable("timeout".to_owned())));
        match result {
            Ok(DirectoryLookupResponse {
                checked_at: at,
                results,
            }) => {
                checked_at = checked_at.or(at);
                for result in results {
                    by_hex.insert(result.pubkey.to_ascii_lowercase(), result);
                }
            }
            Err(failure) => {
                return (
                    directory_coverage(failure.state(), Some(failure.reason()), by_hex.len()),
                    by_hex,
                );
            }
        }
    }
    let mut coverage = directory_coverage("checked", None, by_hex.len());
    coverage.checked_at = checked_at.and_then(format_unix_timestamp);
    (coverage, by_hex)
}

fn directory_coverage(
    state: &str,
    reason: Option<String>,
    checked_keys: usize,
) -> DirectoryCoverage {
    DirectoryCoverage {
        state: state.to_owned(),
        reason,
        checked_at: None,
        checked_keys,
    }
}

/// Forward-check stored names with the pure resolver: name -> key, accepted
/// only on an exact key match. Results are never recorded.
async fn forward_checks(
    state: &ServerState,
    names: &[(String, String)],
) -> BTreeMap<String, ForwardCheck> {
    let mut tasks = tokio::task::JoinSet::new();
    for (npub, name) in names {
        let state = state.clone();
        let npub = npub.clone();
        let name = name.clone();
        tasks.spawn(async move {
            let outcome = match resolve_identity_input(&state, &name).await {
                Ok(resolved) if resolved.npub == npub => ForwardCheck::Matched,
                Ok(_) => ForwardCheck::Mismatched,
                Err(error) if error.status == StatusCode::NOT_FOUND => ForwardCheck::NotRegistered,
                Err(error) if error.status == StatusCode::BAD_REQUEST => ForwardCheck::Invalid,
                Err(_) => ForwardCheck::Unavailable,
            };
            (npub, outcome)
        });
    }
    let mut outcomes = BTreeMap::new();
    let _ = tokio::time::timeout(FORWARD_CHECKS_OVERALL_TIMEOUT, async {
        while let Some(joined) = tasks.join_next().await {
            if let Ok((npub, outcome)) = joined {
                outcomes.insert(npub, outcome);
            }
        }
    })
    .await;
    tasks.abort_all();
    outcomes
}
