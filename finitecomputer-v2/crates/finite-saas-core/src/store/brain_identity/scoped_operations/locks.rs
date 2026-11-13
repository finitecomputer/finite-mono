use super::*;

/// Only affected accounts are locked, in a stable order. No advisory lock or
/// table-wide serialization is introduced into the live account path. Keep
/// FK readers free to insert ordinary account activity while binding updates
/// remain excluded, matching the hosted observation writer's lock strength.
pub(super) async fn lock_accounts<C: GenericClient + Sync>(
    client: &C,
    manifest: &ScopedManifest,
) -> CoreResult<String> {
    let actor = client.query_opt("SELECT id FROM users WHERE normalized_email=$1 AND workos_user_id=$2 AND link_status='linked'",
        &[&manifest.operator.email, &manifest.operator.workos_subject]).await.map_err(store_error)?
        .ok_or_else(|| refused("operator email and WorkOS subject must name one linked account"))?;
    let actor: String = actor.get("id");
    let mut ids = manifest
        .records
        .iter()
        .filter_map(|r| r.account_id.clone())
        .collect::<BTreeSet<_>>();
    ids.insert(actor.clone());
    let rows = client
        .query(
            "SELECT id, normalized_email, workos_user_id, link_status FROM users
        WHERE id=ANY($1) ORDER BY id FOR NO KEY UPDATE",
            &[&ids.into_iter().collect::<Vec<_>>()],
        )
        .await
        .map_err(store_error)?;
    let valid = rows.iter().any(|row| {
        row.get::<_, String>("id") == actor
            && row.get::<_, String>("normalized_email") == manifest.operator.email
            && row.get::<_, Option<String>>("workos_user_id").as_ref()
                == Some(&manifest.operator.workos_subject)
            && row.get::<_, String>("link_status") == "linked"
    });
    if !valid {
        return Err(refused("operator binding changed"));
    }
    Ok(actor)
}

pub(super) async fn lock_evidence<C: GenericClient + Sync>(
    client: &C,
    manifest: &ScopedManifest,
) -> CoreResult<()> {
    let keys: Vec<_> = manifest
        .records
        .iter()
        .map(|r| r.public_key_hex.clone())
        .collect();
    let npubs: Vec<_> = keys.iter().filter_map(|k| npub_for_hex(k)).collect();
    let projects: Vec<_> = manifest
        .records
        .iter()
        .filter_map(|r| r.project_id.clone())
        .collect();
    client
        .query(
            "SELECT id FROM projects WHERE id=ANY($1) ORDER BY id FOR SHARE",
            &[&projects],
        )
        .await
        .map_err(store_error)?;
    client.query("SELECT id FROM agent_runtimes WHERE health_reporting_npub=ANY($1) ORDER BY id FOR SHARE", &[&npubs])
        .await.map_err(store_error)?;
    client
        .query(
            "SELECT id FROM project_runtime_links WHERE project_id=ANY($1) ORDER BY id FOR SHARE",
            &[&projects],
        )
        .await
        .map_err(store_error)?;
    client.query("SELECT id FROM account_brain_principals WHERE public_key_hex=ANY($1) AND status='active'
        ORDER BY public_key_hex FOR SHARE", &[&keys]).await.map_err(store_error)?;
    Ok(())
}
