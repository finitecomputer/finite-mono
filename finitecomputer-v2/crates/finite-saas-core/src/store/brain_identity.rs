//! Brain identity descriptions v1 store: trusted hosted observations and the
//! scoped exact-key description snapshot. See
//! `finitecomputer-v2/docs/brain-identity-descriptions-v1.md`.

mod observations;
mod scoped;
mod scoped_operations;
mod snapshot;

use super::*;
use crate::brain_identity::{
    AgentLifecycle, BrainAccountObservationRequest, BrainIdentityDescriptionsRequest,
    DESCRIPTION_BUDGET, DescribedKind, DescriptionSource, DescriptionState, IdentityDescription,
    MAX_AGENT_RUNTIME_ROWS, MAX_CONTACT_BYTES, MAX_DISPLAY_NAME_BYTES, MAX_RESPONSIBLE_HUMAN_KEYS,
    OBSERVATION_SKEW_SECONDS, OBSERVATION_WINDOW_SECONDS, ObservationActionKind,
    ObservationOutcome, ResponsibleAccount, npub_for_hex, opaque_revision,
};
use crate::generate_surrogate_id;
use std::collections::{BTreeMap, BTreeSet};

/// Observation failures that are not plain store errors. Messages are stable
/// codes and never carry contact details.
#[derive(Debug)]
pub enum BrainObservationError {
    /// `observedAt` is outside the freshness window for a new operation.
    Stale,
    /// The operation id was already used by this account with another payload.
    OperationReused,
    /// The key is actively associated with a different account.
    KeyAssociatedElsewhere,
    /// The observed human key is also a recorded hosted agent key.
    KeyIsAgentRecord,
    /// The participating agent is not a live agent owned by this account.
    AgentNotOwned,
    /// The verified WorkOS subject has no linked Core account. This route
    /// never enrolls an account.
    AccountNotLinked,
    Store(CoreError),
}

impl From<CoreError> for BrainObservationError {
    fn from(error: CoreError) -> Self {
        Self::Store(error)
    }
}

struct AgentPinRow {
    npub: String,
    runtime_id: String,
    /// `None` when the runtime's project row is missing or the runtime has an
    /// active link to a different project: the record is inconsistent.
    consistent: Option<ConsistentPin>,
    offboarding_phase: Option<String>,
    active_link: bool,
    observed_at: String,
    updated_at: String,
}

struct ConsistentPin {
    project_id: String,
    owner_user_id: String,
    /// `None` when Core's value is over the wire bound (never truncated).
    display_name: Option<String>,
}

// LEFT JOIN so a pin whose project is missing is still returned (and fails
// closed) instead of being joined away; a foreign active link marks the
// record inconsistent.
const AGENT_PIN_ROWS_SQL: &str = "
SELECT runtime.health_reporting_npub AS npub,
       runtime.id AS runtime_id,
       runtime.project_id,
       project.id AS joined_project_id,
       project.owner_user_id,
       CASE WHEN octet_length(project.display_name) <= $3 THEN project.display_name END
         AS display_name,
       runtime.offboarding_phase,
       EXISTS (
         SELECT 1 FROM project_runtime_links AS link
         WHERE link.agent_runtime_id = runtime.id
           AND link.project_id = runtime.project_id
           AND link.active
       ) AS active_link,
       EXISTS (
         SELECT 1 FROM project_runtime_links AS link
         WHERE link.agent_runtime_id = runtime.id
           AND link.project_id IS DISTINCT FROM runtime.project_id
           AND link.active
       ) AS foreign_active_link,
       core_rfc3339(COALESCE(runtime.health_reported_at, runtime.updated_at)) AS observed_at,
       core_rfc3339(runtime.updated_at) AS updated_at
FROM agent_runtimes AS runtime
LEFT JOIN projects AS project ON project.id = runtime.project_id
WHERE runtime.health_reporting_npub = ANY($1)
ORDER BY runtime.id
LIMIT $2";

async fn agent_pin_rows<C>(client: &C, npubs: &[String]) -> CoreResult<Vec<AgentPinRow>>
where
    C: GenericClient + Sync,
{
    let rows = client
        .query(
            AGENT_PIN_ROWS_SQL,
            &[
                &npubs,
                &(MAX_AGENT_RUNTIME_ROWS + 1),
                &(MAX_DISPLAY_NAME_BYTES as i32),
            ],
        )
        .await
        .map_err(store_error)?;
    if rows.len() as i64 > MAX_AGENT_RUNTIME_ROWS {
        return Err(CoreError::Store(
            "Brain identity lookup exceeded its runtime row bound".to_string(),
        ));
    }
    Ok(rows
        .iter()
        .map(|row| {
            let joined: Option<String> = row.get("joined_project_id");
            let foreign: bool = row.get("foreign_active_link");
            let consistent = match (joined, foreign) {
                (Some(project_id), false) => Some(ConsistentPin {
                    project_id,
                    owner_user_id: row.get("owner_user_id"),
                    display_name: row.get("display_name"),
                }),
                _ => None,
            };
            AgentPinRow {
                npub: row.get("npub"),
                runtime_id: row.get("runtime_id"),
                consistent,
                offboarding_phase: row.get("offboarding_phase"),
                active_link: row.get("active_link"),
                observed_at: row.get("observed_at"),
                updated_at: row.get("updated_at"),
            }
        })
        .collect())
}

