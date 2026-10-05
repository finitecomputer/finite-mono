//! Read-only authority snapshot behind the Brain access report.
//!
//! Authority is read inside one SQLite read transaction so every fact belongs
//! to the same committed state. It never writes: no alias refresh, no
//! provenance backfill, no cache. Only current-version grants are read, never
//! historical wrapped keys or any wrapped-key payload. Per-key evidence
//! (membership provenance, participation, stored aliases) is read only for
//! the requested page through indexed point lookups. Every read is bounded
//! and fails closed with `CapacityExceeded` rather than truncating.

use crate::*;

mod evidence;
mod fingerprint;
mod mounts;

pub use evidence::{ParticipationEvidence, ParticipationKind};
pub use mounts::{IncomingMount, IncomingMountParticipant};

/// Distinct identities one access report may cover before it fails closed.
pub const MAX_ACCESS_REPORT_IDENTITIES: usize = 5_000;
/// Active outgoing Mounts one access report may cover before it fails closed.
pub const MAX_ACCESS_REPORT_OUTGOING_MOUNTS: usize = BRAIN_CAPACITY_ENVELOPE.shared_connections;
/// Current-version grants one access report may read before it fails closed.
pub const MAX_ACCESS_REPORT_CURRENT_GRANTS: usize = BRAIN_CAPACITY_ENVELOPE.folder_key_grants;
/// Explicit-access source rows one access report may read before it fails closed.
pub const MAX_ACCESS_REPORT_ACCESS_SOURCES: usize =
    4 * BRAIN_CAPACITY_ENVELOPE.folder_access_entries;
/// Keys one page may read per-key evidence for.
pub const MAX_ACCESS_REPORT_EVIDENCE_KEYS: usize = 100;

/// Where one explicit Folder access row came from (`folder_access_sources`).
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct FolderAccessSource {
    /// `direct`, `invitation`, or `mount`.
    pub kind: String,
    /// Admin change, invitation, or Mount connection id.
    pub id: String,
}

/// One grant at its Folder's current key version: metadata, provenance, and
/// the signed access-change event stored with it. The wrapped key is not read.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AccessReportGrant {
    pub id: String,
    pub folder_id: FolderId,
    pub key_version: u32,
    pub issuer_npub: UserId,
    pub recipient_npub: UserId,
    pub access_change_event_json: Option<String>,
    pub created_at: String,
    pub provenance: GrantProvenance,
}

/// Current Brain authority for the report. `fingerprint` changes whenever
/// any of these facts change, including changes that append no Brain record
/// (Invite Token redemption, invitation acceptance, Mount participation).
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AccessReportAuthority {
    /// Brain, Folders, Members, and Admins.
    pub brain: Brain,
    /// The one active Personal Agent relationship, when occupied.
    pub personal_agent: Option<PersonalAgent>,
    /// Explicit Folder access by Folder id.
    pub folder_access: BTreeMap<FolderId, BTreeSet<UserId>>,
    /// Recorded sources for each explicit access row; absent means unsourced.
    pub folder_access_sources: BTreeMap<(FolderId, UserId), BTreeSet<FolderAccessSource>>,
    /// Grants at each Folder's current key version.
    pub current_grants: Vec<AccessReportGrant>,
    /// Active Mounts that share one of this Brain's Folders out.
    pub outgoing_connections: Vec<StoredSharedFolderConnection>,
    /// Folders from other Brains mounted into this Brain.
    pub incoming_mounts: Vec<IncomingMount>,
    /// Mount-sourced access rows still awaiting legacy source repair.
    pub pending_mount_source_repairs: usize,
    /// Latest accepted Brain record sequence.
    pub latest_sequence: u64,
    /// Stable digest over access authority above; the content sequence is excluded.
    pub fingerprint: String,
}

impl AccessReportAuthority {
    /// Union of every key the report must account for.
    pub fn candidate_keys(&self) -> BTreeSet<UserId> {
        let mut keys = BTreeSet::new();
        keys.extend(self.brain.owner_user_id.iter().cloned());
        keys.extend(
            self.personal_agent
                .iter()
                .map(|relationship| relationship.agent_npub.clone()),
        );
        keys.extend(
            self.brain
                .members
                .iter()
                .map(|member| member.user_id.clone()),
        );
        keys.extend(self.brain.admins.iter().cloned());
        keys.extend(self.folder_access.values().flatten().cloned());
        keys.extend(
            self.outgoing_connections
                .iter()
                .flat_map(|connection| connection.member_npubs.iter().cloned()),
        );
        keys.extend(
            self.incoming_mounts
                .iter()
                .flat_map(|mount| mount.participants.iter().flatten())
                .map(|participant| participant.npub.clone()),
        );
        keys.extend(
            self.current_grants
                .iter()
                .map(|grant| grant.recipient_npub.clone()),
        );
        keys
    }
}

