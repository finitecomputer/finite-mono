use crate::*;

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
    let brain_id = BrainId::new(brain_id)?;
    let target_identity = resolve_and_record_identity(&state, &request.target_npub).await?;
    let target = UserId::new(target_identity.npub.clone())?;
    let (event, payload) = validate_admin_access_change_value(
        request.access_change_event,
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
    }
    let mut grants_by_folder =
        std::collections::BTreeMap::<String, &CollaborationGrantRequest>::new();
    for grant in &request.grants {
        if grants_by_folder
            .insert(grant.folder_id.clone(), grant)
            .is_some()
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "collaboration request contains duplicate grant recipients",
            ));
        }
    }

    let mut outcomes = Vec::new();
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
                outcomes.push(CollaborationFolderReceipt {
                    folder_id: folder.id.to_string(),
                    path: folder.path.to_string(),
                    expected_key_version: folder.current_key_version,
                    outcome: CollaborationFolderOutcome::Failed,
                    reason: Some("folderMissingFromSnapshot".to_owned()),
                    retryable: true,
                });
                continue;
            };
            if item.key_version != folder.current_key_version {
                outcomes.push(CollaborationFolderReceipt {
                    folder_id: folder.id.to_string(),
                    path: folder.path.to_string(),
                    expected_key_version: item.key_version,
                    outcome: CollaborationFolderOutcome::StaleVersion,
                    reason: Some("currentKeyVersionChanged".to_owned()),
                    retryable: true,
                });
                continue;
            }
            let already_ready = stored.grants.iter().any(|existing| {
                existing.folder_id == folder.id
                    && existing.key_version == folder.current_key_version
                    && existing.recipient_npub == target
            });
            if already_ready {
                outcomes.push(CollaborationFolderReceipt {
                    folder_id: folder.id.to_string(),
                    path: folder.path.to_string(),
                    expected_key_version: item.key_version,
                    outcome: CollaborationFolderOutcome::AlreadyReady,
                    reason: None,
                    retryable: false,
                });
                continue;
            }
            let Some(grant_request) = grants_by_folder.get(folder.id.as_str()) else {
                outcomes.push(CollaborationFolderReceipt {
                    folder_id: folder.id.to_string(),
                    path: folder.path.to_string(),
                    expected_key_version: item.key_version,
                    outcome: CollaborationFolderOutcome::MissingSourceKey,
                    reason: Some("sourceKeyUnavailable".to_owned()),
                    retryable: true,
                });
                continue;
            };
            if grant_request.grant.key_version != item.key_version {
                outcomes.push(CollaborationFolderReceipt {
                    folder_id: folder.id.to_string(),
                    path: folder.path.to_string(),
                    expected_key_version: item.key_version,
                    outcome: CollaborationFolderOutcome::StaleVersion,
                    reason: Some("submittedGrantVersionChanged".to_owned()),
                    retryable: true,
                });
                continue;
            }
            let grant = grant_request_to_metadata(
                &grant_request.grant,
                &folder.id,
                &actor,
                Some(event.as_json()),
                &server_timestamp(&state),
            )?;
            outcomes.push(CollaborationFolderReceipt {
                folder_id: folder.id.to_string(),
                path: folder.path.to_string(),
                expected_key_version: item.key_version,
                outcome: if already_ready {
                    CollaborationFolderOutcome::AlreadyReady
                } else {
                    CollaborationFolderOutcome::Granted
                },
                reason: None,
                retryable: false,
            });
            accepted.push((grant.clone(), folder_key_grant_sync_record(&grant)?));
        }

        store.ensure_organization_admin_with_grants(
            &brain_id,
            &target,
            &accepted,
            Some(&admin_record),
        )?;
        let final_stored = store.load_brain(&brain_id)?;
        let postcondition = final_stored.brain.admins.contains(&target)
            && final_stored
                .grants
                .iter()
                .filter(|grant| {
                    grant.recipient_npub == target
                        && final_stored.brain.folders.iter().any(|folder| {
                            folder.id == grant.folder_id
                                && folder.current_key_version == grant.key_version
                        })
                })
                .count()
                >= final_stored.brain.folders.len();
        if postcondition {
            for outcome in &mut outcomes {
                if outcome.outcome == CollaborationFolderOutcome::Granted {
                    outcome.retryable = false;
                }
            }
        }
    }

    let ready_count = outcomes
        .iter()
        .filter(|outcome| {
            matches!(
                outcome.outcome,
                CollaborationFolderOutcome::Granted | CollaborationFolderOutcome::AlreadyReady
            )
        })
        .count();
    let all_ready = outcomes.iter().all(|outcome| {
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
        brain_role: "admin".to_owned(),
        total_count: outcomes.len(),
        ready_count,
        folders: outcomes,
        retryable,
    }))
}
