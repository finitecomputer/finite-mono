//! Rollback-only preview and atomic, receipt-bound apply/retire transactions.
mod locks;
mod retirement;
use super::scoped::{RECORD_SELECT, binding, read_record};
use super::snapshot::IdentitySnapshot;
use super::*;
use crate::brain_identity::scoped::*;

pub(super) fn refused(message: &str) -> CoreError {
    CoreError::Store(message.to_string())
}

impl CoreStore {
    /// The CLI enforces root identity and configured-server equality. The
    /// store repeats scope validation and resolves the operator in the same
    /// transaction as every binding check and write. Dry-run stores execute
    /// exactly this path, then roll back through `finish`.
    pub async fn execute_brain_scoped_manifest(
        &self,
        manifest: &ScopedManifest,
        configured_server: &str,
        now: time::OffsetDateTime,
    ) -> CoreResult<ScopedReceipt> {
        manifest.validate(configured_server).map_err(refused)?;
        let digest = manifest.digest();
        let mut client = self.connection().await?;
        let tx = client.transaction().await.map_err(store_error)?;
        tx.batch_execute("SET LOCAL statement_timeout = '5s'; SET LOCAL lock_timeout = '3s'")
            .await
            .map_err(store_error)?;
        let operator = locks::lock_accounts(&*tx, manifest).await?;
        let now_text = now
            .format(&Rfc3339)
            .map_err(|_| CoreError::InvalidTimestamp)?;
        let operation_id = generate_surrogate_id("brain_scope_op")?;
        // UNIQUE(operation_id) serializes exact retries even when different
        // operators race. A failed transaction leaves no placeholder receipt.
        let inserted = tx
            .execute(
                "INSERT INTO brain_scoped_record_operations
            (id, operation_id, manifest_sha256, action, brain_server, brain_id,
             operator_account_id, operator_workos_subject, operator_email,
             target_operation_id, recorded_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11::text::timestamptz)
            ON CONFLICT (operation_id) DO NOTHING",
                &[
                    &operation_id,
                    &manifest.operation_id,
                    &digest,
                    &manifest.action.as_str(),
                    &manifest.brain_server,
                    &manifest.brain_id,
                    &operator,
                    &manifest.operator.workos_subject,
                    &manifest.operator.email,
                    &manifest.target_operation_id,
                    &now_text,
                ],
            )
            .await
            .map_err(store_error)?;
        if inserted == 0 {
            let row = tx
                .query_one(
                    "SELECT manifest_sha256, receipt FROM brain_scoped_record_operations
                WHERE operation_id = $1 FOR UPDATE",
                    &[&manifest.operation_id],
                )
                .await
                .map_err(store_error)?;
            if row.get::<_, String>("manifest_sha256") != digest {
                return Err(refused(
                    "scoped operation id reused with different manifest",
                ));
            }
            let receipt: Option<Value> = row.get("receipt");
            let receipt: ScopedReceipt = serde_json::from_value(
                receipt.ok_or_else(|| refused("incomplete scoped operation receipt"))?,
            )
            .map_err(json_error)?;
            self.finish(tx).await?;
            return Ok(receipt);
        }
        if !manifest.fresh(now) {
            return Err(refused(
                "scoped manifest is outside its evidence/application window",
            ));
        }
        let records = match manifest.action {
            ScopedAction::Apply => apply(&*tx, manifest, &now_text).await?,
            ScopedAction::Retire => retirement::retire(&*tx, manifest, &now_text).await?,
        };
        let receipt = ScopedReceipt {
            operation_id: manifest.operation_id.clone(),
            manifest_sha256: digest,
            operator_account_id: operator,
            action: manifest.action,
            brain_server: manifest.brain_server.clone(),
            brain_id: manifest.brain_id.clone(),
            target_operation_id: manifest.target_operation_id.clone(),
            recorded_at: now_text,
            records,
        };
        tx.execute(
            "UPDATE brain_scoped_record_operations SET receipt = $2 WHERE operation_id = $1",
            &[
                &manifest.operation_id,
                &serde_json::to_value(&receipt).map_err(json_error)?,
            ],
        )
        .await
        .map_err(store_error)?;
        self.finish(tx).await?;
        tracing::info!(
            operation = "brain_scoped_records",
            action = manifest.action.as_str(),
            records = receipt.records.len(),
            "scoped description operation finished"
        );
        Ok(receipt)
    }
}

async fn apply<C: GenericClient + Sync>(
    client: &C,
    manifest: &ScopedManifest,
    now: &str,
) -> CoreResult<Vec<ScopedRecordReceipt>> {
    locks::lock_evidence(client, manifest).await?;
    let mut records: Vec<_> = manifest.records.iter().collect();
    records.sort_by_key(|r| &r.public_key_hex);
    let keys: Vec<_> = records.iter().map(|r| r.public_key_hex.clone()).collect();
    let accounts: Vec<_> = records
        .iter()
        .filter_map(|r| r.account_id.clone())
        .collect();
    let request = BrainIdentityDescriptionsRequest {
        version: crate::brain_identity::DESCRIPTIONS_VERSION_V2.to_string(),
        brain_server: manifest.brain_server.clone(),
        brain_id: manifest.brain_id.clone(),
        requested_by_public_key_hex: "00".repeat(32),
        public_keys_hex: keys.clone(),
    };
    let snapshot = IdentitySnapshot::load(client, &request, &keys, &accounts, Some(&[])).await?;
    let mut results = Vec::new();
    for record in records {
        let valid = binding(record, &snapshot)
            .map_err(|reason| refused(&format!("scoped binding conflict: {reason:?}")))?;
        if valid.description.state == DescriptionState::Unknown {
            return Err(refused("scoped binding has no usable current contact"));
        }
        if let Some(row) = client.query_opt(&format!("{RECORD_SELECT}
            WHERE brain_server=$1 AND brain_id=$2 AND public_key_hex=$3 AND retired_at IS NULL FOR UPDATE"),
            &[&manifest.brain_server, &manifest.brain_id, &record.public_key_hex]).await.map_err(store_error)? {
            let existing = read_record(&row)?;
            if existing.record != *record { return Err(refused("nonidentical scoped record already exists")); }
            results.push(ScopedRecordReceipt { disposition: ScopedDisposition::Existing,
                record_sha256: record.digest(), afterimage: existing });
            continue;
        }
        let id = generate_surrogate_id("brain_scope_record")?;
        let revision = generate_surrogate_id("revision")?;
        // No conflict winner: an absent-row race is refused by the partial
        // unique index, rolling back this entire batch.
        client.execute("INSERT INTO brain_scoped_records
            (id,revision,brain_server,brain_id,public_key_hex,kind,account_id,workos_subject,project_id,
             annotation_label,evidence_sha256,evidence_reference,evidence_observed_at,creating_operation_id,created_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15::text::timestamptz)",
            &[&id,&revision,&manifest.brain_server,&manifest.brain_id,&record.public_key_hex,&record.kind.as_str(),
              &record.account_id,&record.workos_subject,&record.project_id,&record.annotation_label,
              &record.evidence.sha256,&record.evidence.reference,&record.evidence.observed_at,
              &manifest.operation_id,&now]).await.map_err(store_error)?;
        let afterimage = read_record(
            &client
                .query_one(&format!("{RECORD_SELECT} WHERE id=$1"), &[&id])
                .await
                .map_err(store_error)?,
        )?;
        results.push(ScopedRecordReceipt {
            disposition: ScopedDisposition::Created,
            record_sha256: record.digest(),
            afterimage,
        });
    }
    Ok(results)
}
