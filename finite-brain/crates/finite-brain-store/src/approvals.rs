use crate::*;

const APPROVAL_REQUEST_SELECT: &str = r#"
    SELECT id, brain_id, action, payload_json, nonce, expires_at_unix,
           requested_by_npub, status, approval_event_id, resolved_by_npub,
           created_at, updated_at
    FROM brain_approval_requests
"#;

fn approval_request_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<StoredBrainApprovalRequest> {
    let status = row.get::<_, String>(7)?;
    Ok(StoredBrainApprovalRequest {
        id: row.get(0)?,
        brain_id: BrainId::new(row.get::<_, String>(1)?)
            .map_err(to_from_sql_error(1, rusqlite::types::Type::Text))?,
        action: row.get(2)?,
        payload_json: row.get(3)?,
        nonce: row.get(4)?,
        expires_at_unix: row.get::<_, i64>(5)? as u64,
        requested_by_npub: UserId::new(row.get::<_, String>(6)?)
            .map_err(to_from_sql_error(6, rusqlite::types::Type::Text))?,
        status: ApprovalRequestStatus::try_from(status.as_str())
            .map_err(to_store_from_sql_error(7, rusqlite::types::Type::Text))?,
        approval_event_id: row.get(8)?,
        resolved_by_npub: row
            .get::<_, Option<String>>(9)?
            .map(UserId::new)
            .transpose()
            .map_err(to_from_sql_error(9, rusqlite::types::Type::Text))?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn nonce_conflict(reason: &str) -> StoreError {
    StoreError::Conflict {
        reason: reason.to_owned(),
        current_revision: None,
    }
}

impl BrainStore {
    /// Store one pending human Approval request with its server-minted nonce.
    pub fn create_brain_approval_request(
        &mut self,
        request: &StoredBrainApprovalRequest,
    ) -> Result<StoredBrainApprovalRequest, StoreError> {
        self.require_brain_exists(&request.brain_id)?;
        validate_link_id("brain_approval_request_id", &request.id)?;
        if request.status != ApprovalRequestStatus::Pending {
            return Err(StoreError::BrokenInvariant {
                reason: "approval requests are created pending".to_owned(),
            });
        }
        self.conn
            .execute(
                r#"
                INSERT INTO brain_approval_requests (
                    id, brain_id, action, payload_json, nonce, expires_at_unix,
                    requested_by_npub, status, approval_event_id, resolved_by_npub,
                    created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', NULL, NULL, ?8, ?8)
                "#,
                params![
                    request.id,
                    request.brain_id.as_str(),
                    request.action,
                    request.payload_json,
                    request.nonce,
                    request.expires_at_unix as i64,
                    request.requested_by_npub.as_str(),
                    request.created_at,
                ],
            )
            .map_err(|error| {
                let unique_nonce = matches!(
                    &error,
                    rusqlite::Error::SqliteFailure(_, Some(message))
                        if message.contains("UNIQUE constraint failed")
                            && !message.contains("brain_approval_requests.id")
                );
                if unique_nonce {
                    nonce_conflict("approval request nonce already exists")
                } else {
                    map_insert_error("brain_approval_request_id", &request.id)(error)
                }
            })?;
        self.load_brain_approval_request(&request.id)
    }

    /// Load one Approval request by id.
    pub fn load_brain_approval_request(
        &self,
        request_id: &str,
    ) -> Result<StoredBrainApprovalRequest, StoreError> {
        self.conn
            .query_row(
                &format!("{APPROVAL_REQUEST_SELECT} WHERE id = ?1"),
                params![request_id],
                approval_request_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "brain approval request",
            })
    }

    /// Load the Approval request one applied approval event resolved, when any.
    /// This is the durable link from an approval-committed invitation's origin
    /// ref back to the plan the approval authorized.
    pub fn load_brain_approval_request_by_event_id(
        &self,
        brain_id: &BrainId,
        approval_event_id: &str,
    ) -> Result<Option<StoredBrainApprovalRequest>, StoreError> {
        self.conn
            .query_row(
                &format!(
                    "{APPROVAL_REQUEST_SELECT} WHERE brain_id = ?1 AND approval_event_id = ?2"
                ),
                params![brain_id.as_str(), approval_event_id],
                approval_request_from_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// List Approval requests for one Brain, newest first, bounded.
    pub fn list_brain_approval_requests(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<StoredBrainApprovalRequest>, StoreError> {
        self.require_brain_exists(brain_id)?;
        let query = format!(
            "{APPROVAL_REQUEST_SELECT} WHERE brain_id = ?1 ORDER BY created_at DESC, id LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&query)?;
        let rows = stmt.query_map(
            params![brain_id.as_str(), MAX_LINK_LIST_ROWS],
            approval_request_from_row,
        )?;
        let mut requests = Vec::new();
        for row in rows {
            requests.push(row?);
        }
        Ok(requests)
    }

    /// Resolve one pending Approval request. Resolving an already-resolved
    /// request is a conflict so card state can never fork.
    pub fn resolve_brain_approval_request(
        &mut self,
        request_id: &str,
        resolution: ApprovalRequestStatus,
        approval_event_id: Option<&str>,
        resolved_by_npub: &UserId,
        updated_at: &str,
    ) -> Result<StoredBrainApprovalRequest, StoreError> {
        if resolution == ApprovalRequestStatus::Pending {
            return Err(StoreError::BrokenInvariant {
                reason: "approval request resolution must be terminal".to_owned(),
            });
        }
        let updated = self.conn.execute(
            r#"
            UPDATE brain_approval_requests
            SET status = ?2,
                approval_event_id = ?3,
                resolved_by_npub = ?4,
                updated_at = ?5
            WHERE id = ?1 AND status = 'pending'
            "#,
            params![
                request_id,
                resolution.as_str(),
                approval_event_id,
                resolved_by_npub.as_str(),
                updated_at,
            ],
        )?;
        if updated == 0 {
            return Err(nonce_conflict("approval request is already resolved"));
        }
        self.load_brain_approval_request(request_id)
    }

    /// True when this approval nonce was already consumed on this Brain.
    pub fn approval_nonce_seen(&self, brain_id: &BrainId, nonce: &str) -> Result<bool, StoreError> {
        self.conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM brain_approval_nonces WHERE brain_id = ?1 AND nonce = ?2
                 )",
                params![brain_id.as_str(), nonce],
                |row| row.get::<_, bool>(0),
            )
            .map_err(StoreError::from)
    }

    /// Record one consumed approval nonce. A replayed nonce is a conflict.
    pub fn record_brain_approval_nonce(
        &mut self,
        brain_id: &BrainId,
        nonce: &str,
        approval_event_id: &str,
        signer_npub: &UserId,
        action: &str,
        applied_at: &str,
    ) -> Result<(), StoreError> {
        self.conn
            .execute(
                r#"
                INSERT INTO brain_approval_nonces (
                    brain_id, nonce, approval_event_id, signer_npub, action, applied_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                params![
                    brain_id.as_str(),
                    nonce,
                    approval_event_id,
                    signer_npub.as_str(),
                    action,
                    applied_at,
                ],
            )
            .map_err(|error| match error {
                rusqlite::Error::SqliteFailure(_, Some(message))
                    if message.contains("UNIQUE constraint failed") =>
                {
                    nonce_conflict("approval nonce was already applied")
                }
                other => StoreError::from(other),
            })?;
        Ok(())
    }

    /// Grant one Principal an Organization Brain admin role through a signed
    /// Approval: membership is created when missing and stamped with the
    /// approval provenance, then the admin row is added. Granting an existing
    /// admin is a conflict.
    pub fn grant_admin_with_provenance(
        &mut self,
        brain_id: &BrainId,
        target: &UserId,
        provenance: &MemberProvenance,
    ) -> Result<(), StoreError> {
        self.require_organization_brain(brain_id)?;
        let brain = self.load_core_brain(brain_id)?;
        if brain.admins.contains(target) {
            return Err(nonce_conflict("target is already a brain admin"));
        }
        let tx = self.conn.transaction()?;
        insert_member_with_provenance_if_missing(&tx, brain_id, target, provenance)?;
        tx.execute(
            "INSERT INTO brain_admins (brain_id, user_id) VALUES (?1, ?2)",
            params![brain_id.as_str(), target.as_str()],
        )?;
        tx.commit()?;
        Ok(())
    }
}