/// What one exact key's runtime pins say, after the lifecycle rule. Rows of
/// one project are one agent; differing projects or owners are ambiguous; a missing project or foreign active link is
/// inconsistent. Neither ambiguity nor inconsistency selects an owner.
enum AgentEvidence<'a> {
    None,
    Ambiguous(BTreeSet<&'a str>),
    /// At least one pin has a missing project or a foreign active link.
    Inconsistent,
    Agent {
        pin: &'a ConsistentPin,
        /// `None` when an active link and lifecycle phase disagree, or the
        /// agent has neither a live link nor a completed retirement.
        lifecycle: Option<AgentLifecycle>,
        evidence_row: &'a AgentPinRow,
    },
}

fn agent_evidence<'a>(rows: &[&'a AgentPinRow]) -> AgentEvidence<'a> {
    if rows.is_empty() {
        return AgentEvidence::None;
    }
    let consistent = rows
        .iter()
        .filter_map(|row| row.consistent.as_ref().map(|pin| (*row, pin)))
        .collect::<Vec<_>>();
    let owners = consistent
        .iter()
        .map(|(_, pin)| pin.owner_user_id.as_str())
        .collect::<BTreeSet<_>>();
    if consistent.len() != rows.len() {
        return AgentEvidence::Inconsistent;
    }
    let projects = consistent
        .iter()
        .map(|(_, pin)| pin.project_id.as_str())
        .collect::<BTreeSet<_>>();
    if projects.len() > 1 || owners.len() > 1 {
        return AgentEvidence::Ambiguous(owners);
    }
    let (first_row, pin) = consistent[0];
    let retired_phase = |row: &AgentPinRow| {
        matches!(
            row.offboarding_phase.as_deref(),
            Some("link_deactivated" | "archived")
        )
    };
    // One active link per project is guaranteed by
    // project_runtime_links_one_active_runtime.
    let (lifecycle, evidence_row) = if let Some(active) = rows.iter().find(|row| row.active_link) {
        let lifecycle = match active.offboarding_phase.as_deref() {
            None => Some(AgentLifecycle::Active),
            Some("retirement_requested" | "receipt_verified" | "compute_removed") => {
                Some(AgentLifecycle::Offboarding)
            }
            // An active link on a deactivated/archived runtime is inconsistent.
            Some(_) => None,
        };
        (lifecycle, *active)
    } else if rows.iter().all(|row| retired_phase(row)) {
        // Every incarnation finished retirement: the most recent record is
        // the evidence. The responsible account is still the project's
        // current owner, not a remembered one.
        let latest = rows
            .iter()
            .max_by(|left, right| left.updated_at.cmp(&right.updated_at))
            .copied()
            .unwrap_or(first_row);
        (Some(AgentLifecycle::Retired), latest)
    } else {
        // No active link and not every row retired (for example an inactive
        // row with no offboarding phase): unsupported evidence. Do not guess.
        (None, first_row)
    };
    AgentEvidence::Agent {
        pin,
        lifecycle,
        evidence_row,
    }
}

struct AccountRow {
    /// `None` when the stored email is over the wire bound.
    email: Option<String>,
    /// WorkOS name; `None` when absent or over the wire bound.
    display_name: Option<String>,
    linked: bool,
    workos_subject: Option<String>,
    updated_at: String,
}

struct HumanAssociationRow {
    id: String,
    user_id: String,
    last_observed_at: String,
    revision: i64,
}

impl CoreStore {
    /// Describe exact keys for one Brain audience from one REPEATABLE READ
    /// snapshot. Results are returned in request order. Under v1 a key whose
    /// responsible account has not shared with this exact Brain is
    /// `notShared`, identical to a key Core does not know; v2 skips that
    /// check (see `requires_sharing_scope`).
    pub async fn describe_brain_identities(
        &self,
        request: &BrainIdentityDescriptionsRequest,
    ) -> CoreResult<Vec<IdentityDescription>> {
        let mut client = self.connection().await?;
        let tx = client
            .build_transaction()
            .read_only(true)
            .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
            .start()
            .await
            .map_err(store_error)?;
        // Bound the SQL itself; dropping a timed-out future would leave the
        // query running on a pooled connection.
        tx.batch_execute(&format!(
            "SET LOCAL statement_timeout = '{}ms'",
            DESCRIPTION_BUDGET.as_millis()
        ))
        .await
        .map_err(store_error)?;

        let hints =
            (!request.requires_sharing_scope()).then_some(request.public_keys_hex.as_slice());
        let snapshot =
            snapshot::IdentitySnapshot::load(&*tx, request, &request.public_keys_hex, &[], hints)
                .await?;
        tx.commit().await.map_err(store_error)?;
        Ok(request
            .public_keys_hex
            .iter()
            .map(|key| snapshot.describe(key))
            .collect())
    }
}

