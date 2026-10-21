//! Exact-scope eligibility, binding checks and precedence in one snapshot.
use super::snapshot::IdentitySnapshot;
use super::*;
use crate::brain_identity::scoped::*;

pub(super) const RECORD_SELECT: &str = "SELECT id, revision, brain_server, brain_id,
    public_key_hex, kind, account_id, workos_subject, project_id, annotation_label,
    evidence_sha256, evidence_reference, evidence_observed_at, creating_operation_id,
    core_rfc3339(created_at) AS created_at, core_rfc3339(retired_at) AS retired_at
    FROM brain_scoped_records";

pub(super) fn read_record(row: &Row) -> CoreResult<ScopedStoredRecord> {
    let kind = match row.get::<_, &str>("kind") {
        "managed_agent" => ScopedRecordKind::ManagedAgent,
        "account_chat_key" => ScopedRecordKind::AccountChatKey,
        "historical_identity" => ScopedRecordKind::HistoricalIdentity,
        "invitation_key" => ScopedRecordKind::InvitationKey,
        _ => {
            return Err(CoreError::Store(
                "invalid stored scoped record kind".to_string(),
            ));
        }
    };
    let record = ScopedRecordInput {
        public_key_hex: row.get("public_key_hex"),
        kind,
        account_id: row.get("account_id"),
        workos_subject: row.get("workos_subject"),
        project_id: row.get("project_id"),
        annotation_label: row.get("annotation_label"),
        evidence: ScopedEvidence {
            sha256: row.get("evidence_sha256"),
            reference: row.get("evidence_reference"),
            observed_at: row.get("evidence_observed_at"),
        },
    };
    record
        .validate()
        .map_err(|e| CoreError::Store(e.to_string()))?;
    Ok(ScopedStoredRecord {
        id: row.get("id"),
        revision: row.get("revision"),
        brain_server: row.get("brain_server"),
        brain_id: row.get("brain_id"),
        creating_operation_id: row.get("creating_operation_id"),
        created_at: row.get("created_at"),
        retired_at: row.get("retired_at"),
        record,
    })
}