/// One page's view: full current authority plus per-key evidence for
/// `evidence_keys` only.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AccessReportSnapshot {
    pub authority: AccessReportAuthority,
    /// Keys evidence was read for: at most the page limit, after the cursor.
    pub evidence_keys: BTreeSet<UserId>,
    /// Stored provenance for each Brain Membership row among `evidence_keys`.
    pub member_provenance: BTreeMap<UserId, MemberProvenance>,
    /// Keys among `evidence_keys` that participated in this Brain themselves.
    pub verified_participation: BTreeMap<UserId, ParticipationEvidence>,
    /// Stored identity aliases for participating `evidence_keys`; labels only.
    pub aliases: BTreeMap<UserId, IdentityAlias>,
}

impl BrainStore {
    /// Brain roles only (no Folder access sources, grants, or Mounts), for
    /// denying a caller before any heavier report read.
    pub fn load_brain_roles(
        &self,
        brain_id: &BrainId,
    ) -> Result<(Brain, Option<PersonalAgent>), StoreError> {
        // Legacy loaders collect their rows, so preflight their indexed
        // inputs before allocation as well as bounding new report queries.
        for (sql, label, max) in [
            (
                "SELECT COUNT(*) FROM (SELECT 1 FROM brain_members WHERE brain_id = ?1 LIMIT ?2)",
                "access_report_members",
                MAX_ACCESS_REPORT_IDENTITIES,
            ),
            (
                "SELECT COUNT(*) FROM (SELECT 1 FROM brain_admins WHERE brain_id = ?1 LIMIT ?2)",
                "access_report_admins",
                MAX_ACCESS_REPORT_IDENTITIES,
            ),
            (
                "SELECT COUNT(*) FROM (SELECT 1 FROM folders WHERE brain_id = ?1 LIMIT ?2)",
                "access_report_folders",
                BRAIN_CAPACITY_ENVELOPE.folders,
            ),
        ] {
            self.check_report_row_bound(sql, brain_id.as_str(), label, max)?;
        }
        Ok((
            self.load_core_brain(brain_id)?,
            self.load_personal_agent(brain_id)?,
        ))
    }

    /// Read current authority and its fingerprint in one read transaction.
    pub fn access_report_authority(
        &self,
        brain_id: &BrainId,
    ) -> Result<AccessReportAuthority, StoreError> {
        let read = self.conn.unchecked_transaction()?;
        let authority = self.read_authority(brain_id)?;
        drop(read);
        Ok(authority)
    }

    /// Read one page's snapshot: authority plus evidence for at most `limit`
    /// candidate keys after `after`, all in one read transaction.
    pub fn access_report_snapshot(
        &self,
        brain_id: &BrainId,
        after: Option<&UserId>,
        limit: usize,
    ) -> Result<AccessReportSnapshot, StoreError> {
        // Dropping the transaction releases the read snapshot; nothing is written.
        let read = self.conn.unchecked_transaction()?;
        let authority = self.read_authority(brain_id)?;
        let candidates = authority.candidate_keys();
        ensure_within(
            "access_report_identities",
            MAX_ACCESS_REPORT_IDENTITIES,
            candidates.len(),
        )?;
        let evidence_keys = candidates
            .into_iter()
            .filter(|key| after.is_none_or(|after| key > after))
            .take(limit.min(MAX_ACCESS_REPORT_EVIDENCE_KEYS))
            .collect::<BTreeSet<_>>();
        let member_ids = authority
            .brain
            .members
            .iter()
            .map(|member| &member.user_id)
            .collect::<BTreeSet<_>>();
        let mut member_provenance = BTreeMap::new();
        for key in evidence_keys.iter().filter(|key| member_ids.contains(key)) {
            if let Some(provenance) = self.member_provenance(brain_id, key)? {
                member_provenance.insert(key.clone(), provenance);
            }
        }
        let verified_participation = self.participation_for_keys(brain_id, &evidence_keys)?;
        // Aliases are server-global labels anyone's lookup may have written;
        // read them only for keys that participated here themselves.
        let participating = verified_participation.keys().cloned().collect::<Vec<_>>();
        let aliases = self
            .load_identity_aliases(&participating)?
            .into_iter()
            .map(|alias| (alias.npub.clone(), alias))
            .collect();
        drop(read);
        Ok(AccessReportSnapshot {
            authority,
            evidence_keys,
            member_provenance,
            verified_participation,
            aliases,
        })
    }

