//! Viewer session key-delivery records (brain:// live viewer, plan Phase 2).
//!
//! Two-tier ruling: viewer sessions are NOT grants. The entitlement is the
//! requester's existing Folder access, re-checked by the server on every
//! read; these rows only deliver a NIP-44 wrapped Folder Key to an
//! ephemeral principal and carry TTL/revocation key hygiene. The server
//! never sees a plaintext Folder Key.

use crate::*;

/// Deterministic viewer session id, stable across renewals of the same
/// (Brain, Folder, ephemeral npub, key version) so a repeat request is
/// idempotent and the browser keeps polling the same URL.
fn viewer_session_id(
    brain_id: &BrainId,
    folder_id: &FolderId,
    ephemeral_npub: &UserId,
    key_version: u32,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"finite-viewer-session-v1");
    for part in [
        brain_id.as_str(),
        folder_id.as_str(),
        ephemeral_npub.as_str(),
        &key_version.to_string(),
    ] {
        hasher.update(b"\x00");
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn row_to_viewer_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredViewerSession> {
    Ok(StoredViewerSession {
        id: row.get(0)?,
        brain_id: BrainId::new(row.get::<_, String>(1)?)
            .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
        folder_id: FolderId::new(row.get::<_, String>(2)?)
            .map_err(to_from_sql_error(2, rusqlite::types::Type::Text))?,
        ephemeral_npub: UserId::new(row.get::<_, String>(3)?)
            .map_err(to_from_sql_error(3, rusqlite::types::Type::Text))?,
        requester_npub: UserId::new(row.get::<_, String>(4)?)
            .map_err(to_from_sql_error(4, rusqlite::types::Type::Text))?,
        key_version: row.get(5)?,
        requested_ttl_secs: row.get::<_, i64>(6)? as u64,
        wrapped_key_payload: row.get(7)?,
        completed_by_npub: row
            .get::<_, Option<String>>(8)?
            .map(UserId::new)
            .transpose()
            .map_err(to_from_sql_error(8, rusqlite::types::Type::Text))?,
        created_at: row.get(9)?,
        expires_at: row.get(10)?,
        revoked_at: row.get(11)?,
    })
}

const VIEWER_SESSION_COLUMNS: &str = "id, brain_id, folder_id, ephemeral_npub, requester_npub, \
     key_version, requested_ttl_secs, wrapped_key_payload, completed_by_npub, created_at, \
     expires_at, revoked_at";

/// One viewer session request to record.
pub struct ViewerSessionRequest {
    /// Brain holding the Folder.
    pub brain_id: BrainId,
    /// Folder the session may read.
    pub folder_id: FolderId,
    /// Ephemeral viewer principal the Folder Key will be wrapped to.
    pub ephemeral_npub: UserId,
    /// Principal whose Folder access justified the session.
    pub requester_npub: UserId,
    /// Folder Key version that is current at request time.
    pub key_version: u32,
    /// Requested TTL in seconds.
    pub requested_ttl_secs: u64,
    /// Request timestamp.
    pub now: String,
    /// Deadline for the pending slot before the wrap lands.
    pub pending_expires_at: String,
}

/// One viewer-session wrap completion from a key-holding client.
pub struct ViewerWrapCompletion {
    /// Ephemeral viewer principal the wrap is addressed to.
    pub ephemeral_npub: UserId,
    /// Folder Key version the wrap targets.
    pub key_version: u32,
    /// NIP-44 wrapped Folder Key payload.
    pub wrapped_key_payload: String,
    /// npub of the completing key-holding client.
    pub completed_by_npub: UserId,
    /// Expiry timestamp computed by the server from the requested TTL.
    pub expires_at: String,
}

impl BrainStore {
    /// Record a viewer session request: one pending key-delivery row plus
    /// the `viewer-session` pending-wrap marker a key-holding client will
    /// discover on sync. `pending_expires_at` bounds how long the pending
    /// slot may sit uncompleted; the real TTL starts when the wrap lands.
    /// Idempotent per (Brain, Folder, ephemeral npub, key version): a
    /// repeat request refreshes the marker and returns the existing row.
    pub fn create_viewer_session_request(
        &mut self,
        request: ViewerSessionRequest,
    ) -> Result<StoredViewerSession, StoreError> {
        let ViewerSessionRequest {
            brain_id,
            folder_id,
            ephemeral_npub,
            requester_npub,
            key_version,
            requested_ttl_secs,
            now,
            pending_expires_at,
        } = request;
        self.require_brain_exists(&brain_id)?;
        let id = viewer_session_id(&brain_id, &folder_id, &ephemeral_npub, key_version);
        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO brain_viewer_sessions (
                id, brain_id, folder_id, ephemeral_npub, requester_npub,
                key_version, requested_ttl_secs, status,
                wrapped_key_payload, completed_by_npub, created_at, expires_at, revoked_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', NULL, NULL, ?8, ?9, NULL)
            ON CONFLICT (brain_id, folder_id, ephemeral_npub, key_version) DO UPDATE SET
                requested_ttl_secs = excluded.requested_ttl_secs
            "#,
            params![
                id,
                brain_id.as_str(),
                folder_id.as_str(),
                ephemeral_npub.as_str(),
                requester_npub.as_str(),
                key_version,
                requested_ttl_secs as i64,
                now,
                pending_expires_at
            ],
        )?;
        pending_wraps::mark_pending_grant_wrap(
            &tx,
            &brain_id,
            &folder_id,
            &ephemeral_npub,
            key_version,
            PendingGrantWrapReason::ViewerSession,
            &now,
        )?;
        tx.commit()?;
        self.viewer_session(&id)?
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "viewer session row missing after insert".to_owned(),
            })
    }

    /// Load one viewer session by id.
    pub fn viewer_session(&self, id: &str) -> Result<Option<StoredViewerSession>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {VIEWER_SESSION_COLUMNS} FROM brain_viewer_sessions WHERE id = ?1"
        ))?;
        statement
            .query_row(params![id], row_to_viewer_session)
            .optional()
            .map_err(StoreError::from)
    }

    /// The exact session row for one (Folder, ephemeral npub, key version).
    pub fn viewer_session_for_recipient(
        &self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        ephemeral_npub: &UserId,
        key_version: u32,
    ) -> Result<Option<StoredViewerSession>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {VIEWER_SESSION_COLUMNS} FROM brain_viewer_sessions
             WHERE brain_id = ?1 AND folder_id = ?2 AND ephemeral_npub = ?3 AND key_version = ?4"
        ))?;
        statement
            .query_row(
                params![
                    brain_id.as_str(),
                    folder_id.as_str(),
                    ephemeral_npub.as_str(),
                    key_version
                ],
                row_to_viewer_session,
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// The newest-key-version session row one ephemeral principal holds
    /// for a Folder, regardless of status — the honest-state lookup.
    pub fn latest_viewer_session_for_actor(
        &self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        ephemeral_npub: &UserId,
    ) -> Result<Option<StoredViewerSession>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {VIEWER_SESSION_COLUMNS} FROM brain_viewer_sessions
             WHERE brain_id = ?1 AND folder_id = ?2 AND ephemeral_npub = ?3
             ORDER BY key_version DESC LIMIT 1"
        ))?;
        statement
            .query_row(
                params![
                    brain_id.as_str(),
                    folder_id.as_str(),
                    ephemeral_npub.as_str()
                ],
                row_to_viewer_session,
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// Every viewer session of one Brain, newest first, for the admin
    /// access surface (`fbrain viewer-session list`).
    pub fn viewer_sessions_for_brain(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<StoredViewerSession>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {VIEWER_SESSION_COLUMNS} FROM brain_viewer_sessions
             WHERE brain_id = ?1 ORDER BY created_at DESC, id DESC"
        ))?;
        let rows = statement.query_map(params![brain_id.as_str()], row_to_viewer_session)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The unexpired, unrevoked viewer session a principal holds for one
    /// Folder, if any. This is the encrypted-read route's session tier;
    /// callers still re-check the requester's underlying Folder access.
    pub fn live_viewer_session_for(
        &self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        ephemeral_npub: &UserId,
        now: &str,
    ) -> Result<Option<StoredViewerSession>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {VIEWER_SESSION_COLUMNS} FROM brain_viewer_sessions
             WHERE brain_id = ?1 AND folder_id = ?2 AND ephemeral_npub = ?3
               AND revoked_at IS NULL AND expires_at > ?4
             ORDER BY key_version DESC LIMIT 1"
        ))?;
        let row = statement
            .query_row(
                params![
                    brain_id.as_str(),
                    folder_id.as_str(),
                    ephemeral_npub.as_str(),
                    now
                ],
                row_to_viewer_session,
            )
            .optional()?;
        // RFC3339 strings compare correctly here only when both sides use
        // the same timezone suffix convention, so re-derive with the shared
        // parser instead of trusting the SQL comparison.
        Ok(row.filter(|session| session.status_at(now) == ViewerSessionStatus::Ready))
    }

    /// Whether a principal holds any not-yet-revoked, not-yet-expired
    /// viewer session row (pending or ready) for a Brain — the SSE change
    /// signal's viewer tier.
    pub fn has_live_viewer_session_for_brain(
        &self,
        brain_id: &BrainId,
        actor_npub: &str,
        now: &str,
    ) -> Result<bool, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT expires_at FROM brain_viewer_sessions
             WHERE brain_id = ?1 AND ephemeral_npub = ?2 AND revoked_at IS NULL",
        )?;
        let rows = statement.query_map(params![brain_id.as_str(), actor_npub], |row| {
            row.get::<_, String>(0)
        })?;
        for expires_at in rows.collect::<Result<Vec<_>, _>>()? {
            if !timestamp_expired(&expires_at, now) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Complete a pending viewer-session wrap with the NIP-44 payload a
    /// key-holding client produced. Fails closed: the marker must exist at
    /// this key version, the session row must exist, and the completed
    /// payload overwrites the old one so renewals converge. The expiry
    /// clock restarts at completion time from the requested TTL.
    pub fn complete_viewer_session(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        completion: ViewerWrapCompletion,
    ) -> Result<StoredViewerSession, StoreError> {
        let ViewerWrapCompletion {
            ephemeral_npub,
            key_version,
            wrapped_key_payload,
            completed_by_npub,
            expires_at,
        } = completion;
        let tx = self.conn.transaction()?;
        let marked = tx
            .query_row(
                "SELECT COUNT(*) FROM brain_pending_grant_wraps
                 WHERE brain_id = ?1 AND folder_id = ?2 AND recipient_npub = ?3
                   AND key_version = ?4 AND reason = 'viewer-session'",
                params![
                    brain_id.as_str(),
                    folder_id.as_str(),
                    ephemeral_npub.as_str(),
                    key_version
                ],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        if marked == 0 {
            return Err(StoreError::BrokenInvariant {
                reason: "no pending viewer-session wrap marker for this recipient and version"
                    .to_owned(),
            });
        }
        let updated = tx.execute(
            r#"
            UPDATE brain_viewer_sessions
            SET status = 'ready', wrapped_key_payload = ?3, completed_by_npub = ?4,
                expires_at = ?5, revoked_at = NULL
            WHERE brain_id = ?1 AND folder_id = ?2 AND ephemeral_npub = ?6 AND key_version = ?7
            "#,
            params![
                brain_id.as_str(),
                folder_id.as_str(),
                wrapped_key_payload,
                completed_by_npub.as_str(),
                expires_at,
                ephemeral_npub.as_str(),
                key_version
            ],
        )?;
        if updated == 0 {
            return Err(StoreError::BrokenInvariant {
                reason: "viewer session row missing for the marked recipient".to_owned(),
            });
        }
        tx.execute(
            "DELETE FROM brain_pending_grant_wraps
             WHERE brain_id = ?1 AND folder_id = ?2 AND recipient_npub = ?3
               AND key_version = ?4 AND reason = 'viewer-session'",
            params![
                brain_id.as_str(),
                folder_id.as_str(),
                ephemeral_npub.as_str(),
                key_version
            ],
        )?;
        tx.commit()?;
        let id = viewer_session_id(brain_id, folder_id, &ephemeral_npub, key_version);
        self.viewer_session(&id)?
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "viewer session row missing after completion".to_owned(),
            })
    }

    /// Revoke a viewer session by id: key hygiene only, never a Folder
    /// access change. Also clears any still-pending marker so an offline
    /// agent does not later complete a revoked request. Idempotent.
    pub fn revoke_viewer_session(
        &mut self,
        brain_id: &BrainId,
        session_id: &str,
        now: &str,
    ) -> Result<StoredViewerSession, StoreError> {
        let Some(session) = self.viewer_session(session_id)? else {
            return Err(StoreError::UnavailableLink {
                kind: "viewer-session",
            });
        };
        if session.brain_id != *brain_id {
            return Err(StoreError::UnavailableLink {
                kind: "viewer-session",
            });
        }
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE brain_viewer_sessions SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
            params![session_id, now],
        )?;
        tx.execute(
            "DELETE FROM brain_pending_grant_wraps
             WHERE brain_id = ?1 AND folder_id = ?2 AND recipient_npub = ?3
               AND reason = 'viewer-session'",
            params![
                brain_id.as_str(),
                session.folder_id.as_str(),
                session.ephemeral_npub.as_str()
            ],
        )?;
        tx.commit()?;
        self.viewer_session(session_id)?
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "viewer session row missing after revocation".to_owned(),
            })
    }

    /// Pending viewer-session wraps for one Brain, joined with the
    /// requester principal so key-holding clients can apply their
    /// completion policy. Ordered by Folder then ephemeral npub.
    pub fn pending_viewer_session_wraps(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<PendingViewerSessionWrap>, StoreError> {
        let mut statement = self.conn.prepare(
            r#"
            SELECT w.brain_id, w.folder_id, w.recipient_npub, w.key_version, w.created_at,
                   s.requester_npub
            FROM brain_pending_grant_wraps w
            JOIN brain_viewer_sessions s
              ON s.brain_id = w.brain_id AND s.folder_id = w.folder_id
             AND s.ephemeral_npub = w.recipient_npub AND s.key_version = w.key_version
            WHERE w.brain_id = ?1 AND w.reason = 'viewer-session'
            ORDER BY w.folder_id ASC, w.recipient_npub ASC
            "#,
        )?;
        let rows = statement.query_map(params![brain_id.as_str()], |row| {
            Ok(PendingViewerSessionWrap {
                brain_id: BrainId::new(row.get::<_, String>(0)?)
                    .map_err(to_from_sql_error(0, rusqlite::types::Type::Text))?,
                folder_id: FolderId::new(row.get::<_, String>(1)?)
                    .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
                ephemeral_npub: UserId::new(row.get::<_, String>(2)?)
                    .map_err(to_from_sql_error(2, rusqlite::types::Type::Text))?,
                key_version: row.get(3)?,
                created_at: row.get(4)?,
                requester_npub: UserId::new(row.get::<_, String>(5)?)
                    .map_err(to_from_sql_error(5, rusqlite::types::Type::Text))?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Folder-scoped record pull for the encrypted-read route: only
    /// `folder_object_revision` and `folder_object_tombstone` rows for the
    /// named Folder, ordered by sequence, plus that Folder's latest
    /// sequence. Control records (key grants, admin changes) never appear.
    pub fn pull_folder_view_records(
        &self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        after_sequence: u64,
        limit: u64,
    ) -> Result<(Vec<StoredSyncRecord>, u64), StoreError> {
        self.require_brain_exists(brain_id)?;
        let latest_sequence = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM brain_record_index
                 WHERE brain_id = ?1 AND folder_id = ?2
                   AND record_type IN ('folder_object_revision', 'folder_object_tombstone')",
                params![brain_id.as_str(), folder_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value as u64)
            .map_err(StoreError::from)?;
        let mut statement = self.conn.prepare(
            "SELECT sequence, record_event_id, record_type, folder_id, object_id, revision,
                    actor_npub, client_created_at, payload_json, accepted_at, record_event_kind
             FROM brain_record_index
             WHERE brain_id = ?1 AND folder_id = ?2 AND sequence > ?3
               AND record_type IN ('folder_object_revision', 'folder_object_tombstone')
             ORDER BY sequence ASC LIMIT ?4",
        )?;
        let rows = statement.query_map(
            params![
                brain_id.as_str(),
                folder_id.as_str(),
                after_sequence as i64,
                limit as i64
            ],
            sync_records::stored_sync_record_from_row,
        )?;
        Ok((rows.collect::<Result<Vec<_>, _>>()?, latest_sequence))
    }

    /// (record count, total payload bytes) for the Folder's view records —
    /// the live-viewer size caps. Never truncates: callers fail closed.
    pub fn folder_view_record_stats(
        &self,
        brain_id: &BrainId,
        folder_id: &FolderId,
    ) -> Result<(u64, u64), StoreError> {
        self.conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(LENGTH(payload_json)), 0)
                 FROM brain_record_index
                 WHERE brain_id = ?1 AND folder_id = ?2
                   AND record_type IN ('folder_object_revision', 'folder_object_tombstone')",
                params![brain_id.as_str(), folder_id.as_str()],
                |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64)),
            )
            .map_err(StoreError::from)
    }
}