fn describe_key(
    key: &str,
    human: Option<&HumanAssociationRow>,
    agent: AgentEvidence<'_>,
    disclosable: &dyn Fn(&str) -> bool,
    accounts: &BTreeMap<String, AccountRow>,
    human_keys_by_account: &BTreeMap<String, Vec<String>>,
) -> IdentityDescription {
    let not_shared = || IdentityDescription::bare(key, DescriptionState::NotShared);
    match (human, agent) {
        (None, AgentEvidence::None) => not_shared(),
        (Some(human), AgentEvidence::None) => {
            if !disclosable(&human.user_id) {
                return not_shared();
            }
            let account = &accounts[&human.user_id];
            let Some(email) = bounded(account.email.as_deref(), MAX_CONTACT_BYTES) else {
                return IdentityDescription::bare(key, DescriptionState::Unknown);
            };
            IdentityDescription {
                kind: Some(DescribedKind::Human),
                // The account's WorkOS name, released with the email under
                // the same gate. Optional: an unusable name is omitted.
                display_name: bounded(account.display_name.as_deref(), MAX_DISPLAY_NAME_BYTES),
                account_email: Some(email),
                source: Some(DescriptionSource {
                    kind: "hostedDeviceObservation".to_string(),
                    observed_at: human.last_observed_at.clone(),
                    revision: opaque_revision(
                        "account-key",
                        &[&human.id, &human.revision.to_string()],
                    ),
                }),
                ..IdentityDescription::bare(key, DescriptionState::Resolved)
            }
        }
        // A human association and an agent pin on one key, or pins on
        // several projects/owners, are conflicting authoritative records.
        // Say so only to an audience every implicated account shared with.
        (Some(human), AgentEvidence::Agent { pin, .. }) => {
            if disclosable(&human.user_id) && disclosable(&pin.owner_user_id) {
                IdentityDescription::bare(key, DescriptionState::Ambiguous)
            } else {
                not_shared()
            }
        }
        (human, AgentEvidence::Ambiguous(owners)) => {
            if owners.iter().all(|owner| disclosable(owner))
                && human.is_none_or(|human| disclosable(&human.user_id))
            {
                IdentityDescription::bare(key, DescriptionState::Ambiguous)
            } else {
                not_shared()
            }
        }
        // A pin with a missing project or a foreign active link: no owner
        // can be selected and no account's scope can be checked. Fail closed.
        (_, AgentEvidence::Inconsistent) => not_shared(),
        (
            None,
            AgentEvidence::Agent {
                pin,
                lifecycle,
                evidence_row,
            },
        ) => {
            let owner_user_id = pin.owner_user_id.as_str();
            if !disclosable(owner_user_id) {
                return not_shared();
            }
            let Some(lifecycle) = lifecycle else {
                return IdentityDescription::bare(key, DescriptionState::Unknown);
            };
            let account = &accounts[owner_user_id];
            let (Some(email), Some(display_name)) = (
                bounded(account.email.as_deref(), MAX_CONTACT_BYTES),
                bounded(pin.display_name.as_deref(), MAX_DISPLAY_NAME_BYTES),
            ) else {
                return IdentityDescription::bare(key, DescriptionState::Unknown);
            };
            IdentityDescription {
                kind: Some(DescribedKind::Agent),
                display_name: Some(display_name),
                lifecycle: Some(lifecycle),
                responsible_account: Some(ResponsibleAccount {
                    email,
                    source: "coreAccountContact".to_string(),
                    observed_at: account.updated_at.clone(),
                    human_public_keys_hex: human_keys_by_account
                        .get(owner_user_id)
                        .cloned()
                        .unwrap_or_default(),
                }),
                source: Some(DescriptionSource {
                    kind: "finiteRuntimeRecord".to_string(),
                    observed_at: evidence_row.observed_at.clone(),
                    revision: opaque_revision(
                        "runtime-record",
                        &[&evidence_row.runtime_id, &evidence_row.updated_at],
                    ),
                }),
                ..IdentityDescription::bare(key, DescriptionState::Resolved)
            }
        }
    }
}

/// A Core text field fit for the wire: present, non-empty, within `max` bytes
/// and free of control characters. SQL already withholds oversized values;
/// anything unsupported fails closed and is never truncated.
fn bounded(value: Option<&str>, max: usize) -> Option<String> {
    let value = value?;
    (!value.is_empty() && value.len() <= max && !value.chars().any(char::is_control))
        .then(|| value.to_string())
}