    fn check_report_row_bound(
        &self,
        sql: &str,
        id: &str,
        label: &str,
        max: usize,
    ) -> Result<(), StoreError> {
        let count = self
            .conn
            .query_row(sql, params![id, (max + 1) as i64], |row| {
                row.get::<_, usize>(0)
            })?;
        ensure_within(label, max, count)
    }

    fn read_authority(&self, brain_id: &BrainId) -> Result<AccessReportAuthority, StoreError> {
        let (brain, personal_agent) = self.load_brain_roles(brain_id)?;
        self.check_report_row_bound(
            "SELECT COUNT(*) FROM (SELECT 1 FROM folder_access WHERE brain_id = ?1 LIMIT ?2)",
            brain_id.as_str(),
            "access_report_folder_access",
            MAX_ACCESS_REPORT_ACCESS_SOURCES,
        )?;
        let folder_access = self.load_folder_access(brain_id)?;
        let folder_access_sources = self.load_folder_access_sources(brain_id)?;
        let current_grants = self.load_current_grants(brain_id)?;
        let latest_sequence = self.latest_sequence(brain_id)?;
        let outgoing_connections = self.active_outgoing_connections(brain_id)?;
        let incoming_mounts = self.incoming_mounts(brain_id)?;
        let pending_mount_source_repairs = self.conn.query_row(
            "SELECT COUNT(*) FROM legacy_folder_access_source_repairs
             WHERE brain_id = ?1 AND status = 'repair'",
            params![brain_id.as_str()],
            |row| row.get::<_, i64>(0),
        )? as usize;
        let mut authority = AccessReportAuthority {
            brain,
            personal_agent,
            folder_access,
            folder_access_sources,
            current_grants,
            outgoing_connections,
            incoming_mounts,
            pending_mount_source_repairs,
            latest_sequence,
            fingerprint: String::new(),
        };
        authority.fingerprint = fingerprint::authority_fingerprint(&authority);
        Ok(authority)
    }

