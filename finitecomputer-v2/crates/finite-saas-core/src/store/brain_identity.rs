//! Brain identity descriptions v1 store: trusted hosted observations and the
//! scoped exact-key description snapshot. See
//! `finitecomputer-v2/docs/brain-identity-descriptions-v1.md`.

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
    linked: bool,
    updated_at: String,
}

struct HumanAssociationRow {
    id: String,
    user_id: String,
    last_observed_at: String,
    revision: i64,
}

impl CoreStore {
    /// Atomically record one trusted hosted observation: the human key
    /// association (when present), the account/Brain sharing scope and the
    /// idempotency receipt. An exact retry returns the stored outcome without
    /// re-checking freshness or re-applying any write.
    pub async fn record_brain_account_observation(
        &self,
        workos_user_id: &str,
        issuer: &str,
        request: &BrainAccountObservationRequest,
        now: time::OffsetDateTime,
    ) -> Result<ObservationOutcome, BrainObservationError> {
        let payload_sha256 = request.payload_sha256();
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.batch_execute("SET LOCAL statement_timeout = '5s'")
            .await
            .map_err(store_error)?;
        // Resolve an existing linked account only; no user, personal org or
        // link is created or changed here. The row lock (scoped to this
        // account) serializes its observations, so a concurrent duplicate of
        // one operation reads the first attempt's receipt instead of failing
        // on its unique constraint. It does not block key-share readers.
        let user_id: String = tx
            .query_opt(
                "SELECT id FROM users
                 WHERE workos_user_id = $1 AND link_status = 'linked'
                 FOR NO KEY UPDATE",
                &[&workos_user_id],
            )
            .await
            .map_err(store_error)?
            .ok_or(BrainObservationError::AccountNotLinked)?
            .get("id");
        let user_id = user_id.as_str();

        if let Some(receipt) = tx
            .query_opt(
                "SELECT issuer, payload_sha256, outcome FROM brain_account_observation_receipts
                 WHERE user_id = $1 AND operation_id = $2",
                &[&user_id, &request.operation_id],
            )
            .await
            .map_err(store_error)?
        {
            tx.rollback().await.map_err(store_error)?;
            let stored_issuer: String = receipt.get("issuer");
            let stored_payload: String = receipt.get("payload_sha256");
            if stored_issuer != issuer || stored_payload != payload_sha256 {
                return Err(BrainObservationError::OperationReused);
            }
            let outcome: String = receipt.get("outcome");
            return ObservationOutcome::parse(&outcome).ok_or_else(|| {
                CoreError::Store("stored Brain observation outcome is invalid".to_string()).into()
            });
        }

        let observed = parse_time(&request.observed_at)?;
        let age = now - observed;
        if age > time::Duration::seconds(OBSERVATION_WINDOW_SECONDS + OBSERVATION_SKEW_SECONDS)
            || -age > time::Duration::seconds(OBSERVATION_SKEW_SECONDS)
        {
            return Err(BrainObservationError::Stale);
        }
        let now_text = now
            .format(&Rfc3339)
            .map_err(|_| CoreError::InvalidTimestamp)?;

        if request.action_kind == ObservationActionKind::OwnedAgentHostedAction {
            let npub = npub_for_hex(&request.participating_public_key_hex)
                .ok_or(BrainObservationError::AgentNotOwned)?;
            let rows = agent_pin_rows(&*tx, &[npub]).await?;
            let refs = rows.iter().collect::<Vec<_>>();
            let owned_and_live = matches!(
                agent_evidence(&refs),
                AgentEvidence::Agent {
                    pin,
                    lifecycle: Some(AgentLifecycle::Active | AgentLifecycle::Offboarding),
                    ..
                } if pin.owner_user_id == user_id
            );
            if !owned_and_live {
                return Err(BrainObservationError::AgentNotOwned);
            }
        }

        let mut changed = false;
        if let Some(human_key) = request.observed_human_public_key_hex.as_deref() {
            let npub = npub_for_hex(human_key).ok_or_else(|| {
                CoreError::Store("validated human key did not encode".to_string())
            })?;
            if !agent_pin_rows(&*tx, &[npub]).await?.is_empty() {
                return Err(BrainObservationError::KeyIsAgentRecord);
            }
            let association_id = generate_surrogate_id("account_key")?;
            let inserted = tx
                .query_opt(
                    "INSERT INTO account_brain_principals (
                       id, user_id, public_key_hex, source, issuer, status,
                       first_observed_at, last_observed_at, revision
                     ) VALUES ($1, $2, $3, 'hosted_device_observation', $4, 'active',
                               $5::text::timestamptz, $5::text::timestamptz, 1)
                     ON CONFLICT (public_key_hex) WHERE status = 'active' DO NOTHING
                     RETURNING id",
                    &[&association_id, &user_id, &human_key, &issuer, &now_text],
                )
                .await
                .map_err(store_error)?;
            if inserted.is_some() {
                changed = true;
            } else {
                // The conflicting active row may in principle have changed
                // since the insert; anything but this account's own active
                // association is refused.
                let owner: Option<String> = tx
                    .query_opt(
                        "SELECT user_id FROM account_brain_principals
                         WHERE public_key_hex = $1 AND status = 'active'",
                        &[&human_key],
                    )
                    .await
                    .map_err(store_error)?
                    .map(|row| row.get("user_id"));
                if owner.as_deref() != Some(user_id) {
                    return Err(BrainObservationError::KeyAssociatedElsewhere);
                }
                // Refreshes evidence time only; `revision` names the
                // association itself, which is unchanged.
                tx.execute(
                    "UPDATE account_brain_principals
                     SET last_observed_at = GREATEST(last_observed_at, $3::text::timestamptz)
                     WHERE public_key_hex = $1 AND user_id = $2 AND status = 'active'",
                    &[&human_key, &user_id, &now_text],
                )
                .await
                .map_err(store_error)?;
            }
        }

