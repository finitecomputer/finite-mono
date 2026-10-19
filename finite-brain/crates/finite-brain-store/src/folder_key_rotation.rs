//! Folder Key rotation: the one store boundary that moves a Folder to its next
//! key version, re-wraps it for exactly the required recipients and cuts off
//! every other key that held the previous current wrap.
//!
//! Standalone rotation and explicit access removal share this path. Lanes that
//! rotate several Folders inside a larger mutation (member removal, admin
//! demotion) reuse [`validate_folder_key_rotation_material`] and
//! [`apply_folder_key_rotation`] inside their own transaction.

use crate::*;

/// Which key, if any, a Folder Key rotation removes from the Folder.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FolderKeyRotationTarget<'a> {
    /// Rotate for exactly the current required recipients. No access row
    /// changes; every key holding the current wrap without entitlement (the
    /// access report's `revocationIncomplete`) loses it.
    RequiredRecipients,
    /// Remove one key: its explicit Folder access row, or a current wrap it
    /// holds without entitlement. A key entitled by its Brain role is refused.
    RemoveAccess(&'a UserId),
}

/// Client-prepared material for one Folder Key rotation.
#[derive(Debug, Clone, Copy)]
pub struct FolderKeyRotationRequest<'a> {
    pub folder_id: &'a FolderId,
    pub new_key_version: u32,
    pub grants: &'a [FolderKeyGrantMetadata],
    pub reencrypted_records: &'a [FolderObjectRevisionSyncRecord],
    pub updated_at: &'a str,
    /// Signed grant and admin access-change records appended in the same
    /// transaction.
    pub control_records: &'a [SyncRecordInput],
}

/// Authoritative result of one committed Folder Key rotation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FolderKeyRotationOutcome {
    pub folder_id: FolderId,
    pub previous_key_version: u32,
    pub new_key_version: u32,
    /// Keys wrapped for the new version: exactly the required recipients.
    pub recipients: BTreeSet<UserId>,
    /// Keys that held the previous current wrap and received no new one.
    pub cut_off_holders: BTreeSet<UserId>,
}

/// Every key the Folder's current key must be wrapped for, from the stored
/// Brain roster, the Folder's explicit access rows and the Personal Agent.
pub fn folder_key_required_recipients(
    stored: &StoredBrain,
    folder: &Folder,
) -> Result<BTreeSet<UserId>, StoreError> {
    let access = stored
        .folder_access
        .get(&folder.id)
        .cloned()
        .unwrap_or_default();
    required_recipients(
        &stored.brain,
        folder,
        &access,
        stored
            .personal_agent
            .as_ref()
            .map(|relationship| &relationship.agent_npub),
    )
}

/// Keys holding a wrap of the Folder's current key without being a required
/// recipient. This is the access report's `revocationIncomplete` state: a
/// current grant without entitlement. A Folder Key rotation removes them all.
pub fn stale_folder_key_holders(
    stored: &StoredBrain,
    folder: &Folder,
) -> Result<BTreeSet<UserId>, StoreError> {
    let required = folder_key_required_recipients(stored, folder)?;
    Ok(current_folder_key_holders(stored, folder)
        .into_iter()
        .filter(|holder| !required.contains(holder))
        .collect())
}

fn current_folder_key_holders(stored: &StoredBrain, folder: &Folder) -> BTreeSet<UserId> {
    stored
        .grants
        .iter()
        .filter(|grant| {
            grant.folder_id == folder.id && grant.key_version == folder.current_key_version
        })
        .map(|grant| grant.recipient_npub.clone())
        .collect()
}

