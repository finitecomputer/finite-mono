use crate::*;

pub(crate) fn remove_identity<W: Write>(
    args: &[String],
    env: &CliEnvironment,
    json: bool,
    output: &mut W,
    demotion: bool,
) -> Result<(), CliError> {
    let brain_id = command_brain_id(args, env)?;
    let raw_target = required_option_or_positional(args, "--target", 1, "target-identity")?;
    let target = resolve_identity_npub(env, args, &raw_target)?;
    // Always refresh before planning. A retry after a committed, lost response is a no-op.
    let metadata = fetch_brain_metadata(env, args, &brain_id)?;
    let server_url = server_url_for_command(env, args)?;
    let export = fetch_encrypted_export(env, &server_url, &brain_id)?;
    ensure_snapshot_alignment(&metadata, &export)?;
    let actor = load_signer(env)?.npub;
    if actor == target {
        return Err(CliError::InvalidInput(
            "another current admin must perform and verify identity removal".to_owned(),
        ));
    }
    if metadata.kind != "organization" || !metadata.admins.contains(&actor) {
        return Err(CliError::InvalidInput(
            "identity removal requires a current Organization Brain admin".to_owned(),
        ));
    }
    let mut post = if demotion {
        metadata.clone()
    } else {
        metadata_after_member_removal(&metadata, &target)
    };
    post.admins.retain(|admin| admin != &target);
    if post.admins.is_empty() {
        return Err(CliError::InvalidInput(
            "organization brain must keep at least one admin".to_owned(),
        ));
    }
    let mut rotations = Vec::new();
    for folder in &metadata.folders {
        let before =
            folder_required_recipients(&metadata, &folder.access, &folder.access_user_ids)?;
        let after = folder_required_recipients(&post, &folder.access, &folder.access_user_ids)?;
        let held = export.key_grants.iter().any(|grant| {
            grant.folder_id == folder.id
                && grant.key_version == folder.current_key_version
                && grant.recipient_npub == target
        });
        if !(before.contains(&target) || held) || (demotion && after.contains(&target)) {
            continue;
        }
        let prepared = prepare_folder_access_removals_from_export(
            env,
            &post,
            &brain_id,
            &folder.id,
            &BTreeSet::from([target.clone()]),
            &export,
        )?;
        rotations.push(serde_json::json!({ "folderId": folder.id,
            "newKeyVersion": prepared["newKeyVersion"], "grants": prepared["grants"],
            "reencryptedRecords": prepared["reencryptedRecords"] }));
    }
    let mut mount_rotations = Vec::new();
    let mounts_route = format!("/v1/brains/{brain_id}/mounts");
    let mounts = signed_json_request(env, args, "GET", &mounts_route, None)?;
    let incoming_mounts = mounts["incoming"].as_array().ok_or_else(|| {
        CliError::InvalidInput("Mount list omitted incoming destination relationships".to_owned())
    })?;

    verify_mount_sources(env, args, incoming_mounts, &target)?;
    if demotion
        && incoming_mounts.iter().any(|mount| {
            mount["status"] == "active"
                && mount["destinationControllerNpub"].as_str() == Some(&target)
        })
    {
        return Err(CliError::InvalidInput(
            "revoke the active Mount before demoting its destination controller".to_owned(),
        ));
    }
    for mount in incoming_mounts
        .iter()
        .filter(|mount| !demotion && mount["status"] == "active")
    {
        let participants = mount["participantNpubs"].as_array().ok_or_else(|| {
            CliError::InvalidInput("Mount response omitted participant roster".to_owned())
        })?;
        if !participants
            .iter()
            .any(|participant| participant.as_str() == Some(&target))
        {
            continue;
        }
        let mount_id = required_json_string(mount, "id")?;
        let source_brain_id = required_json_string(mount, "sourceBrainId")?;
        let source_folder_id = required_json_string(mount, "sourceFolderId")?;
        let controller = required_json_string(mount, "destinationControllerNpub")?;
        let revoke_mount = controller == target;
        let managed_participants = mount["managedAccessParticipantNpubs"]
            .as_array()
            .ok_or_else(|| {
                CliError::InvalidInput("Mount response omitted managed access roster".to_owned())
            })?;
        let removed = if revoke_mount {
            managed_participants
                .iter()
                .map(|participant| {
                    participant.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                        CliError::InvalidInput("Mount participant was not a string".to_owned())
                    })
                })
                .collect::<Result<BTreeSet<_>, _>>()?
        } else if managed_participants
            .iter()
            .any(|participant| participant.as_str() == Some(&target))
        {
            BTreeSet::from([target.clone()])
        } else {
            BTreeSet::new()
        };
        let prepared = if removed.is_empty() {
            serde_json::json!({
                "newKeyVersion": 0,
                "grants": [],
                "reencryptedRecords": []
            })
        } else {
            let source_metadata = fetch_brain_metadata(env, args, &source_brain_id)?;
            prepare_folder_access_removals(
                env,
                args,
                &source_metadata,
                &source_brain_id,
                &source_folder_id,
                &removed,
            )?
        };
        mount_rotations.push(serde_json::json!({
            "mountId": mount_id,
            "revokeMount": revoke_mount,
            "newKeyVersion": prepared["newKeyVersion"],
            "grants": prepared["grants"],
            "reencryptedRecords": prepared["reencryptedRecords"],
        }));
    }

    let action = if demotion {
        AdminAccessAction::RemoveAdmin
    } else {
        AdminAccessAction::RemoveMember
    };
    let event = admin_access_change_event(env, &brain_id, action, None, Some(&target), None)?;
    let route = if demotion {
        format!("/v1/admin/brains/{brain_id}/roles/admin/{target}/rotate")
    } else {
        format!("/v1/admin/brains/{brain_id}/members/{target}")
    };
    let no_op = rotations.is_empty()
        && mount_rotations.is_empty()
        && !metadata.admins.contains(&target)
        && (demotion || !metadata.members.contains(&target))
        && (demotion
            || metadata
                .folders
                .iter()
                .all(|folder| !folder.access_user_ids.contains(&target)));
    let mutation = signed_json_request(
        env,
        args,
        "DELETE",
        &route,
        Some(serde_json::json!({
            "accessChangeEvent": event, "rotations": rotations, "mountRotations": mount_rotations
        })),
    );
    if let Err(CliError::HttpStatus { status, body }) = &mutation
        && (400..500).contains(status)
    {
        if demotion && *status == 404 {
            return Err(CliError::Unsupported("server does not support rotated admin demotion; upgrade the server before retrying".to_owned()));
        }
        if !demotion && body.contains("remove admin role before removing member") {
            return Err(CliError::Unsupported("server does not support complete admin removal; upgrade the server before retrying. Do not demote first".to_owned()));
        }
        if !demotion && body.contains("brain member does not exist") {
            return Err(CliError::Unsupported("server cannot safely repair or verify an already-removed identity; upgrade the server before retrying".to_owned()));
        }
        return Err(mutation.unwrap_err());
    }
    // The response is only transport evidence. Verify fresh authority, grants and exact revisions.
    let verified = verify_identity_removal(
        env,
        args,
        &brain_id,
        &target,
        demotion,
        &rotations,
        &mount_rotations,
    );
    match verified {
        Ok(mut receipt) => {
            receipt["outcome"] =
                serde_json::json!(if no_op { "alreadyComplete" } else { "changed" });
            if json {
                write_json(output, &receipt)?;
            } else {
                writeln!(
                    output,
                    "{}",
                    if demotion {
                        "Demotion complete"
                    } else {
                        "Removal complete"
                    }
                )?;
                for folder in receipt["folders"].as_array().into_iter().flatten() {
                    writeln!(
                        output,
                        "Folder {}: key version {} → {}",
                        folder["folderId"].as_str().unwrap_or("unknown"),
                        folder["oldKeyVersion"],
                        folder["newKeyVersion"]
                    )?;
                }
                for source in receipt["preservedSourceAccess"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    writeln!(
                        output,
                        "Independent source access remains: {}/{}",
                        source["brainId"].as_str().unwrap_or("unknown"),
                        source["folderId"].as_str().unwrap_or("unknown")
                    )?;
                }
                writeln!(output, "Earlier keys and copies cannot be recalled.")?;
            }
            Ok(())
        }
        Err(verification) => Err(CliError::InvalidInput(format!(
            "Result unknown: {}; verification: {verification}. Refresh before retrying. Earlier keys and copies cannot be recalled.",
            mutation
                .err()
                .map(|error| error.to_string())
                .unwrap_or_else(
                    || "request returned but exact completion was not confirmed".to_owned()
                )
        ))),
    }
}

