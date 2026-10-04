//! Named Brain access report: `GET /v1/brains/{brain_id}/access-report`.
//!
//! A separate route because `/v1/brains/{brain_id}/access` is an existing
//! alias of the metadata response that older clients still consume.
//!
//! Authorization is the canonical key-based Brain admin policy
//! (`ensure_brain_admin_key`): the calling key must itself be the Personal
//! Brain owner, that Brain's delegated Personal Agent, or a Brain admin.
//! Nothing falls back to an account, and an Organization bot without the
//! admin role is denied before any name lookup. Authority is checked before
//! the snapshot, on it, and again (off the async runtime) after lookups; the
//! report is rebuilt if the authority fingerprint changed meanwhile, and a
//! page cursor from an older authority is refused with 409.

use axum::Json;
use axum::extract::{OriginalUri, Path as AxumPath, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use finite_brain_core::{BrainId, BrainKind, UserId};
use serde::Deserialize;

use crate::{
    ApiError, ServerState, ensure_brain_admin_key, lock_error, server_timestamp, spawn_store,
    validate_request_auth,
};

mod audit;
mod contracts;
mod cursor;
mod names;
mod rows;

pub(crate) use contracts::*;
use rows::{DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT, PageRequest};

/// Snapshot attempts before reporting that the Brain kept changing.
const MAX_REPORT_ATTEMPTS: usize = 2;

const AUTHORITY_CHANGED: &str =
    "Brain access changed while the report was being built; restart the report from the first page";

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccessReportQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    after: Option<String>,
}

fn page_request(query: AccessReportQuery) -> Result<PageRequest, ApiError> {
    let limit = query.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    if limit == 0 || limit > MAX_PAGE_LIMIT {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("limit must be between 1 and {MAX_PAGE_LIMIT}"),
        ));
    }
    let (after, fingerprint) = match query.after.filter(|value| !value.is_empty()) {
        None => (None, None),
        Some(value) => {
            let (fingerprint, npub) = cursor::decode(&value).ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "after must be the cursor from a previous page of this report",
                )
            })?;
            (Some(npub), Some(fingerprint))
        }
    };
    Ok(PageRequest {
        limit,
        after,
        fingerprint,
    })
}

fn authority_changed() -> ApiError {
    ApiError::new(StatusCode::CONFLICT, AUTHORITY_CHANGED)
}

pub(crate) async fn access_report_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    Query(query): Query<AccessReportQuery>,
) -> Result<Json<AccessReportResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, None)?;
    let actor_id = UserId::new(actor.clone())?;
    let brain_id = BrainId::new(brain_id)?;
    let page = page_request(query)?;

    for _ in 0..MAX_REPORT_ATTEMPTS {
        let (snapshot, report_rows) = {
            let brain_id = brain_id.clone();
            let actor = actor.clone();
            let after = page.after.clone();
            let limit = page.limit;
            let page_for_rows = page.clone();
            spawn_store(state.clone(), move |state| {
                let store = state.store.lock().map_err(lock_error)?;
                // Deny from roles alone before reading the snapshot.
                let (brain, personal_agent) = store.load_brain_roles(&brain_id)?;
                ensure_brain_admin_key(&brain, personal_agent.as_ref(), &actor)?;
                let snapshot = store.access_report_snapshot(&brain_id, after.as_ref(), limit)?;
                let authority = &snapshot.authority;
                ensure_brain_admin_key(
                    &authority.brain,
                    authority.personal_agent.as_ref(),
                    &actor,
                )?;
                // Release SQLite before bounded CPU work. Row construction and
                // signature checks stay on this blocking worker, never the async runtime.
                drop(store);
                let report_rows = rows::build_rows(&snapshot, &page_for_rows);
                Ok((snapshot, report_rows))
            })
            .await?
        };
        if page
            .fingerprint
            .as_ref()
            .is_some_and(|issued| *issued != snapshot.authority.fingerprint)
        {
            // A later page must come from the authority its cursor was issued under.
            return Err(authority_changed());
        }

        let checked_at = server_timestamp(&state);
        let page_names =
            names::resolve_page_names(&state, &snapshot, &report_rows.rows, &checked_at).await;

        // Recheck after external lookups, off the async runtime: the caller
        // must still be an admin and the authority must be unchanged.
        let current_fingerprint = {
            let brain_id = brain_id.clone();
            let actor = actor.clone();
            spawn_store(state.clone(), move |state| {
                let store = state.store.lock().map_err(lock_error)?;
                let authority = store.access_report_authority(&brain_id)?;
                ensure_brain_admin_key(
                    &authority.brain,
                    authority.personal_agent.as_ref(),
                    &actor,
                )?;
                Ok(authority.fingerprint)
            })
            .await?
        };
        if current_fingerprint != snapshot.authority.fingerprint {
            if page.fingerprint.is_some() {
                return Err(authority_changed());
            }
            continue;
        }

        let mut identities = report_rows.rows;
        for row in &mut identities {
            if let Some((name, identity_type)) = page_names.evidence.get(&row.npub) {
                row.name = name.clone();
                row.identity_type = identity_type.clone();
            }
        }
        let authority = &snapshot.authority;
        let coverage = rows::coverage(authority);
        let brain = &authority.brain;
        return Ok(Json(AccessReportResponse {
            version: ACCESS_REPORT_VERSION.to_owned(),
            brain_id: brain.id.to_string(),
            brain_kind: match brain.kind {
                BrainKind::Personal => "personal",
                BrainKind::Organization => "organization",
            }
            .to_owned(),
            brain_name: brain.name.to_string(),
            authority_sequence: authority.latest_sequence,
            authority_fingerprint: authority.fingerprint.clone(),
            checked_at,
            acting_key: ActingKey {
                npub: actor.clone(),
                brain_role: rows::acting_role(authority, &actor_id),
            },
            current_access_complete: rows::current_access_complete(&coverage),
            coverage,
            directory: page_names.directory,
            totals: report_rows.totals,
            incoming_mounts: rows::incoming_mount_views(authority),
            page: AccessPage {
                limit: page.limit,
                after: page
                    .after
                    .as_ref()
                    .map(|after| cursor::encode(&authority.fingerprint, after.as_str())),
                next: report_rows.next,
                returned: identities.len(),
            },
            identities,
        }));
    }
    Err(authority_changed())
}

#[cfg(test)]
pub(crate) fn entitlement_matches_folder_visible(
    authority: &finite_brain_store::AccessReportAuthority,
) -> bool {
    let stored = finite_brain_store::StoredBrain {
        brain: authority.brain.clone(),
        personal_agent: authority.personal_agent.clone(),
        folder_access: authority.folder_access.clone(),
        grants: Vec::new(),
        setup_incomplete_folder_ids: Default::default(),
        folder_deletion_audience: Default::default(),
    };
    let index = rows::AuthorityIndex::new(authority);
    authority.candidate_keys().iter().all(|key| {
        authority.brain.folders.iter().all(|folder| {
            index.entitlement_sources(folder, key).is_empty()
                != crate::folder_visible(&stored, &folder.id, key.as_str())
        })
    })
}

#[cfg(test)]
pub(crate) use audit::{SignedAudit, signed_grant_audit};
