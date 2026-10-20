//! Admin demotion: remove the Organization Brain admin role and, in the same
//! transaction, rotate every Folder Key the demoted key held only through that
//! role.
//!
//! Without the rotations the demoted key would keep the current wrap of every
//! `restricted` and `admin_only` Folder it no longer qualifies for (the access
//! report's `revocationIncomplete`). Folders it still reaches through explicit
//! Folder access, All-Members membership or the Personal Agent relationship
//! keep their key.

use crate::folder_key_rotation::{
    apply_folder_key_rotation, begin_authoritative_write, current_folder_key_holders,
    validate_folder_key_rotation_material,
};
use crate::*;

/// Client-prepared admin demotion.
#[derive(Debug, Clone, Copy)]
pub struct AdminDemotionRequest<'a> {
    pub removed_user_id: &'a UserId,
    /// Exactly one rotation per Folder in [`admin_demotion_rotation_folders`],
    /// wrapped for the post-demotion required recipients.
    pub rotations: &'a [MemberFolderRotation],
    pub updated_at: &'a str,
    /// Signed admin access-change records (the demotion and one per rotation)
    /// and the new Folder Key Grant records, appended in the same transaction.
    pub control_records: &'a [SyncRecordInput],
}

/// Folders whose current key must rotate when `removed` loses the admin role:
/// the key is a required recipient now and would not be after the demotion.
pub fn admin_demotion_rotation_folders(
    stored: &StoredBrain,
    removed: &UserId,
) -> Result<BTreeSet<FolderId>, StoreError> {
    let demoted = brain_after_admin_demotion(&stored.brain, removed);
    let personal_agent = stored
        .personal_agent
        .as_ref()
        .map(|relationship| &relationship.agent_npub);
    let mut folders = BTreeSet::new();
    for folder in &stored.brain.folders {
        let access = stored
            .folder_access
            .get(&folder.id)
            .cloned()
            .unwrap_or_default();
        if !required_recipients(&stored.brain, folder, &access, personal_agent)?.contains(removed) {
            continue;
        }
        if !required_recipients(&demoted, folder, &access, personal_agent)?.contains(removed) {
            folders.insert(folder.id.clone());
        }
    }
    Ok(folders)
}

fn brain_after_admin_demotion(brain: &Brain, removed: &UserId) -> Brain {
    let mut demoted = brain.clone();
    demoted.admins.retain(|admin| admin != removed);
    demoted
}

impl BrainStore {
    /// Remove an Organization Brain admin, rotate every Folder Key the key
    /// reached only through the role and append the signed records, atomically.
    ///
    /// Refuses unless `request.rotations` names exactly
    /// [`admin_demotion_rotation_folders`], so no client can demote without
    /// cutting the key off. A demotion with nothing to rotate passes an empty
    /// set. Returns one outcome per rotation, in request order.
    pub fn remove_admin(
        &mut self,
        brain_id: &BrainId,
        request: AdminDemotionRequest<'_>,
    ) -> Result<Vec<FolderKeyRotationOutcome>, StoreError> {
        validate_folder_rotation_fanout(
            FolderRotationOperation::AdminDemotion,
            request
                .rotations
                .iter()
                .map(|rotation| FolderRotationFanout {
                    grants: rotation.grants.len(),
                    reencrypted_records: rotation.reencrypted_records.len(),
                }),
        )?;
        let removed = request.removed_user_id;
        // Every read below happens under the write lock, so the roster and key
        // versions validated are the ones the writes apply to.
        let tx = begin_authoritative_write(&self.conn)?;
        let stored = self.load_brain(brain_id)?;
        if stored.brain.kind != BrainKind::Organization {
            return Err(StoreError::BrokenInvariant {
                reason: "member/admin mutation requires an organization brain".to_owned(),
            });
        }
        if !stored.brain.admins.contains(removed) {
            return Err(StoreError::BrokenInvariant {
                reason: "brain admin does not exist".to_owned(),
            });
        }
        if stored.brain.admins.len() == 1 {
            return Err(StoreError::BrokenInvariant {
                reason: "organization brain must keep at least one admin".to_owned(),
            });
        }
        let required_folders = admin_demotion_rotation_folders(&stored, removed)?;
        let supplied_folders = request
            .rotations
            .iter()
            .map(|rotation| rotation.folder_id.clone())
            .collect::<BTreeSet<_>>();
        if required_folders != supplied_folders || supplied_folders.len() != request.rotations.len()
        {
            return Err(StoreError::FolderRotationSetMismatch {
                operation: "admin demotion",
                required_folder_ids: required_folders.iter().map(ToString::to_string).collect(),
                supplied_folder_ids: request
                    .rotations
                    .iter()
                    .map(|rotation| rotation.folder_id.to_string())
                    .collect(),
            });
        }

        let post_demotion_brain = brain_after_admin_demotion(&stored.brain, removed);
        let personal_agent = stored
            .personal_agent
            .as_ref()
            .map(|relationship| &relationship.agent_npub);
        let live_objects = self.load_current_objects(brain_id)?;
        let mut outcomes = Vec::with_capacity(request.rotations.len());
        for rotation in request.rotations {
            let folder = stored
                .brain
                .folders
                .iter()
                .find(|folder| folder.id == rotation.folder_id)
                .ok_or_else(|| StoreError::MissingFolder {
                    folder_id: rotation.folder_id.to_string(),
                })?;
            let access = stored
                .folder_access
                .get(&folder.id)
                .cloned()
                .unwrap_or_default();
            // Issuers are checked against the roster before the demotion, so
            // an admin may demote itself; recipients against the one after.
            let recipients = validate_folder_key_rotation_material(
                &stored.brain,
                &post_demotion_brain,
                folder,
                &access,
                personal_agent,
                rotation.new_key_version,
                &rotation.grants,
                &rotation.reencrypted_records,
                &live_objects,
            )?;
            debug_assert!(!recipients.contains(removed));
            let cut_off_holders = current_folder_key_holders(&stored, folder)
                .into_iter()
                .filter(|holder| !recipients.contains(holder))
                .collect();
            outcomes.push(FolderKeyRotationOutcome {
                folder_id: folder.id.clone(),
                previous_key_version: folder.current_key_version,
                new_key_version: rotation.new_key_version,
                recipients,
                cut_off_holders,
            });
        }

        tx.execute(
            "DELETE FROM brain_admins WHERE brain_id = ?1 AND user_id = ?2",
            params![brain_id.as_str(), removed.as_str()],
        )?;
        for (rotation, outcome) in request.rotations.iter().zip(&outcomes) {
            apply_folder_key_rotation(
                &tx,
                brain_id,
                &rotation.folder_id,
                outcome.previous_key_version,
                rotation.new_key_version,
                &rotation.grants,
                &rotation.reencrypted_records,
                request.updated_at,
            )?;
        }
        sync_records::append_sync_records(&tx, brain_id, request.control_records)?;
        tx.commit()?;
        Ok(outcomes)
    }
}