fn verify_identity_removal(
    env: &CliEnvironment,
    args: &[String],
    brain_id: &str,
    target: &str,
    demotion: bool,
    rotations: &[serde_json::Value],
    mount_rotations: &[serde_json::Value],
) -> Result<serde_json::Value, CliError> {
    let metadata = fetch_brain_metadata(env, args, brain_id)?;
    let export = fetch_encrypted_export(env, &server_url_for_command(env, args)?, brain_id)?;
    ensure_snapshot_alignment(&metadata, &export)?;
    if metadata.admins.iter().any(|admin| admin == target)
        || export
            .access_state
            .admins
            .iter()
            .any(|admin| admin == target)
        || (!demotion
            && (metadata.members.iter().any(|member| member == target)
                || export
                    .access_state
                    .members
                    .iter()
                    .any(|member| member == target)
                || metadata
                    .folders
                    .iter()
                    .any(|folder| folder.access_user_ids.iter().any(|user| user == target))))
    {
        return Err(CliError::InvalidInput(
            "withdrawn authority remains".to_owned(),
        ));
    }
    for folder in &metadata.folders {
        let entitled =
            folder_required_recipients(&metadata, &folder.access, &folder.access_user_ids)?
                .iter()
                .any(|user| user == target);
        if (!demotion || !entitled)
            && export.key_grants.iter().any(|grant| {
                grant.folder_id == folder.id
                    && grant.key_version == folder.current_key_version
                    && grant.recipient_npub == target
            })
        {
            return Err(CliError::InvalidInput(format!(
                "Folder {} retains a target current grant",
                folder.id
            )));
        }
    }
    let mut receipts = Vec::new();
    verify_rotation_receipts(&metadata, &export, rotations, &mut receipts)?;
    let mounts = signed_json_request(
        env,
        args,
        "GET",
        &format!("/v1/brains/{brain_id}/mounts"),
        None,
    )?;
    let incoming = mounts["incoming"].as_array().ok_or_else(|| {
        CliError::InvalidInput("Mount verification omitted relationships".to_owned())
    })?;
    let mut preserved_source_access = verify_mount_sources(env, args, incoming, target)?;
    if !demotion
        && incoming.iter().any(|mount| {
            mount["status"] == "active"
                && mount["participantNpubs"]
                    .as_array()
                    .is_some_and(|users| users.iter().any(|user| user.as_str() == Some(target)))
        })
    {
        return Err(CliError::InvalidInput(
            "active Mount participation remains".to_owned(),
        ));
    }
    for rotation in mount_rotations {
        let mount = signed_json_request(
            env,
            args,
            "GET",
            &format!("/v1/mounts/{}", required_json_string(rotation, "mountId")?),
            None,
        )?;
        let source = required_json_string(&mount, "sourceBrainId")?;
        let mut source_rotation = rotation.clone();
        source_rotation["folderId"] = mount["sourceFolderId"].clone();
        let source_metadata = fetch_brain_metadata(env, args, &source)?;
        let source_export =
            fetch_encrypted_export(env, &server_url_for_command(env, args)?, &source)?;
        ensure_snapshot_alignment(&source_metadata, &source_export)?;
        let folder_id = required_json_string(&mount, "sourceFolderId")?;
        if let Some(folder) = source_metadata
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
            && folder_required_recipients(&source_metadata, &folder.access, &[])?
                .iter()
                .any(|recipient| recipient == target)
        {
            let preserved = serde_json::json!({"brainId": source, "folderId": folder_id, "currentKeyVersion": folder.current_key_version});
            if !preserved_source_access.contains(&preserved) {
                preserved_source_access.push(preserved);
            }
        }
        if rotation["newKeyVersion"].as_u64() == Some(0) {
            continue;
        }
        verify_rotation_receipts(
            &source_metadata,
            &source_export,
            &[source_rotation],
            &mut receipts,
        )?;
    }
    Ok(
        serde_json::json!({"state": "complete", "operation": if demotion { "demotion" } else { "removal" },
        "brainId": brain_id, "targetNpub": target, "folders": receipts, "preservedSourceAccess": preserved_source_access,
        "limitation": "Earlier keys and copies cannot be recalled."}),
    )
}

