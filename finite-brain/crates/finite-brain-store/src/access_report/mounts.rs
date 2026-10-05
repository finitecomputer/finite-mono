//! Folders from other Brains mounted into the reported Brain.
//!
//! Only facts the Mount itself created are read for each participant: its
//! connection participation, its Mount-sourced access row on the source
//! Folder, and its grant at the source Folder's current key version. Other
//! source-Brain routes (source Membership, direct source Guests) belong to the
//! source Brain's own report and are never read here.

use crate::*;

/// Incoming Mounts one access report may read before it fails closed.
pub const MAX_ACCESS_REPORT_INCOMING_MOUNTS: usize = BRAIN_CAPACITY_ENVELOPE.mounts;
/// Participant checks across all incoming Mounts before detail is withheld
/// (and coverage says so).
pub const MAX_ACCESS_REPORT_INCOMING_PARTICIPANTS: usize = 5_000;

/// One Folder from another Brain mounted into this Brain.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IncomingMount {
    pub mount_id: String,
    pub connection_id: String,
    pub source_brain_id: BrainId,
    pub source_folder_id: FolderId,
    pub display_name: String,
    /// Connection status: `active`, `revoked`, or `missing`.
    pub connection_status: String,
    /// Source Folder's current key version; `None` when the Folder is gone.
    pub source_key_version: Option<u32>,
    /// `None` when the participant bound was reached and detail was withheld.
    pub participants: Option<Vec<IncomingMountParticipant>>,
}

impl IncomingMount {
    pub fn is_active(&self) -> bool {
        self.connection_status == "active"
    }
}

/// One destination key participating in an incoming Mount.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IncomingMountParticipant {
    pub npub: UserId,
    /// The source Folder has an access row sourced by this Mount's connection.
    pub mount_access: bool,
    /// A grant exists at the source Folder's current key version.
    pub current_grant: bool,
}

impl BrainStore {
    pub(super) fn incoming_mounts(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<IncomingMount>, StoreError> {
        let rows = {
            let mut stmt = self.conn.prepare(
                "SELECT m.id, m.connection_id, m.source_brain_id, m.source_folder_id,
                        m.display_name, COALESCE(c.status, 'missing'), f.current_key_version
                 FROM folder_mounts m
                 LEFT JOIN shared_folder_connections c ON c.id = m.connection_id
                 LEFT JOIN folders f ON f.brain_id = m.source_brain_id AND f.id = m.source_folder_id
                 WHERE m.destination_brain_id = ?1
                 ORDER BY m.id
                 LIMIT ?2",
            )?;
            stmt.query_map(
                params![
                    brain_id.as_str(),
                    (MAX_ACCESS_REPORT_INCOMING_MOUNTS + 1) as i64
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<u32>>(6)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?
        };
        if rows.len() > MAX_ACCESS_REPORT_INCOMING_MOUNTS {
            return Err(StoreError::CapacityExceeded {
                limit: "access_report_incoming_mounts".to_owned(),
                max: MAX_ACCESS_REPORT_INCOMING_MOUNTS,
                current: rows.len(),
            });
        }
        let mut budget = MAX_ACCESS_REPORT_INCOMING_PARTICIPANTS;
        let mut mounts = Vec::with_capacity(rows.len());
        for (mount_id, connection_id, source_brain, source_folder, display_name, status, version) in
            rows
        {
            let mut mount = IncomingMount {
                mount_id,
                connection_id,
                source_brain_id: BrainId::new(source_brain)?,
                source_folder_id: FolderId::new(source_folder)?,
                display_name,
                connection_status: status,
                source_key_version: version,
                participants: Some(Vec::new()),
            };
            if mount.is_active() {
                mount.participants = self.incoming_participants(&mount, &mut budget)?;
            }
            mounts.push(mount);
        }
        Ok(mounts)
    }

    fn incoming_participants(
        &self,
        mount: &IncomingMount,
        budget: &mut usize,
    ) -> Result<Option<Vec<IncomingMountParticipant>>, StoreError> {
        let count = self.conn.query_row(
            "SELECT COUNT(*) FROM shared_folder_connection_members WHERE connection_id = ?1",
            params![mount.connection_id],
            |row| row.get::<_, i64>(0),
        )? as usize;
        if count > *budget {
            *budget = 0;
            return Ok(None);
        }
        *budget -= count;
        let mut participants = Vec::with_capacity(count);
        for npub in self.load_connection_members(&mount.connection_id)? {
            let mount_access = folder_access_has_source(
                &self.conn,
                &mount.source_brain_id,
                &mount.source_folder_id,
                &npub,
                "mount",
                &mount.connection_id,
            )?;
            let current_grant = match mount.source_key_version {
                None => false,
                Some(version) => self.conn.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM folder_key_grants
                        WHERE brain_id = ?1 AND folder_id = ?2 AND key_version = ?3
                          AND recipient_npub = ?4
                     )",
                    params![
                        mount.source_brain_id.as_str(),
                        mount.source_folder_id.as_str(),
                        version,
                        npub.as_str()
                    ],
                    |row| row.get::<_, bool>(0),
                )?,
            };
            participants.push(IncomingMountParticipant {
                npub,
                mount_access,
                current_grant,
            });
        }
        Ok(Some(participants))
    }
}
