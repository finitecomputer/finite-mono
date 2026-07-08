use crate::*;

pub(crate) fn sync_record_response(record: StoredSyncRecord) -> SyncRecordResponse {
    SyncRecordResponse {
        sequence: record.sequence,
        record_event_id: record.record_event_id,
        record_type: match record.record_type {
            SyncRecordType::FolderObjectRevision => "folder_object_revision",
            SyncRecordType::FolderObjectTombstone => "folder_object_tombstone",
            SyncRecordType::FolderKeyGrant => "folder_key_grant",
            SyncRecordType::VaultAdminAccessChange => "vault_admin_access_change",
        }
        .to_owned(),
        folder_id: record.folder_id.map(|folder_id| folder_id.to_string()),
        object_id: record
            .object_id
            .map(|object_id| object_id.as_str().to_owned()),
        revision: record.revision,
        actor_npub: record.actor_npub.to_string(),
        client_created_at: record.client_created_at,
        payload_json: record.payload_json,
        record_event_kind: record.record_event_kind,
    }
}

pub(crate) fn metadata_response(stored: StoredVault) -> VaultMetadataResponse {
    metadata_response_with_mounts(stored, Vec::new())
}

pub(crate) fn metadata_response_with_mounts(
    stored: StoredVault,
    mounted_folders: Vec<MountedFolderProjection>,
) -> VaultMetadataResponse {
    let folder_access = stored.folder_access;
    let setup_incomplete = stored.setup_incomplete_folder_ids;
    VaultMetadataResponse {
        vault_id: stored.vault.id.to_string(),
        kind: stored.vault.kind,
        name: stored.vault.name.to_string(),
        owner_user_id: stored.vault.owner_user_id.map(|owner| owner.to_string()),
        members: stored
            .vault
            .members
            .iter()
            .map(|member| member.user_id.to_string())
            .collect(),
        admins: stored
            .vault
            .admins
            .iter()
            .map(ToString::to_string)
            .collect(),
        identities: Vec::new(),
        folders: stored
            .vault
            .folders
            .iter()
            .map(|folder| FolderMetadataResponse {
                id: folder.id.to_string(),
                name: folder.name.to_string(),
                role: folder.role,
                access: folder.access,
                parent_folder_id: folder.parent_folder_id.as_ref().map(ToString::to_string),
                path: folder.path.to_string(),
                shared_folder_source: folder.shared_folder_source,
                access_user_ids: folder_access
                    .get(&folder.id)
                    .map(|users| users.iter().map(ToString::to_string).collect())
                    .unwrap_or_default(),
                current_key_version: folder.current_key_version,
                setup_incomplete: setup_incomplete.contains(&folder.id),
            })
            .collect(),
        mounted_folders: mounted_folder_responses(mounted_folders),
        grant_count: stored.grants.len(),
    }
}

pub(crate) fn visible_vaults_response(vaults: Vec<VisibleVault>) -> VisibleVaultsResponse {
    VisibleVaultsResponse {
        vaults: vaults
            .into_iter()
            .map(|vault| VisibleVaultResponse {
                vault_id: vault.id.to_string(),
                kind: vault.kind,
                name: vault.name,
                role: match vault.role {
                    VisibleVaultRole::Owner => "owner",
                    VisibleVaultRole::Admin => "admin",
                    VisibleVaultRole::Member => "member",
                    VisibleVaultRole::Invited => "invited",
                }
                .to_owned(),
                invite_code: vault.invite_code,
            })
            .collect(),
    }
}