/// Source recipients are visible only to a source admin. Incomplete scope is a blocker,
/// including on a retry that appears complete from the destination roster alone.
fn verify_mount_sources(
    env: &CliEnvironment,
    args: &[String],
    incoming: &[serde_json::Value],
    target: &str,
) -> Result<Vec<serde_json::Value>, CliError> {
    // The existing Mount list endpoint caps each direction at 200 rows and has no cursor.
    // Hitting that bound cannot certify complete source coverage.
    if incoming.len() >= 200 {
        return Err(CliError::InvalidInput(
            "Mount list reached its coverage limit; complete source scope cannot be verified"
                .to_owned(),
        ));
    }
    let actor = load_signer(env)?.npub;
    let mut preserved = Vec::new();
    for mount in incoming.iter().filter(|mount| mount["status"] == "active") {
        let mount_id = required_json_string(mount, "id")?;
        let source = required_json_string(mount, "sourceBrainId")?;
        let folder_id = required_json_string(mount, "sourceFolderId")?;
        let metadata = fetch_brain_metadata(env, args, &source).map_err(|error| {
            CliError::InvalidInput(format!(
                "Mount {mount_id} source Brain {source} authority coverage is unresolved: {error}"
            ))
        })?;
        if !metadata.admins.contains(&actor) && metadata.owner_user_id.as_deref() != Some(&actor) {
            return Err(CliError::InvalidInput(format!(
                "Mount {mount_id} source grant coverage is unresolved; a source admin must prepare and verify this removal"
            )));
        }
        let export = fetch_encrypted_export(env, &server_url_for_command(env, args)?, &source)
            .map_err(|error| {
                CliError::InvalidInput(format!(
                    "Mount {mount_id} source Brain {source} grant coverage is unresolved: {error}"
                ))
            })?;
        ensure_snapshot_alignment(&metadata, &export)?;
        let folder = export
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
            .ok_or_else(|| {
                CliError::InvalidInput(format!("Mount {mount_id} source Folder is unresolved"))
            })?;
        let independent = verify_mount_source_target(mount, &metadata, &export, target)?;
        let participating = mount["participantNpubs"]
            .as_array()
            .is_some_and(|users| users.iter().any(|user| user.as_str() == Some(target)));
        if independent && !participating {
            let access = serde_json::json!({"brainId": source, "folderId": folder_id, "currentKeyVersion": folder.current_key_version});
            if !preserved.contains(&access) {
                preserved.push(access);
            }
        }
    }
    Ok(preserved)
}

