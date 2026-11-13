//! Which native Folders a Member removal must rotate, and why.
//!
//! The server's removal plan route and the removal itself both call
//! [`member_removal_folder_rotations`], so a client is told exactly the
//! rotation set the store will demand. A removed key must lose every current
//! Folder Key it holds: the Folders it reads today, and the Folders where it
//! still holds the current wrap without entitlement (the access report's
//! `revocationIncomplete`, left behind by an admin demotion).

use crate::*;

/// Why removing a Member must rotate one native Folder Key.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub enum MemberRemovalRotationReason {
    /// The Member is a required recipient of the Folder's current key.
    Readable,
    /// The Member holds the current Folder Key without being a required
    /// recipient (see [`stale_folder_key_holders`]).
    StaleCurrentGrant,
}

/// One native Folder a Member removal must rotate.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MemberRemovalFolderRotation {
    pub folder_id: FolderId,
    pub current_key_version: u32,
    pub reason: MemberRemovalRotationReason,
}

/// Every native Folder whose current key `removed` holds or must hold, in
/// Brain Folder order. Refuses a Brain admin (demote first) and a key that is
/// not a Member, with the same reasons the removal itself returns.
///
/// Bounded by the Brain's Folder and grant capacity envelope.
pub fn member_removal_folder_rotations(
    stored: &StoredBrain,
    removed: &UserId,
) -> Result<Vec<MemberRemovalFolderRotation>, StoreError> {
    if stored.brain.admins.contains(removed) {
        return Err(StoreError::BrokenInvariant {
            reason: "remove admin role before removing member".to_owned(),
        });
    }
    if !stored
        .brain
        .members
        .iter()
        .any(|member| member.user_id == *removed)
    {
        return Err(StoreError::BrokenInvariant {
            reason: "brain member does not exist".to_owned(),
        });
    }
    let mut rotations = Vec::new();
    for folder in &stored.brain.folders {
        let reason = if folder_key_required_recipients(stored, folder)?.contains(removed) {
            MemberRemovalRotationReason::Readable
        } else if stale_folder_key_holders(stored, folder)?.contains(removed) {
            MemberRemovalRotationReason::StaleCurrentGrant
        } else {
            continue;
        };
        rotations.push(MemberRemovalFolderRotation {
            folder_id: folder.id.clone(),
            current_key_version: folder.current_key_version,
            reason,
        });
    }
    Ok(rotations)
}

/// Require exactly one supplied rotation per planned Folder.
///
/// Any mismatch is the typed [`StoreError::FolderRotationSetMismatch`] that
/// admin demotion also returns, so clients see one contract for a roster
/// change prepared against a different rotation set. Nothing is written.
pub(crate) fn require_member_removal_rotation_set(
    planned: &[MemberRemovalFolderRotation],
    supplied: &[MemberFolderRotation],
) -> Result<(), StoreError> {
    let planned_ids = planned
        .iter()
        .map(|rotation| &rotation.folder_id)
        .collect::<BTreeSet<_>>();
    let supplied_ids = supplied
        .iter()
        .map(|rotation| &rotation.folder_id)
        .collect::<BTreeSet<_>>();
    if supplied_ids.len() == supplied.len() && supplied_ids == planned_ids {
        return Ok(());
    }
    Err(StoreError::FolderRotationSetMismatch {
        operation: "member removal",
        required_folder_ids: planned_ids.iter().map(ToString::to_string).collect(),
        supplied_folder_ids: supplied
            .iter()
            .map(|rotation| rotation.folder_id.to_string())
            .collect(),
    })
}

/// The planned stale-grant Folders a refused removal skipped, when skipping
/// them is its only mismatch; empty otherwise.
///
/// That is the shape a client from before stale-grant rotation sends, or a
/// current client whose plan went stale, so the server can name those
/// Folders and say to upgrade or retry instead of only listing both sets.
pub fn member_removal_skipped_stale_grant_folders(
    planned: &[MemberRemovalFolderRotation],
    supplied_folder_ids: &[String],
) -> Vec<String> {
    let supplied = supplied_folder_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let planned_ids = planned
        .iter()
        .map(|rotation| rotation.folder_id.as_str())
        .collect::<BTreeSet<_>>();
    if supplied.len() != supplied_folder_ids.len() || !supplied.is_subset(&planned_ids) {
        return Vec::new();
    }
    let missing = planned
        .iter()
        .filter(|rotation| !supplied.contains(rotation.folder_id.as_str()))
        .collect::<Vec<_>>();
    if missing
        .iter()
        .all(|rotation| rotation.reason == MemberRemovalRotationReason::StaleCurrentGrant)
    {
        missing
            .iter()
            .map(|rotation| rotation.folder_id.to_string())
            .collect()
    } else {
        Vec::new()
    }
}
