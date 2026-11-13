use super::*;

pub(super) async fn retire<C: GenericClient + Sync>(
    client: &C,
    manifest: &ScopedManifest,
    now: &str,
) -> CoreResult<Vec<ScopedRecordReceipt>> {
    let target = manifest
        .target_operation_id
        .as_deref()
        .ok_or_else(|| refused("missing retirement target"))?;
    let row = client
        .query_opt(
            "SELECT receipt FROM brain_scoped_record_operations WHERE operation_id=$1 FOR UPDATE",
            &[&target],
        )
        .await
        .map_err(store_error)?
        .ok_or_else(|| refused("retirement target does not exist"))?;
    let receipt: Option<Value> = row.get("receipt");
    let source: ScopedReceipt =
        serde_json::from_value(receipt.ok_or_else(|| refused("retirement target is incomplete"))?)
            .map_err(json_error)?;
    if source.action != ScopedAction::Apply
        || source.brain_server != manifest.brain_server
        || source.brain_id != manifest.brain_id
    {
        return Err(refused("retirement target scope or action mismatch"));
    }
    let mut created: Vec<_> = source
        .records
        .iter()
        .filter(|r| r.disposition == ScopedDisposition::Created)
        .collect();
    created.sort_by_key(|r| &r.afterimage.record.public_key_hex);
    let mut result = Vec::new();
    for expected in created {
        let row = client
            .query_opt(
                &format!("{RECORD_SELECT} WHERE id=$1 FOR UPDATE"),
                &[&expected.afterimage.id],
            )
            .await
            .map_err(store_error)?
            .ok_or_else(|| refused("scoped retirement afterimage missing"))?;
        let current = read_record(&row)?;
        if current != expected.afterimage
            || current.retired_at.is_some()
            || current.creating_operation_id != target
            || current.record.digest() != expected.record_sha256
        {
            return Err(refused(
                "scoped retirement afterimage changed; nothing retired",
            ));
        }
        let revision = generate_surrogate_id("revision")?;
        let updated = client
            .execute(
                "UPDATE brain_scoped_records SET retired_at=$3::text::timestamptz, revision=$4
            WHERE id=$1 AND revision=$2 AND retired_at IS NULL",
                &[&current.id, &current.revision, &now, &revision],
            )
            .await
            .map_err(store_error)?;
        if updated != 1 {
            return Err(refused("scoped retirement compare-and-swap failed"));
        }
        let afterimage = read_record(
            &client
                .query_one(&format!("{RECORD_SELECT} WHERE id=$1"), &[&current.id])
                .await
                .map_err(store_error)?,
        )?;
        result.push(ScopedRecordReceipt {
            disposition: ScopedDisposition::Retired,
            record_sha256: expected.record_sha256.clone(),
            afterimage,
        });
    }
    Ok(result)
}