pub(crate) fn vault_invitation_response(
    invitation: StoredVaultInvitation,
) -> VaultInvitationResponse {
    VaultInvitationResponse {
        id: invitation.id,
        vault_id: invitation.vault_id.to_string(),
        target_kind: invitation.target_kind.as_str().to_owned(),
        user_id: invitation.user_id.map(|user_id| user_id.to_string()),
        invited_email: invitation.invited_email,
        invite_unwrap_npub: invitation.invite_unwrap_npub.map(|npub| npub.to_string()),
        bootstrap_payload_hash: invitation.bootstrap_payload_hash,
        bootstrap_wrapped_event_json: invitation.bootstrap_wrapped_event_json,
        bootstrap_authorization_event_json: invitation.bootstrap_authorization_event_json,
        bootstrap_scope: invitation
            .bootstrap_scope
            .into_iter()
            .map(|scope| EmailInviteBootstrapScopeResponse {
                folder_id: scope.folder_id.to_string(),
                access: scope.access,
                key_version: scope.key_version,
            })
            .collect(),
        claimed_by_npub: invitation.claimed_by_npub.map(|npub| npub.to_string()),
        identities: Vec::new(),
        status: link_status_str(invitation.status).to_owned(),
        invite_code: invitation.invite_code,
        accept_path: invitation.accept_path,
        public_instructions_path: String::new(),
        public_instructions_url: None,
        delivery_status: None,
        initial_folder_access: invitation
            .initial_folder_access
            .into_iter()
            .map(|folder_id| folder_id.to_string())
            .collect(),
        expires_at: invitation.expires_at,
        created_at: invitation.created_at,
        updated_at: invitation.updated_at,
        accepted_at: invitation.accepted_at,
        duplicate_accept: invitation.duplicate_accept,
    }
}

pub(crate) fn share_link_response(share_link: StoredShareLink) -> ShareLinkResponse {
    ShareLinkResponse {
        id: share_link.id,
        vault_id: share_link.vault_id.to_string(),
        folder_id: share_link.folder_id.to_string(),
        recipient_npub: share_link.recipient_npub.to_string(),
        created_by_npub: share_link.created_by_npub.to_string(),
        identities: Vec::new(),
        status: link_status_str(share_link.status).to_owned(),
        accept_path: share_link.accept_path,
        expires_at: share_link.expires_at,
        created_at: share_link.created_at,
        updated_at: share_link.updated_at,
        accepted_at: share_link.accepted_at,
        grant_id: share_link.folder_key_grant.id,
        create_personal_mount: share_link.create_personal_mount,
        personal_mount_id: share_link.personal_mount_id,
        duplicate_accept: share_link.duplicate_accept,
    }
}

pub(crate) fn shared_folder_invitation_response(
    invitation: StoredSharedFolderInvitation,
) -> SharedFolderInvitationResponse {
    SharedFolderInvitationResponse {
        id: invitation.id,
        source_vault_id: invitation.source_vault_id.to_string(),
        source_folder_id: invitation.source_folder_id.to_string(),
        destination_vault_id: invitation.destination_vault_id.to_string(),
        destination_admin_npub: invitation.destination_admin_npub.to_string(),
        created_by_npub: invitation.created_by_npub.to_string(),
        identities: Vec::new(),
        status: link_status_str(invitation.status).to_owned(),
        current_key_version: invitation.current_key_version,
        accept_path: invitation.accept_path,
        created_at: invitation.created_at,
        updated_at: invitation.updated_at,
        accepted_at: invitation.accepted_at,
        grant_id: invitation.folder_key_grant.id,
        duplicate_accept: invitation.duplicate_accept,
    }
}

pub(crate) fn shared_folder_connection_response(
    connection: StoredSharedFolderConnection,
) -> SharedFolderConnectionResponse {
    SharedFolderConnectionResponse {
        id: connection.id,
        source_vault_id: connection.source_vault_id.to_string(),
        source_folder_id: connection.source_folder_id.to_string(),
        destination_vault_id: connection.destination_vault_id.to_string(),
        destination_admin_npub: connection.destination_admin_npub.to_string(),
        identities: Vec::new(),
        status: match connection.status {
            SharedFolderConnectionStatus::Active => "active",
            SharedFolderConnectionStatus::Revoked => "revoked",
        }
        .to_owned(),
        created_at: connection.created_at,
        updated_at: connection.updated_at,
        member_npubs: connection
            .member_npubs
            .iter()
            .map(ToString::to_string)
            .collect(),
    }
}