/// Explicit source access is a projection that can also come from a Mount. Until
/// provenance is available on the read surface, only roster/owner standing can
/// prove independent access; ordinary Mount-managed withdrawal remains supported.
fn verify_mount_source_target(
    mount: &serde_json::Value,
    metadata: &BrainMetadataView,
    export: &CliEncryptedBrainExport,
    target: &str,
) -> Result<bool, CliError> {
    let mount_id = required_json_string(mount, "id")?;
    let folder_id = required_json_string(mount, "sourceFolderId")?;
    let folder = metadata
        .folders
        .iter()
        .find(|folder| folder.id == folder_id)
        .ok_or_else(|| {
            CliError::InvalidInput(format!(
                "Mount {mount_id} source Folder {folder_id} is unresolved"
            ))
        })?;
    let participants = mount["participantNpubs"].as_array().ok_or_else(|| {
        CliError::InvalidInput(format!("Mount {mount_id} participant roster is unresolved"))
    })?;
    let managed = mount["managedAccessParticipantNpubs"]
        .as_array()
        .ok_or_else(|| {
            CliError::InvalidInput(format!(
                "Mount {mount_id} managed source access roster is unresolved"
            ))
        })?;
    let independent = folder_required_recipients(metadata, &folder.access, &[])?
        .iter()
        .any(|recipient| recipient == target);
    let entitled = folder_required_recipients(metadata, &folder.access, &folder.access_user_ids)?
        .iter()
        .any(|recipient| recipient == target);
    let held = export.key_grants.iter().any(|grant| {
        grant.folder_id == folder_id
            && grant.key_version == folder.current_key_version
            && grant.recipient_npub == target
    });
    let managed_participant = participants
        .iter()
        .any(|user| user.as_str() == Some(target))
        && managed.iter().any(|user| user.as_str() == Some(target));
    if (held || entitled) && !independent && !managed_participant {
        return Err(CliError::InvalidInput(format!(
            "Mount {mount_id} source Folder {folder_id} retains target access whose independent source provenance cannot be verified; source-authority repair is required before removal"
        )));
    }
    Ok(independent)
}