        let scope_id = generate_surrogate_id("brain_scope")?;
        let scope_inserted = tx
            .execute(
                "INSERT INTO account_brain_sharing_scopes (
                   id, user_id, brain_server, brain_id, issuer, established_at, revoked_at, revision
                 ) VALUES ($1, $2, $3, $4, $5, $6::text::timestamptz, NULL, 1)
                 ON CONFLICT (user_id, brain_server, brain_id) WHERE revoked_at IS NULL DO NOTHING",
                &[
                    &scope_id,
                    &user_id,
                    &request.brain_server,
                    &request.brain_id,
                    &issuer,
                    &now_text,
                ],
            )
            .await
            .map_err(store_error)?;
        changed |= scope_inserted == 1;

        let outcome = if changed {
            ObservationOutcome::Recorded
        } else {
            ObservationOutcome::Unchanged
        };
        let receipt_id = generate_surrogate_id("brain_observation")?;
        tx.execute(
            "INSERT INTO brain_account_observation_receipts (
               id, user_id, operation_id, issuer, payload_sha256, outcome, recorded_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7::text::timestamptz)",
            &[
                &receipt_id,
                &user_id,
                &request.operation_id,
                &issuer,
                &payload_sha256,
                &outcome.as_str(),
                &now_text,
            ],
        )
        .await
        .map_err(store_error)?;
        self.finish(tx).await?;
        Ok(outcome)
    }

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

        let keys = &request.public_keys_hex;
        let npub_by_key = keys
            .iter()
            .filter_map(|key| npub_for_hex(key).map(|npub| (key.clone(), npub)))
            .collect::<BTreeMap<_, _>>();
        let npubs = npub_by_key.values().cloned().collect::<Vec<_>>();

        let humans = tx
            .query(
                "SELECT id, user_id, public_key_hex, core_rfc3339(last_observed_at) AS last_observed_at,
                        revision
                 FROM account_brain_principals
                 WHERE status = 'active' AND public_key_hex = ANY($1)",
                &[keys],
            )
            .await
            .map_err(store_error)?
            .iter()
            .map(|row| {
                (
                    row.get::<_, String>("public_key_hex"),
                    HumanAssociationRow {
                        id: row.get("id"),
                        user_id: row.get("user_id"),
                        last_observed_at: row.get("last_observed_at"),
                        revision: row.get("revision"),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let agent_rows = agent_pin_rows(&*tx, &npubs).await?;
        let mut agents_by_npub: BTreeMap<&str, Vec<&AgentPinRow>> = BTreeMap::new();
        for row in &agent_rows {
            agents_by_npub
                .entry(row.npub.as_str())
                .or_default()
                .push(row);
        }

        let mut implicated = humans
            .values()
            .map(|row| row.user_id.clone())
            .collect::<BTreeSet<_>>();
        implicated.extend(
            agent_rows
                .iter()
                .filter_map(|row| row.consistent.as_ref())
                .map(|pin| pin.owner_user_id.clone()),
        );
        let implicated = implicated.into_iter().collect::<Vec<_>>();

        // v1: only accounts that shared with this exact Brain. v2: every
        // implicated account, because Brain asks only about keys that acted
        // in this Brain themselves.
        let releasable = if request.requires_sharing_scope() {
            tx.query(
                "SELECT user_id FROM account_brain_sharing_scopes
                 WHERE revoked_at IS NULL AND brain_server = $1 AND brain_id = $2
                   AND user_id = ANY($3)",
                &[&request.brain_server, &request.brain_id, &implicated],
            )
            .await
            .map_err(store_error)?
            .iter()
            .map(|row| row.get::<_, String>("user_id"))
            .collect::<BTreeSet<_>>()
        } else {
            implicated.iter().cloned().collect::<BTreeSet<_>>()
        };
        let releasable_ids = releasable.iter().cloned().collect::<Vec<_>>();
        // v2 names only an owner's human keys that are in this same request,
        // never keys outside it; without a sharing scope those would link the
        // owner across Brains.
        let hint_scope = (!request.requires_sharing_scope()).then_some(keys);
        let accounts = tx
            .query(
                "SELECT id,
                        CASE WHEN octet_length(normalized_email) <= $2 THEN normalized_email END
                          AS normalized_email,
                        link_status, core_rfc3339(updated_at) AS updated_at
                 FROM users WHERE id = ANY($1)",
                &[&releasable_ids, &(MAX_CONTACT_BYTES as i32)],
            )
            .await
            .map_err(store_error)?
            .iter()
            .map(|row| {
                (
                    row.get::<_, String>("id"),
                    AccountRow {
                        email: row.get("normalized_email"),
                        linked: row.get::<_, String>("link_status") == "linked",
                        updated_at: row.get("updated_at"),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        // Bounded in SQL: at most MAX_RESPONSIBLE_HUMAN_KEYS rows per releasable
        // account, read through the (user_id, public_key_hex) partial index.
        let mut human_keys_by_account: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for row in tx
            .query(
                "SELECT account.id AS user_id, account_key.public_key_hex
                 FROM unnest($1::text[]) AS account(id)
                 CROSS JOIN LATERAL (
                   SELECT public_key_hex FROM account_brain_principals
                   WHERE user_id = account.id AND status = 'active'
                     AND ($3::text[] IS NULL OR public_key_hex = ANY($3))
                   ORDER BY public_key_hex
                   LIMIT $2
                 ) AS account_key
                 ORDER BY account.id, account_key.public_key_hex",
                &[
                    &releasable_ids,
                    &(MAX_RESPONSIBLE_HUMAN_KEYS as i64),
                    &hint_scope,
                ],
            )
            .await
            .map_err(store_error)?
        {
            human_keys_by_account
                .entry(row.get::<_, String>("user_id"))
                .or_default()
                .push(row.get("public_key_hex"));
        }
        // A human key that is also pinned as an agent is a conflicting
        // record, not a hint. Exclude it with one bounded, indexed equality
        // read over the (at most 8 per account) candidate keys.
        let hint_npubs = human_keys_by_account
            .values()
            .flatten()
            .filter_map(|key| npub_for_hex(key))
            .collect::<Vec<_>>();
        let pinned_hints = tx
            .query(
                "SELECT DISTINCT health_reporting_npub FROM agent_runtimes
                 WHERE health_reporting_npub = ANY($1)",
                &[&hint_npubs],
            )
            .await
            .map_err(store_error)?
            .iter()
            .map(|row| row.get::<_, String>("health_reporting_npub"))
            .collect::<BTreeSet<_>>();
        for keys in human_keys_by_account.values_mut() {
            keys.retain(|key| npub_for_hex(key).is_some_and(|npub| !pinned_hints.contains(&npub)));
        }
        tx.commit().await.map_err(store_error)?;

        // An account may disclose only when it is a linked (WorkOS-verified)
        // account and, under v1, has shared with this exact Brain.
        let disclosable = |user_id: &str| {
            releasable.contains(user_id)
                && accounts.get(user_id).is_some_and(|account| account.linked)
        };
        let empty = Vec::new();
        Ok(keys
            .iter()
            .map(|key| {
                let human = humans.get(key);
                let pins = npub_by_key
                    .get(key)
                    .and_then(|npub| agents_by_npub.get(npub.as_str()))
                    .unwrap_or(&empty);
                describe_key(
                    key,
                    human,
                    agent_evidence(pins),
                    &disclosable,
                    &accounts,
                    &human_keys_by_account,
                )
            })
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
            let Some(email) = bounded(accounts[&human.user_id].email.as_deref(), MAX_CONTACT_BYTES)
            else {
                return IdentityDescription::bare(key, DescriptionState::Unknown);
            };
            IdentityDescription {
                kind: Some(DescribedKind::Human),
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