pub(crate) fn mounted_folder_responses(
    mounted_folders: Vec<MountedFolderProjection>,
) -> Vec<MountedFolderResponse> {
    mounted_folders
        .into_iter()
        .map(|mount| MountedFolderResponse {
            mount_id: mount.mount_id,
            organization_vault_id: mount.organization_vault_id.to_string(),
            source_vault_id: mount.source_vault_id.to_string(),
            source_folder_id: mount.source_folder_id.to_string(),
            connection_id: mount.connection_id,
            display_name: mount.display_name,
            display_parent_folder_id: mount.display_parent_folder_id.map(|id| id.to_string()),
            state: match mount.state {
                MountedFolderState::Available => "available",
                MountedFolderState::Locked => "locked",
                MountedFolderState::Revoked => "revoked",
            }
            .to_owned(),
        })
        .collect()
}

pub(crate) fn encrypted_vault_export_response(
    export: EncryptedVaultExport,
) -> EncryptedVaultExportResponse {
    EncryptedVaultExportResponse {
        version: export.version,
        vault: ExportVaultSummaryResponse {
            id: export.vault.id.to_string(),
            kind: export.vault.kind,
            name: export.vault.name.to_string(),
            owner_user_id: export.vault.owner_user_id.map(|owner| owner.to_string()),
        },
        folders: export
            .folders
            .into_iter()
            .map(|folder| EncryptedExportFolderResponse {
                id: folder.id.to_string(),
                path: folder.path.to_string(),
                access: folder.access,
                current_key_version: folder.current_key_version,
                shared_folder_source: folder.shared_folder_source,
                accessible: folder.accessible,
            })
            .collect(),
        objects: export
            .objects
            .into_iter()
            .map(|object| EncryptedExportObjectResponse {
                folder_id: object.folder_id.to_string(),
                object_id: object.object_id.as_str().to_owned(),
                payload_json: object.payload_json,
                revision: object.revision,
                updated_at: object.updated_at,
                deleted: object.deleted,
                opaque: object.opaque,
            })
            .collect(),
        key_grants: export
            .key_grants
            .into_iter()
            .map(folder_key_grant_response)
            .collect(),
        access_state: EncryptedExportAccessStateResponse {
            members: export
                .access_state
                .members
                .into_iter()
                .map(|member| member.to_string())
                .collect(),
            admins: export
                .access_state
                .admins
                .into_iter()
                .map(|admin| admin.to_string())
                .collect(),
            folders: export
                .access_state
                .folders
                .into_iter()
                .map(|folder| EncryptedExportFolderAccessResponse {
                    folder_id: folder.folder_id.to_string(),
                    user_ids: folder
                        .user_ids
                        .into_iter()
                        .map(|user_id| user_id.to_string())
                        .collect(),
                })
                .collect(),
        },
    }
}

pub(crate) fn folder_key_grant_response(grant: FolderKeyGrantMetadata) -> FolderKeyGrantResponse {
    FolderKeyGrantResponse {
        id: grant.id,
        folder_id: grant.folder_id.to_string(),
        key_version: grant.key_version,
        issuer_npub: grant.issuer_npub.to_string(),
        recipient_npub: grant.recipient_npub.to_string(),
        format: grant.format,
        wrapped_event_json: grant.wrapped_event_json,
        access_change_event_json: grant.access_change_event_json,
        created_at: grant.created_at,
    }
}

fn link_status_str(status: LinkStatus) -> &'static str {
    match status {
        LinkStatus::Pending => "pending",
        LinkStatus::Accepted => "accepted",
        LinkStatus::Revoked => "revoked",
    }
}