/// Validate one Folder's client-prepared rotation against the roster that
/// will hold after the enclosing mutation commits.
///
/// `post_change_brain` and `remaining_access` describe that roster (for
/// example with a removed member or demoted admin already taken out). Returns
/// the required recipient set the grants were checked against.
#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_folder_key_rotation_material(
    post_change_brain: &Brain,
    folder: &Folder,
    remaining_access: &BTreeSet<UserId>,
    personal_agent: Option<&UserId>,
    new_key_version: u32,
    grants: &[FolderKeyGrantMetadata],
    reencrypted_records: &[FolderObjectRevisionSyncRecord],
    live_objects: &[CurrentEncryptedObject],
) -> Result<BTreeSet<UserId>, StoreError> {
    require_next_key_version(folder, new_key_version)?;
    let mut rotated_folder = folder.clone();
    rotated_folder.current_key_version = new_key_version;
    let required = required_recipients(
        post_change_brain,
        &rotated_folder,
        remaining_access,
        personal_agent,
    )?;
    validate_folder_grants(
        post_change_brain,
        &rotated_folder,
        &required,
        grants,
        personal_agent,
    )?;
    let live_objects = live_objects
        .iter()
        .filter(|object| object.folder_id == folder.id && !object.deleted)
        .cloned()
        .collect::<Vec<_>>();
    validate_rotation_records(&live_objects, reencrypted_records)?;
    Ok(required)
}

pub(crate) fn require_next_key_version(
    folder: &Folder,
    new_key_version: u32,
) -> Result<(), StoreError> {
    if folder.current_key_version.checked_add(1) == Some(new_key_version) {
        Ok(())
    } else {
        Err(StoreError::StaleFolderKeyVersion {
            folder_id: folder.id.to_string(),
            current_key_version: folder.current_key_version,
            requested_key_version: new_key_version,
        })
    }
}

/// Write one validated Folder Key rotation inside the caller's transaction.
///
/// The version bump is a compare-and-swap on the previous version, so a
/// concurrent rotation that committed first fails here even outside the
/// server's store lock. Every pending wrap marker below the new version is
/// dropped: the new grants satisfy entitled recipients, and any marker left
/// for a cut-off key would otherwise block later wrap completion forever.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_folder_key_rotation(
    tx: &Transaction<'_>,
    brain_id: &BrainId,
    folder_id: &FolderId,
    previous_key_version: u32,
    new_key_version: u32,
    grants: &[FolderKeyGrantMetadata],
    reencrypted_records: &[FolderObjectRevisionSyncRecord],
    updated_at: &str,
) -> Result<(), StoreError> {
    let changed = tx.execute(
        "UPDATE folders SET current_key_version = ?4 WHERE brain_id = ?1 AND id = ?2 AND current_key_version = ?3",
        params![
            brain_id.as_str(),
            folder_id.as_str(),
            previous_key_version,
            new_key_version
        ],
    )?;
    if changed != 1 {
        let current_key_version = tx.query_row(
            "SELECT current_key_version FROM folders WHERE brain_id = ?1 AND id = ?2",
            params![brain_id.as_str(), folder_id.as_str()],
            |row| row.get::<_, u32>(0),
        )?;
        return Err(StoreError::StaleFolderKeyVersion {
            folder_id: folder_id.to_string(),
            current_key_version,
            requested_key_version: new_key_version,
        });
    }
    invalidate_pending_email_bootstraps_for_rotated_folder(tx, brain_id, folder_id, updated_at)?;
    for grant in grants {
        insert_grant(tx, brain_id, grant)?;
    }
    tx.execute(
        "DELETE FROM brain_pending_grant_wraps WHERE brain_id = ?1 AND folder_id = ?2 AND key_version < ?3",
        params![brain_id.as_str(), folder_id.as_str(), new_key_version],
    )?;
    for record in reencrypted_records {
        let input = SyncRecordInput::FolderObjectRevision(record.clone());
        sync_records::validate_sync_input(&input)?;
        sync_records::validate_sync_conflict(tx, brain_id, &input)?;
        let sequence = sync_records::next_sequence(tx, brain_id)?;
        sync_records::insert_sync_record(tx, brain_id, sequence, &input)?;
        sync_records::project_sync_record(tx, brain_id, &input)?;
    }
    Ok(())
}