fn ensure_snapshot_alignment(
    metadata: &BrainMetadataView,
    export: &CliEncryptedBrainExport,
) -> Result<(), CliError> {
    let metadata_folders = metadata
        .folders
        .iter()
        .map(|folder| (&folder.id, folder.current_key_version))
        .collect::<BTreeSet<_>>();
    let export_folders = export
        .folders
        .iter()
        .map(|folder| (&folder.id, folder.current_key_version))
        .collect::<BTreeSet<_>>();
    if metadata.brain_id != export.brain.id
        || metadata_folders != export_folders
        || metadata.members.iter().collect::<BTreeSet<_>>()
            != export.access_state.members.iter().collect::<BTreeSet<_>>()
        || metadata.admins.iter().collect::<BTreeSet<_>>()
            != export.access_state.admins.iter().collect::<BTreeSet<_>>()
    {
        return Err(CliError::InvalidInput(
            "Brain authority or Folder state changed between reads; refresh before retrying"
                .to_owned(),
        ));
    }
    Ok(())
}

fn verify_rotation_receipts(
    metadata: &BrainMetadataView,
    export: &CliEncryptedBrainExport,
    rotations: &[serde_json::Value],
    receipts: &mut Vec<serde_json::Value>,
) -> Result<(), CliError> {
    for rotation in rotations {
        let folder_id = required_json_string(rotation, "folderId")?;
        let version = rotation["newKeyVersion"]
            .as_u64()
            .ok_or_else(|| CliError::InvalidInput("rotation omitted version".to_owned()))?;
        let folder = metadata
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
            .ok_or_else(|| CliError::NotFound(format!("Folder {folder_id}")))?;
        if u64::from(folder.current_key_version) != version
            || !export.folders.iter().any(|folder| {
                folder.id == folder_id && u64::from(folder.current_key_version) == version
            })
        {
            return Err(CliError::InvalidInput(format!(
                "Folder {folder_id} key version differs from the exact plan"
            )));
        }
        let expected = rotation["grants"]
            .as_array()
            .ok_or_else(|| CliError::InvalidInput("rotation omitted grants".to_owned()))?;
        let planned = expected
            .iter()
            .map(|grant| required_json_string(grant, "recipientNpub"))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let required =
            folder_required_recipients(metadata, &folder.access, &folder.access_user_ids)?
                .into_iter()
                .collect::<BTreeSet<_>>();
        if planned != required {
            return Err(CliError::InvalidInput(format!(
                "Folder {folder_id} authorized recipients changed from the exact plan"
            )));
        }
        let actual = export
            .key_grants
            .iter()
            .filter(|grant| grant.folder_id == folder_id && u64::from(grant.key_version) == version)
            .map(|grant| grant.recipient_npub.clone())
            .collect::<BTreeSet<_>>();
        if planned != actual
            || expected.iter().any(|prepared| {
                !export.key_grants.iter().any(|grant| {
                    grant.folder_id == folder_id
                        && u64::from(grant.key_version) == version
                        && prepared["recipientNpub"].as_str() == Some(grant.recipient_npub.as_str())
                        && prepared["wrappedEventJson"].as_str()
                            == Some(grant.wrapped_event_json.as_str())
                })
            })
        {
            return Err(CliError::InvalidInput(format!(
                "Folder {folder_id} current grants differ from the exact plan"
            )));
        }
        let records = rotation["reencryptedRecords"]
            .as_array()
            .ok_or_else(|| CliError::InvalidInput("rotation omitted revisions".to_owned()))?;
        let live = export
            .objects
            .iter()
            .filter(|object| object.folder_id == folder_id && !object.deleted)
            .collect::<Vec<_>>();
        if live.len() != records.len()
            || records.iter().any(|record| {
                !live.iter().any(|object| {
                    Some(object.object_id.as_str()) == record["objectId"].as_str()
                        && Some(object.revision)
                            == record["baseRevision"]
                                .as_u64()
                                .and_then(|revision| revision.checked_add(1))
                        && object.payload_json.as_deref().is_some_and(|payload| {
                            finite_brain_core::decode_sync_payload(payload)
                                .ciphertext_or_raw(payload)
                                == record["ciphertext"].as_str().unwrap_or("INVALID")
                        })
                })
            })
        {
            return Err(CliError::InvalidInput(format!(
                "Folder {folder_id} live revisions differ from the exact plan"
            )));
        }
        receipts.push(serde_json::json!({"brainId": export.brain.id, "folderId": folder_id, "oldKeyVersion": version - 1, "newKeyVersion": version}));
    }
    Ok(())
}