    fn load_folder_access_sources(
        &self,
        brain_id: &BrainId,
    ) -> Result<BTreeMap<(FolderId, UserId), BTreeSet<FolderAccessSource>>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT folder_id, user_id, source_kind, source_id FROM folder_access_sources
             WHERE brain_id = ?1
             ORDER BY folder_id, user_id, source_kind, source_id
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(
                params![
                    brain_id.as_str(),
                    (MAX_ACCESS_REPORT_ACCESS_SOURCES + 1) as i64
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        ensure_within(
            "access_report_access_sources",
            MAX_ACCESS_REPORT_ACCESS_SOURCES,
            rows.len(),
        )?;
        let mut sources = BTreeMap::<(FolderId, UserId), BTreeSet<FolderAccessSource>>::new();
        for (folder_id, user_id, kind, id) in rows {
            sources
                .entry((FolderId::new(folder_id)?, UserId::new(user_id)?))
                .or_default()
                .insert(FolderAccessSource { kind, id });
        }
        Ok(sources)
    }

    /// Grants at each Folder's current key version, through the
    /// `(brain_id, folder_id, key_version, recipient_npub)` unique index.
    fn load_current_grants(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<AccessReportGrant>, StoreError> {
        let mut stmt = self.conn.prepare(CURRENT_GRANTS_SQL)?;
        let rows = stmt
            .query_map(
                params![
                    brain_id.as_str(),
                    (MAX_ACCESS_REPORT_CURRENT_GRANTS + 1) as i64
                ],
                |row| {
                    Ok((
                        (
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, u32>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ),
                        (
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, Option<String>>(7)?,
                            row.get::<_, String>(8)?,
                            row.get::<_, Option<String>>(9)?,
                            row.get::<_, Option<i64>>(10)?,
                        ),
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        ensure_within(
            "access_report_current_grants",
            MAX_ACCESS_REPORT_CURRENT_GRANTS,
            rows.len(),
        )?;
        let mut grants = Vec::with_capacity(rows.len());
        for (
            (id, folder_id, key_version, issuer, recipient),
            (event, created_at, delegated_by, origin_kind, origin_ref, roster_revision),
        ) in rows
        {
            grants.push(AccessReportGrant {
                id,
                folder_id: FolderId::new(folder_id)?,
                key_version,
                issuer_npub: UserId::new(issuer)?,
                recipient_npub: UserId::new(recipient)?,
                access_change_event_json: event,
                created_at,
                provenance: GrantProvenance {
                    delegated_by_npub: delegated_by.map(UserId::new).transpose()?,
                    origin_kind: ProvenanceOriginKind::try_from(origin_kind.as_str())?,
                    origin_ref,
                    roster_revision,
                },
            });
        }
        Ok(grants)
    }

    fn active_outgoing_connections(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<StoredSharedFolderConnection>, StoreError> {
        let ids = {
            let mut stmt = self.conn.prepare(
                "SELECT id FROM shared_folder_connections
                 WHERE source_brain_id = ?1 AND status = 'active'
                 ORDER BY id
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(
                params![
                    brain_id.as_str(),
                    (MAX_ACCESS_REPORT_OUTGOING_MOUNTS + 1) as i64
                ],
                |row| row.get::<_, String>(0),
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        ensure_within(
            "access_report_outgoing_mounts",
            MAX_ACCESS_REPORT_OUTGOING_MOUNTS,
            ids.len(),
        )?;
        for id in &ids {
            self.check_report_row_bound(
                "SELECT COUNT(*) FROM (SELECT 1 FROM shared_folder_connection_members WHERE connection_id = ?1 LIMIT ?2)",
                id, "access_report_connection_members", MAX_ACCESS_REPORT_IDENTITIES,
            )?;
        }
        ids.iter()
            .map(|id| self.load_shared_folder_connection(id))
            .collect()
    }
}

/// Current-version grants. Joined from `folders` so only each Folder's
/// current key version is read; `wrapped_event_json` is never selected.
pub(crate) const CURRENT_GRANTS_SQL: &str =
    "SELECT g.id, g.folder_id, g.key_version, g.issuer_npub, g.recipient_npub,
        g.access_change_event_json, g.created_at, g.delegated_by_npub, g.origin_kind,
        g.origin_ref, g.roster_revision
 FROM folders f
 JOIN folder_key_grants g
   ON g.brain_id = f.brain_id AND g.folder_id = f.id AND g.key_version = f.current_key_version
 WHERE f.brain_id = ?1
 ORDER BY g.folder_id, g.recipient_npub
 LIMIT ?2";

fn ensure_within(limit: &str, max: usize, current: usize) -> Result<(), StoreError> {
    if current > max {
        return Err(StoreError::CapacityExceeded {
            limit: limit.to_owned(),
            max,
            current,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Detail column of `EXPLAIN QUERY PLAN` for `sql`.
    pub(super) fn query_plan(
        store: &BrainStore,
        sql: &str,
        args: &[&dyn rusqlite::ToSql],
    ) -> Vec<String> {
        let mut stmt = store
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        stmt.query_map(args, |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn current_grants_read_through_the_unique_index_without_wrapped_keys() {
        let store = BrainStore::open_in_memory().unwrap();
        let plan = query_plan(&store, CURRENT_GRANTS_SQL, &[&"brain", &1_i64]);
        assert!(
            plan.iter().all(|detail| !detail.starts_with("SCAN g")
                && !detail.starts_with("SCAN folder_key_grants")),
            "{plan:?}"
        );
        assert!(!CURRENT_GRANTS_SQL.contains("wrapped_event_json"));
    }

    #[test]
    fn missing_brain_fails_without_writing() {
        let store = BrainStore::open_in_memory().unwrap();
        let before = store.conn.total_changes();
        let brain = BrainId::new("absent").unwrap();
        assert!(matches!(
            store.access_report_snapshot(&brain, None, 10),
            Err(StoreError::MissingBrain { .. })
        ));
        assert!(matches!(
            store.access_report_authority(&brain),
            Err(StoreError::MissingBrain { .. })
        ));
        assert_eq!(store.conn.total_changes(), before);
    }

    #[test]
    fn report_bounds_fail_closed_instead_of_truncating() {
        assert!(ensure_within("x", 2, 2).is_ok());
        assert!(matches!(
            ensure_within("x", 2, 3),
            Err(StoreError::CapacityExceeded {
                max: 2,
                current: 3,
                ..
            })
        ));
    }
}