impl CoreStore {
    pub async fn describe_scoped_brain_identities(
        &self,
        request: &ScopedDescriptionsRequest,
    ) -> CoreResult<Vec<ScopedIdentityDescription>> {
        // The HTTP boundary authenticates the configured server. Internal
        // callers must still obey the page/subset bounds before any SQL.
        request
            .validate(&request.brain_server)
            .map_err(|error| CoreError::Store(error.to_string()))?;
        let mut client = self.connection().await?;
        let tx = client
            .build_transaction()
            .read_only(true)
            .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
            .start()
            .await
            .map_err(store_error)?;
        tx.batch_execute(&format!(
            "SET LOCAL statement_timeout = '{}ms'",
            DESCRIPTION_BUDGET.as_millis()
        ))
        .await
        .map_err(store_error)?;
        let records = tx
            .query(
                &format!(
                    "{RECORD_SELECT}
            WHERE brain_server = $1 AND brain_id = $2 AND public_key_hex = ANY($3)
              AND retired_at IS NULL ORDER BY public_key_hex"
                ),
                &[
                    &request.brain_server,
                    &request.brain_id,
                    &request.public_keys_hex,
                ],
            )
            .await
            .map_err(store_error)?
            .iter()
            .map(read_record)
            .collect::<CoreResult<Vec<_>>>()?;
        let participating: BTreeSet<_> = request
            .participating_public_keys_hex
            .iter()
            .cloned()
            .collect();
        let mut eligible = participating.clone();
        eligible.extend(records.iter().map(|row| row.record.public_key_hex.clone()));
        let eligible: Vec<_> = eligible.into_iter().collect();
        let accounts: Vec<_> = records
            .iter()
            .filter_map(|r| r.record.account_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let snapshot = IdentitySnapshot::load(
            &*tx,
            &request.legacy_scope(),
            &eligible,
            &accounts,
            Some(&request.participating_public_keys_hex),
        )
        .await?;
        let by_key: BTreeMap<_, _> = records
            .iter()
            .map(|r| (r.record.public_key_hex.as_str(), r))
            .collect();
        let result = request
            .public_keys_hex
            .iter()
            .map(|key| {
                let normal = if participating.contains(key) {
                    snapshot.describe(key)
                } else {
                    IdentityDescription::bare(key, DescriptionState::NotShared)
                };
                match by_key.get(key.as_str()) {
                    Some(record) => overlay(normal, record, &snapshot),
                    None => normal.into(),
                }
            })
            .collect();
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }
}

pub(super) struct ValidBinding {
    pub(super) description: IdentityDescription,
    pub(super) annotation: Option<OperatorAnnotation>,
}

/// Reused by apply and read. A normal notShared result is never used as proof
/// that the underlying key is unbound or safe to disclose.
pub(super) fn binding(
    record: &ScopedRecordInput,
    snapshot: &IdentitySnapshot,
) -> Result<ValidBinding, OperatorRecordReason> {
    use OperatorRecordReason::*;
    let key = &record.public_key_hex;
    let human = snapshot.human(key);
    let pins = snapshot.pins(key);
    if matches!(
        record.kind,
        ScopedRecordKind::HistoricalIdentity | ScopedRecordKind::InvitationKey
    ) {
        if human.is_some() || !pins.is_empty() {
            return Err(AnnotationSuperseded);
        }
        let annotation = OperatorAnnotation {
            kind: if record.kind == ScopedRecordKind::HistoricalIdentity {
                AnnotationKind::HistoricalIdentity
            } else {
                AnnotationKind::InvitationKey
            },
            label: record
                .annotation_label
                .clone()
                .ok_or(AnnotationSuperseded)?,
            observed_at: record.evidence.observed_at.clone(),
        };
        if !annotation.valid() {
            return Err(AnnotationSuperseded);
        }
        return Ok(ValidBinding {
            description: IdentityDescription::bare(key, DescriptionState::NotShared),
            annotation: Some(annotation),
        });
    }
    let id = record.account_id.as_deref().ok_or(AccountBindingChanged)?;
    let account = snapshot
        .account(id)
        .filter(|account| account.linked && account.workos_subject == record.workos_subject)
        .ok_or(AccountBindingChanged)?;
    let unknown = || ValidBinding {
        description: IdentityDescription::bare(key, DescriptionState::Unknown),
        annotation: None,
    };
    if record.kind == ScopedRecordKind::AccountChatKey {
        if !pins.is_empty() || human.is_some_and(|row| row.user_id != id) {
            return Err(IdentityEvidenceConflict);
        }
        let Some(email) = bounded(account.email.as_deref(), MAX_CONTACT_BYTES) else {
            return Ok(unknown());
        };
        return Ok(ValidBinding {
            description: IdentityDescription {
                kind: Some(DescribedKind::Human),
                account_email: Some(email),
                ..IdentityDescription::bare(key, DescriptionState::Resolved)
            },
            annotation: None,
        });
    }
    if human.is_some() {
        return Err(IdentityEvidenceConflict);
    }
    let AgentEvidence::Agent {
        pin,
        lifecycle: Some(lifecycle),
        ..
    } = agent_evidence(&pins)
    else {
        return Err(AgentBindingChanged);
    };
    if pin.owner_user_id != id || Some(&pin.project_id) != record.project_id.as_ref() {
        return Err(AgentBindingChanged);
    }
    let (Some(email), Some(name)) = (
        bounded(account.email.as_deref(), MAX_CONTACT_BYTES),
        bounded(pin.display_name.as_deref(), MAX_DISPLAY_NAME_BYTES),
    ) else {
        return Ok(unknown());
    };
    Ok(ValidBinding {
        description: IdentityDescription {
            kind: Some(DescribedKind::Agent),
            display_name: Some(name),
            lifecycle: Some(lifecycle),
            responsible_account: Some(ResponsibleAccount {
                email,
                source: "coreAccountContact".to_string(),
                observed_at: account.updated_at.clone(),
                human_public_keys_hex: Vec::new(),
            }),
            ..IdentityDescription::bare(key, DescriptionState::Resolved)
        },
        annotation: None,
    })
}

fn overlay(
    normal: IdentityDescription,
    record: &ScopedStoredRecord,
    snapshot: &IdentitySnapshot,
) -> ScopedIdentityDescription {
    let revision = opaque_revision("operator-record", &[&record.id, &record.revision]);
    let observed_at = record.record.evidence.observed_at.clone();
    let diagnostic = |state, reason| OperatorRecord {
        state,
        reason,
        revision: revision.clone(),
        observed_at: observed_at.clone(),
    };
    let wins = normal.state != DescriptionState::NotShared;
    let checked = binding(&record.record, snapshot);
    let mut result: ScopedIdentityDescription = normal.into();
    result.operator_record = Some(match checked {
        Err(reason) => diagnostic(OperatorRecordState::Conflict, Some(reason)),
        Ok(_) if wins => diagnostic(
            OperatorRecordState::Suppressed,
            Some(OperatorRecordReason::NormalResultPrecedence),
        ),
        Ok(valid) if valid.description.state == DescriptionState::Unknown => {
            result = valid.description.into();
            diagnostic(
                OperatorRecordState::Suppressed,
                Some(OperatorRecordReason::ContactUnavailable),
            )
        }
        Ok(mut valid) => {
            if valid.description.state == DescriptionState::Resolved {
                valid.description.source = Some(DescriptionSource {
                    kind: OPERATOR_SOURCE.to_string(),
                    revision: revision.clone(),
                    observed_at: observed_at.clone(),
                });
            }
            result = valid.description.into();
            result.operator_annotation = valid.annotation;
            diagnostic(OperatorRecordState::Used, None)
        }
    });
    result
}