impl BrainStore {
    /// Rotate one Folder Key to its next version and append the signed
    /// control records, atomically.
    ///
    /// With [`FolderKeyRotationTarget::RemoveAccess`] the target must hold the
    /// Folder through an explicit access row or a stale current wrap; a key
    /// entitled by its Brain role is refused because rotation would hand it
    /// the new key again.
    pub fn rotate_folder_key(
        &mut self,
        brain_id: &BrainId,
        target: FolderKeyRotationTarget<'_>,
        request: FolderKeyRotationRequest<'_>,
    ) -> Result<FolderKeyRotationOutcome, StoreError> {
        validate_folder_rotation_fanout(
            match target {
                FolderKeyRotationTarget::RequiredRecipients => {
                    FolderRotationOperation::FolderKeyRotation
                }
                FolderKeyRotationTarget::RemoveAccess(_) => {
                    FolderRotationOperation::FolderAccessRemoval
                }
            },
            [FolderRotationFanout {
                grants: request.grants.len(),
                reencrypted_records: request.reencrypted_records.len(),
            }],
        )?;
        let stored = self.load_brain(brain_id)?;
        let folder = stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == *request.folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: request.folder_id.to_string(),
            })?;
        require_next_key_version(folder, request.new_key_version)?;
        let mut remaining_access = stored
            .folder_access
            .get(&folder.id)
            .cloned()
            .unwrap_or_default();
        let removed_access_row = match target {
            FolderKeyRotationTarget::RequiredRecipients => None,
            FolderKeyRotationTarget::RemoveAccess(removed) => {
                let had_row = remaining_access.remove(removed);
                if !had_row {
                    refuse_removal_without_stale_wrap(&stored, folder, removed)?;
                }
                if folder_access_has_mount_source(&self.conn, brain_id, &folder.id, removed)? {
                    return Err(StoreError::BrokenInvariant {
                        reason: "Folder access target still receives this Folder through an active Mount; remove the target from that Mount before rotating explicit Folder access"
                            .to_owned(),
                    });
                }
                had_row.then_some(removed)
            }
        };
        let personal_agent = stored
            .personal_agent
            .as_ref()
            .map(|relationship| &relationship.agent_npub);
        let live_objects = self.load_current_objects(brain_id)?;
        let recipients = validate_folder_key_rotation_material(
            &stored.brain,
            folder,
            &remaining_access,
            personal_agent,
            request.new_key_version,
            request.grants,
            request.reencrypted_records,
            &live_objects,
        )?;
        let cut_off_holders = current_folder_key_holders(&stored, folder)
            .into_iter()
            .filter(|holder| !recipients.contains(holder))
            .collect::<BTreeSet<_>>();

        let tx = self.conn.transaction()?;
        if let Some(removed) = removed_access_row {
            tx.execute(
                "DELETE FROM folder_access WHERE brain_id = ?1 AND folder_id = ?2 AND user_id = ?3",
                params![brain_id.as_str(), folder.id.as_str(), removed.as_str()],
            )?;
        }
        if let FolderKeyRotationTarget::RemoveAccess(removed) = target {
            pending_wraps::clear_pending_grant_wraps_for_folder_recipient(
                &tx, brain_id, &folder.id, removed,
            )?;
        }
        apply_folder_key_rotation(
            &tx,
            brain_id,
            &folder.id,
            folder.current_key_version,
            request.new_key_version,
            request.grants,
            request.reencrypted_records,
            request.updated_at,
        )?;
        sync_records::append_sync_records(&tx, brain_id, request.control_records)?;
        tx.commit()?;
        Ok(FolderKeyRotationOutcome {
            folder_id: folder.id.clone(),
            previous_key_version: folder.current_key_version,
            new_key_version: request.new_key_version,
            recipients,
            cut_off_holders,
        })
    }
}

/// A removal target without an explicit access row must hold a stale current
/// wrap. Anything else is refused with the reason an admin can act on.
fn refuse_removal_without_stale_wrap(
    stored: &StoredBrain,
    folder: &Folder,
    removed: &UserId,
) -> Result<(), StoreError> {
    if stale_folder_key_holders(stored, folder)?.contains(removed) {
        return Ok(());
    }
    let reason = if folder_key_required_recipients(stored, folder)?.contains(removed) {
        "folder access target holds this Folder through its Brain role, not Folder access; change the role (for example revoke admin) instead of revoking Folder access"
    } else {
        "folder access target does not currently have access"
    };
    Err(StoreError::BrokenInvariant {
        reason: reason.to_owned(),
    })
}
