use crate::*;

/// Explicit fanout limits keep collaboration requests and their transaction
/// control-record batches bounded by the Brain capacity envelope.
pub(crate) const MAX_COLLABORATION_FOLDERS: usize = 1_000;
pub(crate) const MAX_COLLABORATION_GRANTS: usize = 1_000;

fn current_key_holders(
    state: &ServerState,
    stored: &StoredBrain,
    folder_id: &FolderId,
    key_version: u32,
) -> Result<Vec<CollaborationKeyHolder>, ApiError> {
    let mut npubs = stored
        .grants
        .iter()
        .filter(|grant| grant.folder_id == *folder_id && grant.key_version == key_version)
        .map(|grant| grant.issuer_npub.clone())
        .collect::<Vec<_>>();
    npubs.sort();
    npubs.dedup();
    let aliases = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_identity_aliases(&npubs)?
    };
    Ok(npubs
        .into_iter()
        .map(|npub| CollaborationKeyHolder {
            email: aliases
                .iter()
                .find(|alias| alias.npub == npub)
                .and_then(|alias| alias.preferred_nip05.clone()),
            npub: npub.to_string(),
        })
        .collect())
}

/// Converge one Organization Brain collaborator to administrator plus the
/// current encrypted Folder Key Grants supplied by the trusted client.
pub(crate) async fn ensure_organization_admin_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    method: Method,
    OriginalUri(uri): OriginalUri,
    AxumPath(brain_id): AxumPath<String>,
    body: Bytes,
) -> Result<Json<EnsureOrganizationAdminResponse>, ApiError> {
    let actor = validate_request_auth(&state, &headers, &method, &uri, Some(&body))?;
    let request: EnsureOrganizationAdminRequest = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid JSON request body"))?;
    if request.folders.len() > MAX_COLLABORATION_FOLDERS {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("collaboration Folder snapshot exceeds {MAX_COLLABORATION_FOLDERS} entries"),
        ));
    }
    if request.grants.len() > MAX_COLLABORATION_GRANTS {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("collaboration grants exceed {MAX_COLLABORATION_GRANTS} entries"),
        ));
    }

    let brain_id = BrainId::new(brain_id)?;
    let target_identity = resolve_and_record_identity(&state, &request.target_npub).await?;
    let target = UserId::new(target_identity.npub.clone())?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event.clone(),
        &brain_id,
        &actor,
        AdminAccessAction::AddAdmin,
        None,
        Some(target.as_str()),
        None,
    )?;
    let admin_record = admin_access_change_sync_record(&actor, &event, &payload)?;

    let snapshot = request
        .folders
        .iter()
        .map(|folder| (folder.folder_id.clone(), folder))
        .collect::<std::collections::BTreeMap<_, _>>();
    if snapshot.len() != request.folders.len() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "collaboration Folder snapshot contains duplicate identities",
        ));
    }
    for folder in &request.folders {
        FolderId::new(folder.folder_id.clone())?;
        SafeRelativePath::new("collaboration_folder_path", folder.path.clone())?;
    }
    let mut grants_by_folder =
        std::collections::BTreeMap::<String, &CollaborationGrantRequest>::new();
    for grant in &request.grants {
        if grant.grant.recipient_npub != target.as_str() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "collaboration grant recipient does not match target",
            ));
        }
        if !snapshot.contains_key(&grant.folder_id) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "collaboration grant Folder is not in the requested snapshot",
            ));
        }
        if grants_by_folder
            .insert(grant.folder_id.clone(), grant)
            .is_some()
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "collaboration request contains duplicate grant recipients",
            ));
        }
        let folder_id = FolderId::new(grant.folder_id.clone())?;
        validate_admin_access_change_value(
            grant.access_change_event.clone(),
            &brain_id,
            &actor,
            AdminAccessAction::GrantFolderAccess,
            Some(&folder_id),
            Some(target.as_str()),
            Some(grant.grant.key_version),
        )?;
    }

    let mut provisional = Vec::new();
    let mut accepted = Vec::new();
    {
        let mut store = state.store.lock().map_err(lock_error)?;
        let stored = store.load_brain(&brain_id)?;
        ensure_brain_admin(&stored, &actor)?;
        if stored.brain.kind != BrainKind::Organization {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Organization Brain collaboration is not available for Personal Brains",
            ));
        }
        for folder in &stored.brain.folders {
            let Some(item) = snapshot.get(folder.id.as_str()) else {
                provisional.push(CollaborationFolderReceipt {
                    folder_id: folder.id.to_string(),
                    path: folder.path.to_string(),
                    expected_key_version: folder.current_key_version,
                    outcome: CollaborationFolderOutcome::Failed,
                    reason: Some("folderMissingFromSnapshot".to_owned()),
                    retryable: true,
                    key_holders: Vec::new(),
                });
                continue;
            };
            if item.key_version != folder.current_key_version {
                provisional.push(CollaborationFolderReceipt {
                    folder_id: folder.id.to_string(),
                    path: folder.path.to_string(),
                    expected_key_version: item.key_version,
                    outcome: CollaborationFolderOutcome::StaleVersion,
                    reason: Some("currentKeyVersionChanged".to_owned()),
                    retryable: true,
                    key_holders: Vec::new(),
                });
                continue;
            }
            let already_ready = stored.grants.iter().any(|existing| {
                existing.folder_id == folder.id
                    && existing.key_version == folder.current_key_version
                    && existing.recipient_npub == target
            });
            if already_ready {
                provisional.push(CollaborationFolderReceipt {
                    folder_id: folder.id.to_string(),
                    path: folder.path.to_string(),
                    expected_key_version: item.key_version,
                    outcome: CollaborationFolderOutcome::AlreadyReady,
                    reason: None,
                    retryable: false,
                    key_holders: Vec::new(),
                });
                continue;
            }
            let Some(grant_request) = grants_by_folder.get(folder.id.as_str()) else {
                provisional.push(CollaborationFolderReceipt {
                    folder_id: folder.id.to_string(),
                    path: folder.path.to_string(),
                    expected_key_version: item.key_version,
                    outcome: CollaborationFolderOutcome::MissingSourceKey,
                    reason: Some("sourceKeyUnavailable".to_owned()),
                    retryable: true,
                    key_holders: Vec::new(),
                });
                continue;
            };
            if grant_request.grant.key_version != item.key_version {
                provisional.push(CollaborationFolderReceipt {
                    folder_id: folder.id.to_string(),
                    path: folder.path.to_string(),
                    expected_key_version: item.key_version,
                    outcome: CollaborationFolderOutcome::StaleVersion,
                    reason: Some("submittedGrantVersionChanged".to_owned()),
                    retryable: true,
                    key_holders: Vec::new(),
                });
                continue;
            }
            let (folder_event, folder_payload) = validate_admin_access_change_value(
                grant_request.access_change_event.clone(),
                &brain_id,
                &actor,
                AdminAccessAction::GrantFolderAccess,
                Some(&folder.id),
                Some(target.as_str()),
                Some(item.key_version),
            )?;
            let grant = grant_request_to_metadata(
                &grant_request.grant,
                &folder.id,
                &actor,
                Some(folder_event.as_json()),
                &server_timestamp(&state),
            )?;
            provisional.push(CollaborationFolderReceipt {
                folder_id: folder.id.to_string(),
                path: folder.path.to_string(),
                expected_key_version: item.key_version,
                outcome: CollaborationFolderOutcome::Granted,
                reason: None,
                retryable: false,
                key_holders: Vec::new(),
            });
            accepted.push((
                grant.clone(),
                folder_key_grant_sync_record(&grant)?,
                admin_access_change_sync_record(&actor, &folder_event, &folder_payload)?,
            ));
        }
        store.ensure_organization_admin_with_grants(
            &brain_id,
            &target,
            &accepted,
            Some(&admin_record),
        )?;
    }

    // Never infer completion from the submitted batch. Re-load the server's
    // authoritative role, current Folder inventory, and current grants so
    // concurrent creation, deletion, or rotation is visible as drift.
    let final_stored = {
        let store = state.store.lock().map_err(lock_error)?;
        store.load_brain(&brain_id)?
    };
    let final_is_member = final_stored
        .brain
        .members
        .iter()
        .any(|member| member.user_id == target);
    let final_is_admin = final_stored.brain.admins.contains(&target);
    let mut authoritative = std::collections::BTreeMap::new();
    for snapshot_folder in &request.folders {
        let Some(folder) = final_stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id.as_str() == snapshot_folder.folder_id)
        else {
            authoritative.insert(
                snapshot_folder.folder_id.clone(),
                CollaborationFolderReceipt {
                    folder_id: snapshot_folder.folder_id.clone(),
                    path: snapshot_folder.path.clone(),
                    expected_key_version: snapshot_folder.key_version,
                    outcome: CollaborationFolderOutcome::Failed,
                    reason: Some("folderRemovedSinceSnapshot".to_owned()),
                    retryable: true,
                    key_holders: Vec::new(),
                },
            );
            continue;
        };
        if folder.current_key_version != snapshot_folder.key_version {
            authoritative.insert(
                snapshot_folder.folder_id.clone(),
                CollaborationFolderReceipt {
                    folder_id: folder.id.to_string(),
                    path: folder.path.to_string(),
                    expected_key_version: snapshot_folder.key_version,
                    outcome: CollaborationFolderOutcome::StaleVersion,
                    reason: Some("currentKeyVersionChanged".to_owned()),
                    retryable: true,
                    key_holders: current_key_holders(
                        &state,
                        &final_stored,
                        &folder.id,
                        folder.current_key_version,
                    )?,
                },
            );
            continue;
        }
        let ready = final_stored.grants.iter().any(|grant| {
            grant.folder_id == folder.id
                && grant.key_version == folder.current_key_version
                && grant.recipient_npub == target
        });
        let was_already_ready = provisional.iter().any(|outcome| {
            outcome.folder_id == folder.id.to_string()
                && outcome.outcome == CollaborationFolderOutcome::AlreadyReady
        });
        authoritative.insert(
            snapshot_folder.folder_id.clone(),
            CollaborationFolderReceipt {
                folder_id: folder.id.to_string(),
                path: folder.path.to_string(),
                expected_key_version: snapshot_folder.key_version,
                outcome: if ready {
                    if was_already_ready {
                        CollaborationFolderOutcome::AlreadyReady
                    } else {
                        CollaborationFolderOutcome::Granted
                    }
                } else {
                    provisional
                        .iter()
                        .find(|outcome| outcome.folder_id == folder.id.to_string())
                        .map(|outcome| outcome.outcome)
                        .unwrap_or(CollaborationFolderOutcome::Failed)
                },
                reason: if ready {
                    None
                } else {
                    provisional
                        .iter()
                        .find(|outcome| outcome.folder_id == folder.id.to_string())
                        .and_then(|outcome| outcome.reason.clone())
                        .or_else(|| Some("postconditionMissingCurrentGrant".to_owned()))
                },
                retryable: !ready,
                key_holders: if ready {
                    Vec::new()
                } else {
                    current_key_holders(
                        &state,
                        &final_stored,
                        &folder.id,
                        folder.current_key_version,
                    )?
                },
            },
        );
    }
    for folder in &final_stored.brain.folders {
        if !authoritative.contains_key(folder.id.as_str()) {
            authoritative.insert(
                folder.id.to_string(),
                CollaborationFolderReceipt {
                    folder_id: folder.id.to_string(),
                    path: folder.path.to_string(),
                    expected_key_version: folder.current_key_version,
                    outcome: CollaborationFolderOutcome::Failed,
                    reason: Some("folderAddedSinceSnapshot".to_owned()),
                    retryable: true,
                    key_holders: current_key_holders(
                        &state,
                        &final_stored,
                        &folder.id,
                        folder.current_key_version,
                    )?,
                },
            );
        }
    }
    let folders = authoritative.into_values().collect::<Vec<_>>();
    let ready_count = folders
        .iter()
        .filter(|outcome| {
            matches!(
                outcome.outcome,
                CollaborationFolderOutcome::Granted | CollaborationFolderOutcome::AlreadyReady
            )
        })
        .count();
    let all_ready = final_is_member
        && final_is_admin
        && folders.len() == request.folders.len()
        && folders.iter().all(|outcome| {
            matches!(
                outcome.outcome,
                CollaborationFolderOutcome::Granted | CollaborationFolderOutcome::AlreadyReady
            )
        });
    let state = if all_ready {
        CollaborationReceiptState::Complete
    } else {
        CollaborationReceiptState::Partial
    };
    let retryable = state != CollaborationReceiptState::Complete;
    Ok(Json(EnsureOrganizationAdminResponse {
        brain_id: brain_id.to_string(),
        target_npub: target.to_string(),
        state,
        brain_role: if final_is_admin {
            "admin".to_owned()
        } else {
            "unknown".to_owned()
        },
        total_count: folders.len(),
        ready_count,
        folders,
        retryable,
    }))
}