pub(crate) fn metadata_after_member_removal(
    metadata: &BrainMetadataView,
    target: &str,
) -> BrainMetadataView {
    let mut updated = metadata.clone();
    updated.members.retain(|member| member != target);
    updated.admins.retain(|admin| admin != target);
    updated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fin159_mount_source_projection_does_not_prove_independent_access() {
        let mut metadata: BrainMetadataView = serde_json::from_value(serde_json::json!({
            "brainId": "source", "kind": "organization", "name": "Source", "ownerUserId": null,
            "members": ["target"], "admins": ["survivor"],
            "folders": [{"id": "private", "name": "Private", "access": "restricted",
                "path": "Private", "currentKeyVersion": 1, "accessUserIds": ["target"]}]
        }))
        .unwrap();
        let export: CliEncryptedBrainExport = serde_json::from_value(serde_json::json!({
            "brain": {"id": "source", "kind": "organization", "name": "Source", "ownerUserId": null},
            "folders": [{"id": "private", "path": "Private", "access": "restricted", "currentKeyVersion": 1, "accessible": true}],
            "keyGrants": [{"folderId": "private", "keyVersion": 1, "issuerNpub": "survivor", "recipientNpub": "target", "wrappedEventJson": "synthetic"}],
            "accessState": {"members": ["target"], "admins": ["survivor"]}
        })).unwrap();
        let mut mount = serde_json::json!({"id": "source-mount", "sourceFolderId": "private",
            "participantNpubs": [], "managedAccessParticipantNpubs": []});
        let error = verify_mount_source_target(&mount, &metadata, &export, "target")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("source-mount")
                && error.contains("private")
                && error.contains("provenance")
        );
        mount["participantNpubs"] = serde_json::json!(["target"]);
        // A direct-access projection remains ambiguous even while listed as a participant.
        assert!(verify_mount_source_target(&mount, &metadata, &export, "target").is_err());
        mount["managedAccessParticipantNpubs"] = serde_json::json!(["target"]);
        assert!(!verify_mount_source_target(&mount, &metadata, &export, "target").unwrap());
        mount["participantNpubs"] = serde_json::json!([]);
        metadata.admins.push("target".to_owned());
        assert!(verify_mount_source_target(&mount, &metadata, &export, "target").unwrap());
        metadata.admins.retain(|admin| admin != "target");
        metadata.folders[0].access = "all_members".to_owned();
        assert!(verify_mount_source_target(&mount, &metadata, &export, "target").unwrap());
        metadata.kind = "personal".to_owned();
        metadata.owner_user_id = Some("target".to_owned());
        metadata.folders[0].access = "restricted".to_owned();
        assert!(verify_mount_source_target(&mount, &metadata, &export, "target").unwrap());
    }

    #[test]
    fn fin159_completion_requires_exact_grants_and_live_revisions_not_just_key_version() {
        let metadata: BrainMetadataView = serde_json::from_value(serde_json::json!({
            "brainId": "acme", "kind": "organization", "name": "Acme", "ownerUserId": null,
            "members": ["survivor"], "admins": ["survivor"],
            "folders": [{"id": "private", "name": "Private", "role": "folder", "access": "restricted",
                "parentFolderId": null, "path": "Private", "currentKeyVersion": 2, "accessUserIds": []}]
        })).unwrap();
        let mut export: CliEncryptedBrainExport = serde_json::from_value(serde_json::json!({
            "brain": {"id": "acme", "kind": "organization", "name": "Acme", "ownerUserId": null},
            "folders": [{"id": "private", "path": "Private", "access": "restricted", "currentKeyVersion": 2, "accessible": true}],
            "objects": [{"folderId": "private", "objectId": "obj_000000000001", "payloadJson": "old-ciphertext", "revision": 2, "updatedAt": "synthetic", "deleted": false, "opaque": false}],
            "keyGrants": [{"folderId": "private", "keyVersion": 2, "issuerNpub": "survivor", "recipientNpub": "survivor", "wrappedEventJson": "synthetic"}],
            "accessState": {"members": ["survivor"], "admins": ["survivor"]}
        })).unwrap();
        let plan = vec![
            serde_json::json!({"folderId": "private", "newKeyVersion": 2,
            "grants": [{"recipientNpub": "survivor", "wrappedEventJson": "synthetic"}],
            "reencryptedRecords": [{"objectId": "obj_000000000001", "baseRevision": 1, "ciphertext": "new-ciphertext"}]}),
        ];
        assert!(verify_rotation_receipts(&metadata, &export, &plan, &mut vec![]).is_err());
        export.objects[0].payload_json = Some("new-ciphertext".to_owned());
        let mut receipts = vec![];
        verify_rotation_receipts(&metadata, &export, &plan, &mut receipts).unwrap();
        assert_eq!(receipts[0]["oldKeyVersion"], 1);
        export.key_grants[0].wrapped_event_json = "different-wrap".to_owned();
        assert!(verify_rotation_receipts(&metadata, &export, &plan, &mut vec![]).is_err());
        export.key_grants[0].wrapped_event_json = "synthetic".to_owned();
        export.key_grants[0].recipient_npub = "removed".to_owned();
        assert!(verify_rotation_receipts(&metadata, &export, &plan, &mut vec![]).is_err());
        export.key_grants[0].recipient_npub = "survivor".to_owned();
        export.objects[0].revision = 3;
        assert!(verify_rotation_receipts(&metadata, &export, &plan, &mut vec![]).is_err());
    }
}
