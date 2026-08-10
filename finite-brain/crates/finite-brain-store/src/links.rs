use crate::*;

const BRAIN_INVITATION_SELECT: &str = r#"
    SELECT id, brain_id, user_id, status, invite_code, accept_path,
           initial_folder_access_json, created_by_npub, expires_at,
           created_at, updated_at, accepted_at, target_kind, invited_email,
           invite_unwrap_npub, bootstrap_payload_hash, bootstrap_wrapped_event_json,
           bootstrap_authorization_event_json, claimed_by_npub, bootstrap_scope_json
           , folder_only
    FROM brain_invitations
"#;

fn capacity_count(
    conn: &Connection,
    table: &str,
    predicate: &str,
    brain_id: &BrainId,
    now: &str,
) -> Result<usize, StoreError> {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE brain_id = ?1 {predicate}");
    let value = if predicate.contains("?2") {
        conn.query_row(&sql, params![brain_id.as_str(), now], |row| {
            row.get::<_, i64>(0)
        })?
    } else {
        conn.query_row(&sql, params![brain_id.as_str()], |row| row.get::<_, i64>(0))?
    };
    usize::try_from(value).map_err(|_| StoreError::BrokenInvariant {
        reason: format!("negative or oversized {table} capacity count"),
    })
}

fn brain_invitation_capacity_reservations_on(
    conn: &Connection,
    brain_id: &BrainId,
    excluding_cohort_invitation_id: Option<&str>,
    now: &str,
) -> Result<BrainCapacityReservations, StoreError> {
    let args = params![brain_id.as_str(), excluding_cohort_invitation_id, now];
    let mut member_statement = conn.prepare(
        r#"
        SELECT DISTINCT json_extract(participant.value, '$.npub')
        FROM cohort_invitation_plans plan
        JOIN brain_invitations invitation ON invitation.id = plan.invitation_id,
             json_each(plan.participants_json) participant
        WHERE invitation.brain_id = ?1 AND invitation.status = 'pending'
          AND julianday(invitation.expires_at) > julianday(?3)
          AND plan.scope_kind = 'brain'
          AND (?2 IS NULL OR plan.invitation_id <> ?2)
          AND NOT EXISTS (
              SELECT 1 FROM brain_members member
              WHERE member.brain_id = ?1
                AND member.user_id = json_extract(participant.value, '$.npub')
          )
        "#,
    )?;
    let members = member_statement
        .query_map(args, |row| row.get::<_, String>(0))?
        .collect::<Result<BTreeSet<_>, _>>()?;

    let mut access_statement = conn.prepare(
        r#"
        SELECT DISTINCT candidate.folder_id, candidate.user_id
        FROM (
            SELECT plan.folder_id AS folder_id,
                   json_extract(participant.value, '$.npub') AS user_id
            FROM cohort_invitation_plans plan
            JOIN brain_invitations invitation ON invitation.id = plan.invitation_id,
                 json_each(plan.participants_json) participant
            WHERE invitation.brain_id = ?1 AND invitation.status = 'pending'
              AND julianday(invitation.expires_at) > julianday(?3)
              AND plan.scope_kind = 'folder'
              AND (?2 IS NULL OR plan.invitation_id <> ?2)
            UNION
            SELECT folder.value AS folder_id,
                   json_extract(participant.value, '$.npub') AS user_id
            FROM cohort_invitation_plans plan
            JOIN brain_invitations invitation ON invitation.id = plan.invitation_id,
                 json_each(plan.participants_json) participant,
                 json_each(invitation.initial_folder_access_json) folder
            WHERE invitation.brain_id = ?1 AND invitation.status = 'pending'
              AND julianday(invitation.expires_at) > julianday(?3)
              AND plan.scope_kind = 'brain'
              AND (?2 IS NULL OR plan.invitation_id <> ?2)
        ) candidate
        WHERE NOT EXISTS (
            SELECT 1 FROM folder_access access
            WHERE access.brain_id = ?1 AND access.folder_id = candidate.folder_id
              AND access.user_id = candidate.user_id
        )
        "#,
    )?;
    let folder_access_entries = access_statement
        .query_map(
            params![brain_id.as_str(), excluding_cohort_invitation_id, now],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?
        .collect::<Result<BTreeSet<_>, _>>()?;

    let mut grant_statement = conn.prepare(
        r#"
        SELECT DISTINCT grant.folder_id, grant.key_version, grant.recipient_npub
        FROM cohort_invitation_grants grant
        JOIN brain_invitations invitation ON invitation.id = grant.invitation_id
        WHERE invitation.brain_id = ?1 AND invitation.status = 'pending'
          AND julianday(invitation.expires_at) > julianday(?3)
          AND (?2 IS NULL OR grant.invitation_id <> ?2)
          AND NOT EXISTS (
              SELECT 1 FROM folder_key_grants current_grant
              WHERE current_grant.brain_id = ?1
                AND current_grant.folder_id = grant.folder_id
                AND current_grant.key_version = grant.key_version
                AND current_grant.recipient_npub = grant.recipient_npub
          )
        "#,
    )?;
    let folder_key_grants = grant_statement
        .query_map(
            params![brain_id.as_str(), excluding_cohort_invitation_id, now],
            |row| {
                let version = row.get::<_, i64>(1)?;
                let version = u32::try_from(version).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })?;
                Ok((row.get::<_, String>(0)?, version, row.get::<_, String>(2)?))
            },
        )?
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(BrainCapacityReservations {
        members,
        folder_access_entries,
        folder_key_grants,
    })
}

fn brain_invitation_capacity_usage_on(
    conn: &Connection,
    brain_id: &BrainId,
    excluding_cohort_invitation_id: Option<&str>,
    now: &str,
) -> Result<BrainCapacityUsage, StoreError> {
    let reservations = brain_invitation_capacity_reservations_on(
        conn,
        brain_id,
        excluding_cohort_invitation_id,
        now,
    )?;
    let grants = reservations.folder_key_grants.len();
    Ok(BrainCapacityUsage {
        members: capacity_count(conn, "brain_members", "", brain_id, now)?
            .saturating_add(reservations.members.len()),
        folder_access_entries: capacity_count(conn, "folder_access", "", brain_id, now)?
            .saturating_add(reservations.folder_access_entries.len()),
        folder_key_grants: capacity_count(conn, "folder_key_grants", "", brain_id, now)?
            .saturating_add(grants),
        sync_records: capacity_count(
            conn,
            "brain_record_index",
            "AND COALESCE(json_extract(payload_json, '$.recordType'), '') <> 'folder_subtree_tombstone'",
            brain_id,
            now,
        )?
        .saturating_add(grants),
        pending_invitations: capacity_count(
            conn,
            "brain_invitations",
            "AND status = 'pending' AND julianday(expires_at) > julianday(?2)",
            brain_id,
            now,
        )?,
    })
}

fn revoke_expired_pending_invitations_on(
    conn: &Connection,
    brain_id: &BrainId,
    now: &str,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE brain_invitations SET status = 'revoked', updated_at = ?2
         WHERE brain_id = ?1 AND status = 'pending'
           AND julianday(expires_at) <= julianday(?2)",
        params![brain_id.as_str(), now],
    )?;
    Ok(())
}

fn enforce_invitation_capacity_on(
    conn: &Connection,
    brain_id: &BrainId,
    now: &str,
) -> Result<(), StoreError> {
    let usage = brain_invitation_capacity_usage_on(conn, brain_id, None, now)?;
    for (limit, max, current) in [
        (
            "brain_members",
            BRAIN_CAPACITY_ENVELOPE.members,
            usage.members,
        ),
        (
            "folder_access",
            BRAIN_CAPACITY_ENVELOPE.folder_access_entries,
            usage.folder_access_entries,
        ),
        (
            "folder_key_grants",
            BRAIN_CAPACITY_ENVELOPE.folder_key_grants,
            usage.folder_key_grants,
        ),
        (
            "sync_records",
            BRAIN_CAPACITY_ENVELOPE.sync_records - BRAIN_CAPACITY_ENVELOPE.folders,
            usage.sync_records,
        ),
        (
            "pending_invitations",
            BRAIN_CAPACITY_ENVELOPE.invitations,
            usage.pending_invitations,
        ),
    ] {
        if current > max {
            return Err(StoreError::CapacityExceeded {
                limit: limit.to_owned(),
                max,
                current,
            });
        }
    }
    Ok(())
}

impl BrainStore {
    /// Resolve an exact immutable cohort plan for idempotent HTTP retry.
    pub fn load_account_cohort_invitation_by_plan_id(
        &self,
        brain_id: &BrainId,
        plan_id: &str,
    ) -> Result<Option<StoredBrainInvitation>, StoreError> {
        let invitation_id = self
            .conn
            .query_row(
                r#"
                SELECT plan.invitation_id
                FROM cohort_invitation_plans plan
                JOIN brain_invitations invitation ON invitation.id = plan.invitation_id
                WHERE plan.plan_id = ?1 AND invitation.brain_id = ?2
                "#,
                params![plan_id, brain_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        invitation_id
            .map(|invitation_id| self.load_brain_invitation(&invitation_id))
            .transpose()
    }

    /// Read the capacity counters affected by an account-cohort invitation.
    /// Aggregate SQL keeps this independent of the number of historical rows.
    pub fn brain_invitation_capacity_usage(
        &self,
        brain_id: &BrainId,
        excluding_cohort_invitation_id: Option<&str>,
        now: &str,
    ) -> Result<BrainCapacityUsage, StoreError> {
        self.require_brain_exists(brain_id)?;
        validate_link_timestamp("now", now)?;
        brain_invitation_capacity_usage_on(
            &self.conn,
            brain_id,
            excluding_cohort_invitation_id,
            now,
        )
    }

    pub fn brain_invitation_capacity_reservations(
        &self,
        brain_id: &BrainId,
        excluding_cohort_invitation_id: Option<&str>,
        now: &str,
    ) -> Result<BrainCapacityReservations, StoreError> {
        self.require_brain_exists(brain_id)?;
        validate_link_timestamp("now", now)?;
        brain_invitation_capacity_reservations_on(
            &self.conn,
            brain_id,
            excluding_cohort_invitation_id,
            now,
        )
    }

    /// Persist one immutable account-cohort invitation and all encrypted grants
    /// needed to make its approved participants readable at acceptance time.
    #[allow(clippy::too_many_arguments)]
    pub fn create_account_cohort_invitation(
        &mut self,
        brain_id: &BrainId,
        id: &str,
        plan_id: &str,
        account_id: &str,
        human_email: &str,
        roster_revision: u64,
        participants: &[StoredCohortParticipant],
        exclusions_json: &str,
        key_versions_json: &str,
        folder_only: bool,
        initial_folder_access: &[FolderId],
        grants: &[FolderKeyGrantMetadata],
        control_records: &[SyncRecordInput],
        invite_code: &str,
        accept_path: &str,
        created_by_npub: &UserId,
        expires_at: &str,
        created_at: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        if let Some(existing_id) = self
            .conn
            .query_row(
                "SELECT invitation_id FROM cohort_invitation_plans WHERE plan_id = ?1",
                params![plan_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            return self.load_brain_invitation(&existing_id);
        }
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, created_by_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "account-cohort invitations require Brain operational authority".to_owned(),
            });
        }
        validate_link_id("brain_invitation_id", id)?;
        validate_link_id("invite_code", invite_code)?;
        validate_required_text("plan_id", plan_id)?;
        validate_required_text("account_id", account_id)?;
        validate_bounded_offer_expiry(expires_at, created_at)?;
        let human_email = canonical_invited_email(human_email)?;
        let human = participants
            .iter()
            .filter(|participant| participant.relationship == "human")
            .collect::<Vec<_>>();
        if human.len() != 1
            || participants.is_empty()
            || human[0].nip05.trim().to_ascii_lowercase() != human_email
        {
            return Err(StoreError::BrokenInvariant {
                reason: "cohort invitation requires one matching human participant".to_owned(),
            });
        }
        let participant_npubs = participants
            .iter()
            .map(|participant| participant.npub.clone())
            .collect::<BTreeSet<_>>();
        if participant_npubs.len() != participants.len()
            || participants.iter().any(|participant| {
                !matches!(participant.relationship.as_str(), "human" | "account_agent")
                    || participant.name.trim().is_empty()
                    || participant.nip05.trim().is_empty()
            })
        {
            return Err(StoreError::BrokenInvariant {
                reason: "cohort invitation participants are invalid or duplicated".to_owned(),
            });
        }
        let missing_members = participants
            .iter()
            .filter(|participant| {
                !stored
                    .brain
                    .members
                    .iter()
                    .any(|member| member.user_id == participant.npub)
            })
            .count();
        if !folder_only
            && stored.brain.members.len().saturating_add(missing_members)
                > BRAIN_CAPACITY_ENVELOPE.members
        {
            return Err(StoreError::CapacityExceeded {
                limit: "brain_members".to_owned(),
                max: BRAIN_CAPACITY_ENVELOPE.members,
                current: stored.brain.members.len().saturating_add(missing_members),
            });
        }
        let scope = email_bootstrap_scope(&stored.brain, initial_folder_access, folder_only)?;
        let required = scope
            .iter()
            .flat_map(|folder| {
                participants.iter().map(move |participant| {
                    (
                        folder.folder_id.clone(),
                        folder.key_version,
                        participant.npub.clone(),
                    )
                })
            })
            .collect::<BTreeSet<_>>();
        let provided = grants
            .iter()
            .map(|grant| {
                (
                    grant.folder_id.clone(),
                    grant.key_version,
                    grant.recipient_npub.clone(),
                )
            })
            .collect::<BTreeSet<_>>();
        if provided != required || grants.len() != required.len() {
            return Err(StoreError::BrokenInvariant {
                reason: "cohort invitation grants must exactly cover every participant and Folder"
                    .to_owned(),
            });
        }
        for grant in grants {
            validate_grant_metadata(grant)?;
            validate_grant_issuer(
                &stored.brain,
                grant,
                stored
                    .personal_agent
                    .as_ref()
                    .map(|relationship| &relationship.agent_npub),
                has_brain_operational_authority(&stored, &grant.issuer_npub),
            )?;
            if grant.issuer_npub != *created_by_npub {
                return Err(StoreError::BrokenInvariant {
                    reason: "cohort invitation grant issuer must be the invitation actor"
                        .to_owned(),
                });
            }
        }
        validate_folder_key_grant_control_records(grants, control_records)?;
        let participants_json =
            serde_json::to_string(participants).map_err(|error| StoreError::BrokenInvariant {
                reason: format!("cohort participants did not serialize: {error}"),
            })?;
        serde_json::from_str::<serde_json::Value>(exclusions_json).map_err(|_| {
            StoreError::BrokenInvariant {
                reason: "cohort exclusions must be JSON".to_owned(),
            }
        })?;
        serde_json::from_str::<serde_json::Value>(key_versions_json).map_err(|_| {
            StoreError::BrokenInvariant {
                reason: "cohort key versions must be JSON".to_owned(),
            }
        })?;
        let initial_folder_access_json = folder_id_vec_json(initial_folder_access)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        revoke_expired_pending_invitations_on(&tx, brain_id, created_at)?;
        tx.execute(
            r#"
            INSERT INTO brain_invitations (
                id, brain_id, user_id, target_kind, invited_email, status,
                invite_code, accept_path, initial_folder_access_json,
                created_by_npub, expires_at, created_at, updated_at, folder_only,
                bootstrap_scope_json
            ) VALUES (
                ?1, ?2, ?3, 'account_cohort', ?4, 'pending',
                ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?11, '[]'
            )
            "#,
            params![
                id,
                brain_id.as_str(),
                human[0].npub.as_str(),
                human_email,
                invite_code,
                accept_path,
                initial_folder_access_json,
                created_by_npub.as_str(),
                expires_at,
                created_at,
                i64::from(folder_only),
            ],
        )
        .map_err(map_insert_error("brain_invitation_id", id))?;
        tx.execute(
            r#"
            INSERT INTO cohort_invitation_plans (
                invitation_id, plan_id, account_id, human_email, roster_revision,
                scope_kind, folder_id, participants_json, exclusions_json,
                key_versions_json, actor_npub, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                id,
                plan_id,
                account_id,
                human_email,
                i64::try_from(roster_revision).map_err(|_| StoreError::BrokenInvariant {
                    reason: "roster revision exceeds SQLite integer range".to_owned(),
                })?,
                if folder_only { "folder" } else { "brain" },
                folder_only.then(|| initial_folder_access[0].as_str()),
                participants_json,
                exclusions_json,
                key_versions_json,
                created_by_npub.as_str(),
                created_at,
            ],
        )?;
        for (grant, record) in grants.iter().zip(control_records) {
            let SyncRecordInput::Control(record) = record else {
                return Err(StoreError::BrokenInvariant {
                    reason: "cohort invitation grants require control sync records".to_owned(),
                });
            };
            tx.execute(
                r#"
                INSERT INTO cohort_invitation_grants (
                    invitation_id, grant_id, folder_id, key_version, issuer_npub,
                    recipient_npub, format, wrapped_event_json, created_at,
                    record_event_id, record_payload_json, record_event_kind
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                "#,
                params![
                    id,
                    grant.id,
                    grant.folder_id.as_str(),
                    i64::from(grant.key_version),
                    grant.issuer_npub.as_str(),
                    grant.recipient_npub.as_str(),
                    grant.format,
                    grant.wrapped_event_json,
                    grant.created_at,
                    record.record_event_id,
                    record.payload_json,
                    i64::from(record.record_event_kind),
                ],
            )?;
        }
        enforce_invitation_capacity_on(&tx, brain_id, created_at)?;
        tx.commit()?;
        self.load_brain_invitation(id)
    }

    /// Convert one pending internal-beta Finite VIP mailbox bootstrap into a
    /// fixed, key-complete account-cohort invitation without changing its
    /// delivery identity, scope, expiry, or notification receipt.
    #[allow(clippy::too_many_arguments)]
    pub fn convert_pending_email_invitation_to_account_cohort(
        &mut self,
        brain_id: &BrainId,
        invitation_id: &str,
        plan_id: &str,
        account_id: &str,
        human_email: &str,
        roster_revision: u64,
        participants: &[StoredCohortParticipant],
        exclusions_json: &str,
        key_versions_json: &str,
        grants: &[FolderKeyGrantMetadata],
        control_records: &[SyncRecordInput],
        actor: &UserId,
        backup_reference: &str,
        converted_at: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        validate_link_id("brain_invitation_id", invitation_id)?;
        validate_required_text("plan_id", plan_id)?;
        validate_required_text("account_id", account_id)?;
        validate_required_text("backup_reference", backup_reference)?;
        validate_link_timestamp("convertedAt", converted_at)?;
        if let Some(existing_invitation_id) = self
            .conn
            .query_row(
                "SELECT invitation_id FROM invitation_cohort_conversion_receipts WHERE plan_id = ?1",
                params![plan_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            if existing_invitation_id == invitation_id {
                return self.load_brain_invitation(invitation_id);
            }
            return Err(StoreError::BrokenInvariant {
                reason: "invitation conversion plan belongs to another invitation".to_owned(),
            });
        }
        let invitation = self.load_brain_invitation(invitation_id)?;
        if invitation.brain_id != *brain_id
            || invitation.status != LinkStatus::Pending
            || invitation.target_kind != BrainInvitationTargetKind::EmailBootstrap
        {
            return Err(StoreError::BrokenInvariant {
                reason: "invitation is not a pending mailbox bootstrap eligible for conversion"
                    .to_owned(),
            });
        }
        validate_link_timestamp("expiresAt", &invitation.expires_at)?;
        let expires = OffsetDateTime::parse(&invitation.expires_at, &Rfc3339).map_err(|_| {
            StoreError::BrokenInvariant {
                reason: "expiresAt must be an RFC3339 timestamp".to_owned(),
            }
        })?;
        let converted = OffsetDateTime::parse(converted_at, &Rfc3339).map_err(|_| {
            StoreError::BrokenInvariant {
                reason: "convertedAt must be an RFC3339 timestamp".to_owned(),
            }
        })?;
        if expires <= converted {
            return Err(StoreError::BrokenInvariant {
                reason: "expired invitation cannot be converted".to_owned(),
            });
        }
        let human_email = canonical_invited_email(human_email)?;
        if invitation.invited_email.as_deref() != Some(human_email.as_str()) {
            return Err(StoreError::BrokenInvariant {
                reason: "conversion mailbox does not match the pending invitation".to_owned(),
            });
        }
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, actor) {
            return Err(StoreError::BrokenInvariant {
                reason: "invitation conversion requires Brain operational authority".to_owned(),
            });
        }
        let human = participants
            .iter()
            .filter(|participant| participant.relationship == "human")
            .collect::<Vec<_>>();
        if human.len() != 1
            || participants.is_empty()
            || human[0].nip05.trim().to_ascii_lowercase() != human_email
        {
            return Err(StoreError::BrokenInvariant {
                reason: "converted invitation requires one matching human participant".to_owned(),
            });
        }
        let participant_npubs = participants
            .iter()
            .map(|participant| participant.npub.clone())
            .collect::<BTreeSet<_>>();
        if participant_npubs.len() != participants.len()
            || participants.iter().any(|participant| {
                !matches!(participant.relationship.as_str(), "human" | "account_agent")
                    || participant.name.trim().is_empty()
                    || participant.nip05.trim().is_empty()
            })
        {
            return Err(StoreError::BrokenInvariant {
                reason: "converted invitation participants are invalid or duplicated".to_owned(),
            });
        }
        let missing_members = participants
            .iter()
            .filter(|participant| {
                !stored
                    .brain
                    .members
                    .iter()
                    .any(|member| member.user_id == participant.npub)
            })
            .count();
        if !invitation.folder_only
            && stored.brain.members.len().saturating_add(missing_members)
                > BRAIN_CAPACITY_ENVELOPE.members
        {
            return Err(StoreError::CapacityExceeded {
                limit: "brain_members".to_owned(),
                max: BRAIN_CAPACITY_ENVELOPE.members,
                current: stored.brain.members.len().saturating_add(missing_members),
            });
        }
        let scope = email_bootstrap_scope(
            &stored.brain,
            &invitation.initial_folder_access,
            invitation.folder_only,
        )?;
        let required = scope
            .iter()
            .flat_map(|folder| {
                participants.iter().map(move |participant| {
                    (
                        folder.folder_id.clone(),
                        folder.key_version,
                        participant.npub.clone(),
                    )
                })
            })
            .collect::<BTreeSet<_>>();
        let provided = grants
            .iter()
            .map(|grant| {
                (
                    grant.folder_id.clone(),
                    grant.key_version,
                    grant.recipient_npub.clone(),
                )
            })
            .collect::<BTreeSet<_>>();
        if provided != required || grants.len() != required.len() {
            return Err(StoreError::BrokenInvariant {
                reason:
                    "converted invitation grants must exactly cover every participant and Folder"
                        .to_owned(),
            });
        }
        for grant in grants {
            validate_grant_metadata(grant)?;
            validate_grant_issuer(
                &stored.brain,
                grant,
                stored
                    .personal_agent
                    .as_ref()
                    .map(|relationship| &relationship.agent_npub),
                has_brain_operational_authority(&stored, &grant.issuer_npub),
            )?;
            if grant.issuer_npub != *actor {
                return Err(StoreError::BrokenInvariant {
                    reason: "converted invitation grant issuer must be the conversion actor"
                        .to_owned(),
                });
            }
        }
        validate_folder_key_grant_control_records(grants, control_records)?;
        let participants_json =
            serde_json::to_string(participants).map_err(|error| StoreError::BrokenInvariant {
                reason: format!("cohort participants did not serialize: {error}"),
            })?;
        serde_json::from_str::<serde_json::Value>(exclusions_json).map_err(|_| {
            StoreError::BrokenInvariant {
                reason: "cohort exclusions must be JSON".to_owned(),
            }
        })?;
        serde_json::from_str::<serde_json::Value>(key_versions_json).map_err(|_| {
            StoreError::BrokenInvariant {
                reason: "cohort key versions must be JSON".to_owned(),
            }
        })?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        revoke_expired_pending_invitations_on(&tx, brain_id, converted_at)?;
        let changed = tx.execute(
            r#"
            UPDATE brain_invitations
            SET target_kind = 'account_cohort', user_id = ?2,
                bootstrap_wrapped_event_json = NULL, updated_at = ?3
            WHERE id = ?1 AND brain_id = ?4
              AND target_kind = 'email_bootstrap' AND status = 'pending'
            "#,
            params![
                invitation_id,
                human[0].npub.as_str(),
                converted_at,
                brain_id.as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::BrokenInvariant {
                reason: "pending invitation changed before conversion commit".to_owned(),
            });
        }
        tx.execute(
            r#"
            INSERT INTO cohort_invitation_plans (
                invitation_id, plan_id, account_id, human_email, roster_revision,
                scope_kind, folder_id, participants_json, exclusions_json,
                key_versions_json, actor_npub, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                invitation_id,
                plan_id,
                account_id,
                human_email,
                i64::try_from(roster_revision).map_err(|_| StoreError::BrokenInvariant {
                    reason: "roster revision exceeds SQLite integer range".to_owned(),
                })?,
                if invitation.folder_only {
                    "folder"
                } else {
                    "brain"
                },
                invitation
                    .folder_only
                    .then(|| invitation.initial_folder_access[0].as_str()),
                participants_json,
                exclusions_json,
                key_versions_json,
                actor.as_str(),
                converted_at,
            ],
        )?;
        for (grant, record) in grants.iter().zip(control_records) {
            let SyncRecordInput::Control(record) = record else {
                return Err(StoreError::BrokenInvariant {
                    reason: "converted invitation grants require control sync records".to_owned(),
                });
            };
            tx.execute(
                r#"
                INSERT INTO cohort_invitation_grants (
                    invitation_id, grant_id, folder_id, key_version, issuer_npub,
                    recipient_npub, format, wrapped_event_json, created_at,
                    record_event_id, record_payload_json, record_event_kind
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                "#,
                params![
                    invitation_id,
                    grant.id,
                    grant.folder_id.as_str(),
                    i64::from(grant.key_version),
                    grant.issuer_npub.as_str(),
                    grant.recipient_npub.as_str(),
                    grant.format,
                    grant.wrapped_event_json,
                    grant.created_at,
                    record.record_event_id,
                    record.payload_json,
                    i64::from(record.record_event_kind),
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO invitation_cohort_conversion_receipts (
                invitation_id, plan_id, backup_reference, converted_by_npub, converted_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                invitation_id,
                plan_id,
                backup_reference,
                actor.as_str(),
                converted_at,
            ],
        )?;
        enforce_invitation_capacity_on(&tx, brain_id, converted_at)?;
        tx.commit()?;
        self.load_brain_invitation(invitation_id)
    }

    pub fn load_cohort_invitation_plan(
        &self,
        invitation_id: &str,
    ) -> Result<Option<StoredCohortInvitationPlan>, StoreError> {
        self.conn
            .query_row(
                r#"
                SELECT invitation_id, plan_id, account_id, human_email, roster_revision,
                       scope_kind, folder_id, participants_json, exclusions_json,
                       key_versions_json, actor_npub, created_at
                FROM cohort_invitation_plans
                WHERE invitation_id = ?1
                "#,
                params![invitation_id],
                |row| {
                    let participants_json: String = row.get("participants_json")?;
                    let participants =
                        serde_json::from_str(&participants_json).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                participants_json.len(),
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?;
                    let roster_revision = row.get::<_, i64>("roster_revision")?;
                    Ok(StoredCohortInvitationPlan {
                        invitation_id: row.get("invitation_id")?,
                        plan_id: row.get("plan_id")?,
                        account_id: row.get("account_id")?,
                        human_email: row.get("human_email")?,
                        roster_revision: u64::try_from(roster_revision).map_err(|_| {
                            rusqlite::Error::IntegralValueOutOfRange(4, roster_revision)
                        })?,
                        scope_kind: row.get("scope_kind")?,
                        folder_id: row
                            .get::<_, Option<String>>("folder_id")?
                            .map(FolderId::new)
                            .transpose()
                            .map_err(|error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    6,
                                    rusqlite::types::Type::Text,
                                    Box::new(error),
                                )
                            })?,
                        participants,
                        exclusions_json: row.get("exclusions_json")?,
                        key_versions_json: row.get("key_versions_json")?,
                        actor_npub: UserId::new(row.get::<_, String>("actor_npub")?).map_err(
                            |error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    10,
                                    rusqlite::types::Type::Text,
                                    Box::new(error),
                                )
                            },
                        )?,
                        created_at: row.get("created_at")?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// Atomically claim the right to deliver one invitation email. Successful
    /// and in-flight claims deduplicate exact retries.
    pub fn begin_brain_invitation_email_delivery(
        &mut self,
        invitation_id: &str,
        attempted_at: &str,
    ) -> Result<bool, StoreError> {
        validate_link_timestamp("attemptedAt", attempted_at)?;
        self.load_brain_invitation(invitation_id)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let status = tx
            .query_row(
                "SELECT status FROM brain_invitation_email_deliveries WHERE invitation_id = ?1",
                params![invitation_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let should_send = match status.as_deref() {
            Some("sent" | "sending") => false,
            Some("failed") => {
                tx.execute(
                    "UPDATE brain_invitation_email_deliveries
                     SET status = 'sending', attempted_at = ?2, delivered_at = NULL, error = NULL
                     WHERE invitation_id = ?1",
                    params![invitation_id, attempted_at],
                )?;
                true
            }
            None => {
                tx.execute(
                    "INSERT INTO brain_invitation_email_deliveries
                     (invitation_id, status, attempted_at)
                     VALUES (?1, 'sending', ?2)",
                    params![invitation_id, attempted_at],
                )?;
                true
            }
            Some(_) => {
                return Err(StoreError::BrokenInvariant {
                    reason: "unknown invitation email delivery state".to_owned(),
                });
            }
        };
        tx.commit()?;
        Ok(should_send)
    }

    pub fn finish_brain_invitation_email_delivery(
        &mut self,
        invitation_id: &str,
        delivered: bool,
        completed_at: &str,
    ) -> Result<(), StoreError> {
        validate_link_timestamp("completedAt", completed_at)?;
        let changed = self.conn.execute(
            "UPDATE brain_invitation_email_deliveries
             SET status = ?2,
                 delivered_at = CASE WHEN ?3 THEN ?4 ELSE NULL END,
                 error = CASE WHEN ?3 THEN NULL ELSE 'delivery_failed' END
             WHERE invitation_id = ?1 AND status = 'sending'",
            params![
                invitation_id,
                if delivered { "sent" } else { "failed" },
                delivered,
                completed_at,
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::BrokenInvariant {
                reason: "invitation email delivery was not in flight".to_owned(),
            });
        }
        Ok(())
    }

    /// Return the one shared pending-invitation view for an included human or
    /// Agent Principal. Visibility is account view state, not invitation state.
    pub fn list_account_invitations(
        &self,
        actor: &UserId,
        include_hidden: bool,
    ) -> Result<Vec<(StoredBrainInvitation, bool)>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            r#"
            {BRAIN_INVITATION_SELECT}
            WHERE id IN (
                SELECT plans.invitation_id
                FROM cohort_invitation_plans plans,
                     json_each(plans.participants_json) participant
                WHERE json_extract(participant.value, '$.npub') = ?1
                  AND (?2 = 1 OR NOT EXISTS (
                      SELECT 1 FROM account_invitation_dismissals dismissals
                      WHERE dismissals.invitation_id = plans.invitation_id
                        AND dismissals.account_id = plans.account_id
                        AND dismissals.hidden = 1
                  ))
            )
              AND status = 'pending'
            ORDER BY created_at DESC, id DESC
            LIMIT {MAX_LINK_LIST_ROWS}
            "#
        ))?;
        let invitations = statement
            .query_map(params![actor.as_str(), i64::from(include_hidden)], |row| {
                brain_invitation_from_row(row)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        invitations
            .into_iter()
            .map(|invitation| {
                let hidden = self.conn.query_row(
                    r#"
                    SELECT COALESCE((
                        SELECT dismissals.hidden
                        FROM account_invitation_dismissals dismissals
                        JOIN cohort_invitation_plans plans
                          ON plans.invitation_id = dismissals.invitation_id
                         AND plans.account_id = dismissals.account_id
                        WHERE dismissals.invitation_id = ?1
                    ), 0)
                    "#,
                    params![invitation.id],
                    |row| row.get::<_, bool>(0),
                )?;
                Ok((invitation, hidden))
            })
            .collect()
    }

    pub fn set_account_invitation_hidden(
        &mut self,
        invitation_id: &str,
        actor: &UserId,
        hidden: bool,
        updated_at: &str,
    ) -> Result<(), StoreError> {
        validate_link_timestamp("updatedAt", updated_at)?;
        let plan = self.load_cohort_invitation_plan(invitation_id)?.ok_or(
            StoreError::UnavailableLink {
                kind: "account invitation",
            },
        )?;
        if !plan
            .participants
            .iter()
            .any(|participant| participant.npub == *actor)
        {
            return Err(StoreError::UnavailableLink {
                kind: "account invitation",
            });
        }
        let invitation = self.load_brain_invitation(invitation_id)?;
        if invitation.status != LinkStatus::Pending {
            return Err(StoreError::UnavailableLink {
                kind: "account invitation",
            });
        }
        self.conn.execute(
            r#"
            INSERT INTO account_invitation_dismissals
                (invitation_id, account_id, hidden, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(invitation_id, account_id) DO UPDATE SET
                hidden = excluded.hidden,
                updated_at = excluded.updated_at
            "#,
            params![invitation_id, plan.account_id, hidden, updated_at],
        )?;
        Ok(())
    }

    /// Create one npub-bound singleton Brain Invitation.
    #[allow(clippy::too_many_arguments)]
    pub fn create_brain_invitation(
        &mut self,
        brain_id: &BrainId,
        id: &str,
        user_id: &UserId,
        invite_code: &str,
        accept_path: &str,
        initial_folder_access: &[FolderId],
        created_by_npub: &UserId,
        expires_at: &str,
        created_at: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, created_by_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "brain invitations require brain operational authority".to_owned(),
            });
        }
        if self.member_exists(brain_id, user_id)? {
            return Err(StoreError::BrokenInvariant {
                reason: "target is already a brain member".to_owned(),
            });
        }
        validate_link_id("brain_invitation_id", id)?;
        validate_link_id("invite_code", invite_code)?;
        validate_bounded_offer_expiry(expires_at, created_at)?;
        for folder_id in initial_folder_access {
            ensure_folder_exists(&self.conn, brain_id, folder_id)?;
        }
        let initial_folder_access_json = folder_id_vec_json(initial_folder_access)?;

        self.conn
            .execute(
                r#"
                INSERT INTO brain_invitations (
                    id, brain_id, user_id, target_kind, status, invite_code, accept_path,
                    initial_folder_access_json, created_by_npub, expires_at,
                    created_at, updated_at, bootstrap_scope_json
                )
                VALUES (?1, ?2, ?3, 'npub', 'pending', ?4, ?5, ?6, ?7, ?8, ?9, ?9, '[]')
                "#,
                params![
                    id,
                    brain_id.as_str(),
                    user_id.as_str(),
                    invite_code,
                    accept_path,
                    initial_folder_access_json,
                    created_by_npub.as_str(),
                    expires_at,
                    created_at
                ],
            )
            .map_err(map_insert_error("brain_invitation_id", id))?;

        self.load_brain_invitation(id)
    }

    /// Create one email-targeted Brain Invitation with encrypted bootstrap material.
    #[allow(clippy::too_many_arguments)]
    pub fn create_email_brain_invitation(
        &mut self,
        brain_id: &BrainId,
        id: &str,
        invited_email: &str,
        invite_unwrap_npub: &UserId,
        bootstrap_payload_hash: &str,
        bootstrap_wrapped_event_json: &str,
        bootstrap_authorization_event_json: &str,
        invite_code: &str,
        accept_path: &str,
        selected_restricted_folder_access: &[FolderId],
        folder_only: bool,
        created_by_npub: &UserId,
        expires_at: &str,
        created_at: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let stored = self.load_brain(brain_id)?;
        let brain = &stored.brain;
        if !has_brain_operational_authority(&stored, created_by_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "email brain invitations require brain operational authority".to_owned(),
            });
        }
        validate_link_id("brain_invitation_id", id)?;
        validate_link_id("invite_code", invite_code)?;
        validate_bounded_offer_expiry(expires_at, created_at)?;
        let invited_email = canonical_invited_email(invited_email)?;
        validate_required_text("bootstrapPayloadHash", bootstrap_payload_hash)?;
        validate_required_text("bootstrapWrappedEventJson", bootstrap_wrapped_event_json)?;
        validate_required_text(
            "bootstrapAuthorizationEventJson",
            bootstrap_authorization_event_json,
        )?;
        let bootstrap_scope =
            email_bootstrap_scope(brain, selected_restricted_folder_access, folder_only)?;
        let initial_folder_access = bootstrap_scope
            .iter()
            .map(|scope| scope.folder_id.clone())
            .collect::<Vec<_>>();
        let initial_folder_access_json = folder_id_vec_json(&initial_folder_access)?;
        let bootstrap_scope_json = serde_json::to_string(&bootstrap_scope).map_err(|error| {
            StoreError::BrokenInvariant {
                reason: format!("email bootstrap scope did not serialize: {error}"),
            }
        })?;

        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            UPDATE brain_invitations
            SET status = 'revoked',
                bootstrap_wrapped_event_json = NULL,
                updated_at = ?3
            WHERE brain_id = ?1
              AND target_kind = 'email_bootstrap'
              AND invited_email = ?2
              AND status = 'pending'
              AND folder_only = ?4
              AND (folder_only = 0 OR initial_folder_access_json = ?5)
            "#,
            params![
                brain_id.as_str(),
                invited_email,
                created_at,
                i64::from(folder_only),
                initial_folder_access_json
            ],
        )?;

        tx.execute(
            r#"
                INSERT INTO brain_invitations (
                    id, brain_id, user_id, target_kind, invited_email, invite_unwrap_npub,
                    bootstrap_payload_hash, bootstrap_wrapped_event_json,
                    bootstrap_authorization_event_json, bootstrap_scope_json,
                    status, invite_code, accept_path, initial_folder_access_json,
                    created_by_npub, expires_at, created_at, updated_at, folder_only
                )
                VALUES (
                    ?1, ?2, NULL, 'email_bootstrap', ?3, ?4,
                    ?5, ?6, ?7, ?8,
                    'pending', ?9, ?10, ?11, ?12, ?13, ?14, ?14, ?15
                )
                "#,
            params![
                id,
                brain_id.as_str(),
                invited_email,
                invite_unwrap_npub.as_str(),
                bootstrap_payload_hash,
                bootstrap_wrapped_event_json,
                bootstrap_authorization_event_json,
                bootstrap_scope_json,
                invite_code,
                accept_path,
                initial_folder_access_json,
                created_by_npub.as_str(),
                expires_at,
                created_at,
                i64::from(folder_only)
            ],
        )
        .map_err(map_insert_error("brain_invitation_id", id))?;
        tx.commit()?;

        self.load_brain_invitation(id)
    }

    /// Load one Brain Invitation by id.
    pub fn load_brain_invitation(
        &self,
        invitation_id: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        self.conn
            .query_row(
                &format!("{BRAIN_INVITATION_SELECT} WHERE id = ?1"),
                params![invitation_id],
                brain_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "brain invitation",
            })
    }

    /// Load one Brain Invitation by invite code without applying recipient availability rules.
    pub fn load_brain_invitation_by_code(
        &self,
        invite_code: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        self.conn
            .query_row(
                &format!("{BRAIN_INVITATION_SELECT} WHERE invite_code = ?1"),
                params![invite_code],
                brain_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "brain invitation",
            })
    }

    /// List Brain Invitations for one Brain, newest first, bounded by MAX_LINK_LIST_ROWS.
    pub fn list_brain_invitations(
        &self,
        brain_id: &BrainId,
    ) -> Result<Vec<StoredBrainInvitation>, StoreError> {
        self.require_brain_exists(brain_id)?;
        let query = format!(
            "{BRAIN_INVITATION_SELECT} WHERE brain_id = ?1 ORDER BY created_at DESC, id LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&query)?;
        let rows = stmt.query_map(
            params![brain_id.as_str(), MAX_LINK_LIST_ROWS],
            brain_invitation_from_row,
        )?;
        let mut invitations = Vec::new();
        for row in rows {
            invitations.push(row?);
        }
        Ok(invitations)
    }

    fn tombstone_email_bootstrap_ciphertext(
        &mut self,
        invitation_id: &str,
        updated_at: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            r#"
            UPDATE brain_invitations
            SET bootstrap_wrapped_event_json = NULL,
                updated_at = ?2
            WHERE id = ?1 AND target_kind = 'email_bootstrap'
            "#,
            params![invitation_id, updated_at],
        )?;
        Ok(())
    }

    /// Load a pending Brain Invitation by invite code for its target user only.
    pub fn load_available_brain_invitation_by_code(
        &self,
        invite_code: &str,
        user_id: &UserId,
        now: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let invitation = self
            .conn
            .query_row(
                &format!("{BRAIN_INVITATION_SELECT} WHERE invite_code = ?1"),
                params![invite_code],
                brain_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "brain invitation",
            })?;
        if invitation.target_kind == BrainInvitationTargetKind::AccountCohort {
            let plan = self
                .load_cohort_invitation_plan(&invitation.id)?
                .ok_or_else(|| StoreError::BrokenInvariant {
                    reason: "account cohort invitation is missing its plan".to_owned(),
                })?;
            if invitation.status != LinkStatus::Pending
                || timestamp_expired(&invitation.expires_at, now)
                || !plan
                    .participants
                    .iter()
                    .any(|participant| participant.npub == *user_id)
            {
                return Err(StoreError::UnavailableLink {
                    kind: "brain invitation",
                });
            }
        } else {
            ensure_invitation_available(&invitation, user_id, now)?;
        }
        Ok(invitation)
    }

    /// Revoke a Brain Invitation delivery handle. Accepted membership is unchanged.
    pub fn revoke_brain_invitation(
        &mut self,
        brain_id: &BrainId,
        invitation_id: &str,
        actor_npub: &UserId,
        updated_at: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "brain invitation revocation requires brain operational authority"
                    .to_owned(),
            });
        }
        let invitation = self.load_brain_invitation(invitation_id)?;
        if invitation.brain_id != *brain_id {
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }
        if invitation.status != LinkStatus::Pending {
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }
        self.conn.execute(
            r#"
            UPDATE brain_invitations
            SET status = 'revoked',
                bootstrap_wrapped_event_json = CASE
                    WHEN target_kind = 'email_bootstrap' THEN NULL
                    ELSE bootstrap_wrapped_event_json
                END,
                updated_at = ?3
            WHERE brain_id = ?1 AND id = ?2
            "#,
            params![brain_id.as_str(), invitation_id, updated_at],
        )?;
        self.load_brain_invitation(invitation_id)
    }

    /// Accept a pending Brain Invitation, adding the target as a member exactly once.
    pub fn accept_brain_invitation_by_code(
        &mut self,
        invite_code: &str,
        user_id: &UserId,
        now: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let mut invitation = self
            .conn
            .query_row(
                &format!("{BRAIN_INVITATION_SELECT} WHERE invite_code = ?1"),
                params![invite_code],
                brain_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "brain invitation",
            })?;

        if invitation.target_kind != BrainInvitationTargetKind::Npub
            || invitation.user_id.as_ref() != Some(user_id)
        {
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }
        if invitation.status == LinkStatus::Accepted {
            invitation.duplicate_accept = true;
            return Ok(invitation);
        }
        ensure_invitation_available(&invitation, user_id, now)?;
        let already_member = self.member_exists(&invitation.brain_id, user_id)?;
        let brain = self.load_core_brain(&invitation.brain_id)?;
        let restricted_initial_folder_access = invitation
            .initial_folder_access
            .iter()
            .filter(|folder_id| {
                brain.folders.iter().any(|folder| {
                    folder.id == **folder_id && folder.access == FolderAccessMode::Restricted
                })
            })
            .cloned()
            .collect::<Vec<_>>();

        let tx = self.conn.transaction()?;
        insert_member_if_missing(&tx, &invitation.brain_id, user_id)?;
        for folder_id in restricted_initial_folder_access {
            insert_folder_access_if_missing(&tx, &invitation.brain_id, &folder_id, user_id)?;
            insert_folder_access_source(
                &tx,
                &invitation.brain_id,
                &folder_id,
                user_id,
                "invitation",
                &invitation.id,
                now,
            )?;
        }
        tx.execute(
            r#"
            UPDATE brain_invitations
            SET status = 'accepted', updated_at = ?3, accepted_at = ?3
            WHERE brain_id = ?1 AND id = ?2 AND status = 'pending'
            "#,
            params![invitation.brain_id.as_str(), invitation.id, now],
        )?;
        tx.commit()?;

        let mut invitation = self.load_brain_invitation(&invitation.id)?;
        invitation.duplicate_accept = already_member;
        Ok(invitation)
    }

    /// Accept one fixed account-level invitation as any included principal.
    /// Membership/access, encrypted grants, cohort provenance, delegated
    /// authority, audit, and invitation consumption commit together.
    pub fn accept_account_cohort_invitation_by_code(
        &mut self,
        invite_code: &str,
        actor: &UserId,
        removed_participants: &BTreeMap<UserId, String>,
        now: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let mut invitation = self
            .conn
            .query_row(
                &format!("{BRAIN_INVITATION_SELECT} WHERE invite_code = ?1"),
                params![invite_code],
                brain_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "account cohort invitation",
            })?;
        if invitation.target_kind != BrainInvitationTargetKind::AccountCohort {
            return Err(StoreError::UnavailableLink {
                kind: "account cohort invitation",
            });
        }
        let plan = self
            .load_cohort_invitation_plan(&invitation.id)?
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "account cohort invitation is missing its immutable plan".to_owned(),
            })?;
        if !plan
            .participants
            .iter()
            .any(|participant| participant.npub == *actor)
            || removed_participants.contains_key(actor)
        {
            return Err(StoreError::UnavailableLink {
                kind: "account cohort invitation",
            });
        }
        if removed_participants.keys().any(|removed| {
            !plan.participants.iter().any(|participant| {
                participant.relationship == "account_agent" && participant.npub == *removed
            })
        }) {
            return Err(StoreError::BrokenInvariant {
                reason: "acceptance narrowing may only remove approved account agents".to_owned(),
            });
        }
        if invitation.status == LinkStatus::Accepted {
            invitation.duplicate_accept = true;
            return Ok(invitation);
        }
        if invitation.status != LinkStatus::Pending
            || timestamp_expired(&invitation.expires_at, now)
        {
            return Err(StoreError::UnavailableLink {
                kind: "account cohort invitation",
            });
        }
        let stored = self.load_brain(&invitation.brain_id)?;
        let rows = self
            .conn
            .prepare(
                r#"
                SELECT grant_id, folder_id, key_version, issuer_npub, recipient_npub,
                       format, wrapped_event_json, created_at, record_event_id,
                       record_payload_json, record_event_kind
                FROM cohort_invitation_grants
                WHERE invitation_id = ?1
                ORDER BY folder_id, recipient_npub
                "#,
            )?
            .query_map(params![invitation.id], |row| {
                let folder_id =
                    FolderId::new(row.get::<_, String>("folder_id")?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                let key_version = row.get::<_, i64>("key_version")?;
                let issuer_npub =
                    UserId::new(row.get::<_, String>("issuer_npub")?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                let recipient_npub =
                    UserId::new(row.get::<_, String>("recipient_npub")?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            4,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                let created_at: String = row.get("created_at")?;
                let grant = FolderKeyGrantMetadata {
                    id: row.get("grant_id")?,
                    folder_id: folder_id.clone(),
                    key_version: u32::try_from(key_version)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(2, key_version))?,
                    issuer_npub: issuer_npub.clone(),
                    recipient_npub,
                    format: row.get("format")?,
                    wrapped_event_json: row.get("wrapped_event_json")?,
                    access_change_event_json: None,
                    created_at: created_at.clone(),
                };
                let record_kind = row.get::<_, i64>("record_event_kind")?;
                let record = SyncRecordInput::Control(ControlSyncRecord {
                    record_event_id: row.get("record_event_id")?,
                    record_type: SyncRecordType::FolderKeyGrant,
                    folder_id: Some(folder_id),
                    actor_npub: issuer_npub,
                    client_created_at: created_at,
                    payload_json: row.get("record_payload_json")?,
                    record_event_kind: u16::try_from(record_kind)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(10, record_kind))?,
                });
                Ok((grant, record))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let (grants, control_records): (Vec<_>, Vec<_>) = rows
            .into_iter()
            .filter(|(grant, _)| !removed_participants.contains_key(&grant.recipient_npub))
            .unzip();
        let current_versions = stored
            .brain
            .folders
            .iter()
            .map(|folder| (folder.id.clone(), folder.current_key_version))
            .collect::<BTreeMap<_, _>>();
        if grants
            .iter()
            .any(|grant| current_versions.get(&grant.folder_id).copied() != Some(grant.key_version))
        {
            return Err(StoreError::BrokenInvariant {
                reason: "account cohort invitation plan is stale for current Folder Key versions"
                    .to_owned(),
            });
        }
        validate_folder_key_grant_control_records(&grants, &control_records)?;
        let human = plan
            .participants
            .iter()
            .find(|participant| participant.relationship == "human")
            .ok_or_else(|| StoreError::BrokenInvariant {
                reason: "account cohort plan has no human participant".to_owned(),
            })?;
        let cohort_id = format!("cohort-{}", invitation.id);
        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO account_access_cohorts (
                id, brain_id, account_id, human_npub, human_email, scope_kind, folder_id,
                provenance_kind, provenance_id, roster_revision, status,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'invitation', ?8, ?9, 'active', ?10, ?10)
            "#,
            params![
                cohort_id,
                invitation.brain_id.as_str(),
                plan.account_id,
                human.npub.as_str(),
                plan.human_email,
                plan.scope_kind,
                plan.folder_id.as_ref().map(FolderId::as_str),
                invitation.id,
                i64::try_from(plan.roster_revision).map_err(|_| {
                    StoreError::BrokenInvariant {
                        reason: "roster revision exceeds SQLite integer range".to_owned(),
                    }
                })?,
                now,
            ],
        )?;
        for participant in &plan.participants {
            if let Some(reason) = removed_participants.get(&participant.npub) {
                tx.execute(
                    r#"
                    INSERT INTO account_access_cohort_participants (
                        cohort_id, participant_npub, relationship, nip05, display_name,
                        status, exclusion_reason, created_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, 'excluded', ?6, ?7, ?7)
                    "#,
                    params![
                        cohort_id,
                        participant.npub.as_str(),
                        participant.relationship,
                        participant.nip05,
                        participant.name,
                        reason,
                        now,
                    ],
                )?;
                tx.execute(
                    r#"
                    INSERT INTO account_access_cohort_exclusions (
                        cohort_id, participant_npub, folder_id, reason, active,
                        created_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)
                    "#,
                    params![
                        cohort_id,
                        participant.npub.as_str(),
                        plan.folder_id.as_ref().map_or("", FolderId::as_str),
                        reason,
                        now,
                    ],
                )?;
                continue;
            }
            if invitation.folder_only {
                let folder_id =
                    plan.folder_id
                        .as_ref()
                        .ok_or_else(|| StoreError::BrokenInvariant {
                            reason: "Folder cohort invitation has no Folder".to_owned(),
                        })?;
                insert_folder_access_if_missing(
                    &tx,
                    &invitation.brain_id,
                    folder_id,
                    &participant.npub,
                )?;
                insert_folder_access_source(
                    &tx,
                    &invitation.brain_id,
                    folder_id,
                    &participant.npub,
                    "invitation",
                    &invitation.id,
                    now,
                )?;
            } else {
                insert_member_if_missing(&tx, &invitation.brain_id, &participant.npub)?;
                for folder_id in &invitation.initial_folder_access {
                    insert_folder_access_if_missing(
                        &tx,
                        &invitation.brain_id,
                        folder_id,
                        &participant.npub,
                    )?;
                    insert_folder_access_source(
                        &tx,
                        &invitation.brain_id,
                        folder_id,
                        &participant.npub,
                        "invitation",
                        &invitation.id,
                        now,
                    )?;
                }
            }
            tx.execute(
                r#"
                INSERT INTO account_access_cohort_participants (
                    cohort_id, participant_npub, relationship, nip05, display_name,
                    status, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6)
                "#,
                params![
                    cohort_id,
                    participant.npub.as_str(),
                    participant.relationship,
                    participant.nip05,
                    participant.name,
                    now,
                ],
            )?;
            if !invitation.folder_only && participant.relationship == "account_agent" {
                tx.execute(
                    r#"
                    INSERT INTO human_anchored_agent_authorities (
                        cohort_id, brain_id, human_npub, agent_npub, status,
                        created_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5)
                    "#,
                    params![
                        cohort_id,
                        invitation.brain_id.as_str(),
                        human.npub.as_str(),
                        participant.npub.as_str(),
                        now,
                    ],
                )?;
            }
        }
        for (grant, record) in grants.iter().zip(&control_records) {
            let current_grant_exists = tx.query_row(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM folder_key_grants
                    WHERE brain_id = ?1 AND folder_id = ?2 AND key_version = ?3
                      AND recipient_npub = ?4
                )
                "#,
                params![
                    invitation.brain_id.as_str(),
                    grant.folder_id.as_str(),
                    i64::from(grant.key_version),
                    grant.recipient_npub.as_str(),
                ],
                |row| row.get::<_, bool>(0),
            )?;
            if !current_grant_exists {
                insert_grant(&tx, &invitation.brain_id, grant)?;
                sync_records::append_sync_records(
                    &tx,
                    &invitation.brain_id,
                    std::slice::from_ref(record),
                )?;
            }
        }
        tx.execute(
            r#"
            INSERT INTO account_access_cohort_audit (
                id, cohort_id, action, actor_npub, anchoring_human_npub,
                detail_json, occurred_at
            ) VALUES (?1, ?2, 'invitation_accepted', ?3, ?4, ?5, ?6)
            "#,
            params![
                format!("audit-{}-accepted", invitation.id),
                cohort_id,
                actor.as_str(),
                human.npub.as_str(),
                serde_json::json!({
                    "invitationId": invitation.id,
                    "participantCount": plan.participants.len() - removed_participants.len(),
                    "removedParticipants": removed_participants.iter().map(|(npub, reason)| {
                        serde_json::json!({ "npub": npub, "reason": reason })
                    }).collect::<Vec<_>>(),
                })
                .to_string(),
                now,
            ],
        )?;
        tx.execute(
            r#"
            UPDATE brain_invitations
            SET status = 'accepted', claimed_by_npub = ?3,
                updated_at = ?4, accepted_at = ?4
            WHERE brain_id = ?1 AND id = ?2 AND status = 'pending'
            "#,
            params![
                invitation.brain_id.as_str(),
                invitation.id,
                actor.as_str(),
                now,
            ],
        )?;
        tx.commit()?;
        self.load_brain_invitation(&invitation.id)
    }

    /// Test-only claim helper that synthesizes the signed-record metadata boundary.
    #[cfg(test)]
    pub fn claim_email_brain_invitation_by_code(
        &mut self,
        invite_code: &str,
        invited_email: &str,
        claimant: &UserId,
        grants: &[FolderKeyGrantMetadata],
        now: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let control_records = grants
            .iter()
            .map(|grant| {
                SyncRecordInput::Control(ControlSyncRecord {
                    record_event_id: format!("{}-test-claim-control", grant.id),
                    record_type: SyncRecordType::FolderKeyGrant,
                    folder_id: Some(grant.folder_id.clone()),
                    actor_npub: grant.issuer_npub.clone(),
                    client_created_at: grant.created_at.clone(),
                    payload_json: "{}".to_owned(),
                    record_event_kind: NIP59_GIFT_WRAP_KIND,
                })
            })
            .collect::<Vec<_>>();
        self.claim_email_brain_invitation_by_code_with_control_records(
            invite_code,
            invited_email,
            claimant,
            grants,
            &control_records,
            now,
        )
    }

    /// Claim a pending Email Invite Bootstrap and append every Folder Key
    /// Grant control record in the same transaction.
    pub fn claim_email_brain_invitation_by_code_with_control_records(
        &mut self,
        invite_code: &str,
        invited_email: &str,
        claimant: &UserId,
        grants: &[FolderKeyGrantMetadata],
        control_records: &[SyncRecordInput],
        now: &str,
    ) -> Result<StoredBrainInvitation, StoreError> {
        let mut invitation = self
            .conn
            .query_row(
                &format!("{BRAIN_INVITATION_SELECT} WHERE invite_code = ?1"),
                params![invite_code],
                brain_invitation_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "brain invitation",
            })?;

        if invitation.target_kind != BrainInvitationTargetKind::EmailBootstrap {
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }
        if invitation.status == LinkStatus::Accepted {
            if invitation.claimed_by_npub.as_ref() == Some(claimant) {
                invitation.duplicate_accept = true;
                return Ok(invitation);
            }
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }
        if invitation.status != LinkStatus::Pending {
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }
        if timestamp_expired(&invitation.expires_at, now) {
            self.tombstone_email_bootstrap_ciphertext(&invitation.id, now)?;
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }
        let invited_email = canonical_invited_email(invited_email)?;
        if invitation.invited_email.as_deref() != Some(invited_email.as_str()) {
            return Err(StoreError::UnavailableLink {
                kind: "brain invitation",
            });
        }

        let stored = self.load_brain(&invitation.brain_id)?;
        if email_bootstrap_scope_stale(&stored.brain, &invitation.bootstrap_scope)? {
            self.tombstone_email_bootstrap_ciphertext(&invitation.id, now)?;
            return Err(StoreError::BrokenInvariant {
                reason: "email bootstrap scope is stale for current Folder Key versions".to_owned(),
            });
        }
        validate_email_claim_grants(&stored.brain, &invitation.bootstrap_scope, claimant, grants)?;
        validate_folder_key_grant_control_records(grants, control_records)?;
        let invited_scope = invitation
            .bootstrap_scope
            .iter()
            .map(|scope| scope.folder_id.clone())
            .collect::<Vec<_>>();

        let tx = self.conn.transaction()?;
        if !invitation.folder_only {
            insert_member_if_missing(&tx, &invitation.brain_id, claimant)?;
        }
        for folder_id in invited_scope {
            insert_folder_access_if_missing(&tx, &invitation.brain_id, &folder_id, claimant)?;
            insert_folder_access_source(
                &tx,
                &invitation.brain_id,
                &folder_id,
                claimant,
                "invitation",
                &invitation.id,
                now,
            )?;
        }
        for grant in grants {
            insert_grant(&tx, &invitation.brain_id, grant)?;
        }
        sync_records::append_sync_records(&tx, &invitation.brain_id, control_records)?;
        tx.execute(
            r#"
            UPDATE brain_invitations
            SET status = 'accepted',
                user_id = ?3,
                claimed_by_npub = ?3,
                bootstrap_wrapped_event_json = NULL,
                updated_at = ?4,
                accepted_at = ?4
            WHERE brain_id = ?1 AND id = ?2 AND status = 'pending'
            "#,
            params![
                invitation.brain_id.as_str(),
                invitation.id,
                claimant.as_str(),
                now
            ],
        )?;
        tx.commit()?;

        self.load_brain_invitation(&invitation.id)
    }

    /// Create one npub-bound singleton Share Link for a restricted Folder.
    #[allow(clippy::too_many_arguments)]
    pub fn create_share_link(
        &mut self,
        brain_id: &BrainId,
        folder_id: &FolderId,
        id: &str,
        recipient_npub: &UserId,
        created_by_npub: &UserId,
        expires_at: &str,
        accept_path: &str,
        grant: &FolderKeyGrantMetadata,
        created_at: &str,
    ) -> Result<StoredShareLink, StoreError> {
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, created_by_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "Folder Invitations require Brain operational authority".to_owned(),
            });
        }
        let folder = stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == *folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: folder_id.to_string(),
            })?;
        if stored.setup_incomplete_folder_ids.contains(folder_id) {
            return Err(StoreError::BrokenInvariant {
                reason: "Folder Invitation source setup must be complete".to_owned(),
            });
        }
        validate_link_id("share_link_id", id)?;
        validate_bounded_offer_expiry(expires_at, created_at)?;
        validate_grant_metadata(grant)?;
        validate_grant_issuer(
            &stored.brain,
            grant,
            stored
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub),
            has_brain_operational_authority(&stored, &grant.issuer_npub),
        )?;
        if grant.folder_id != *folder_id
            || grant.key_version != folder.current_key_version
            || grant.recipient_npub != *recipient_npub
            || grant.issuer_npub != *created_by_npub
        {
            return Err(StoreError::BrokenInvariant {
                reason:
                    "share link grant must match folder, current key version, issuer, and recipient"
                        .to_owned(),
            });
        }
        let access_change_event_json =
            grant
                .access_change_event_json
                .clone()
                .ok_or_else(|| StoreError::BrokenInvariant {
                    reason: "Folder Invitation requires an access-change event".to_owned(),
                })?;

        self.conn
            .execute(
                r#"
                INSERT INTO share_links (
                    id, brain_id, folder_id, recipient_npub, created_by_npub, status,
                    accept_path, expires_at, created_at, updated_at, grant_id,
                    grant_key_version, grant_wrapped_event_json, access_change_event_json,
                    create_personal_mount
                )
                VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7, ?8, ?8, ?9, ?10, ?11, ?12, 0)
                "#,
                params![
                    id,
                    brain_id.as_str(),
                    folder_id.as_str(),
                    recipient_npub.as_str(),
                    created_by_npub.as_str(),
                    accept_path,
                    expires_at,
                    created_at,
                    grant.id,
                    grant.key_version,
                    grant.wrapped_event_json,
                    access_change_event_json
                ],
            )
            .map_err(map_insert_error("share_link_id", id))?;

        self.load_share_link(id)
    }

    /// Load one Share Link by id.
    pub fn load_share_link(&self, share_link_id: &str) -> Result<StoredShareLink, StoreError> {
        self.conn
            .query_row(
                r#"
                SELECT id, brain_id, folder_id, recipient_npub, created_by_npub, status,
                       accept_path, expires_at, created_at, updated_at, accepted_at,
                       grant_id, grant_key_version, grant_wrapped_event_json,
                       access_change_event_json
                FROM share_links
                WHERE id = ?1
                "#,
                params![share_link_id],
                share_link_from_row,
            )
            .optional()?
            .ok_or(StoreError::UnavailableLink {
                kind: "Folder Invitation",
            })
    }

    /// List Share Links for one Folder, newest first, bounded by MAX_LINK_LIST_ROWS.
    pub fn list_folder_share_links(
        &self,
        brain_id: &BrainId,
        folder_id: &FolderId,
    ) -> Result<Vec<StoredShareLink>, StoreError> {
        ensure_folder_exists(&self.conn, brain_id, folder_id)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, brain_id, folder_id, recipient_npub, created_by_npub, status,
                   accept_path, expires_at, created_at, updated_at, accepted_at,
                   grant_id, grant_key_version, grant_wrapped_event_json,
                   access_change_event_json
            FROM share_links
            WHERE brain_id = ?1 AND folder_id = ?2
            ORDER BY created_at DESC, id
            LIMIT ?3
            "#,
        )?;
        let rows = stmt.query_map(
            params![brain_id.as_str(), folder_id.as_str(), MAX_LINK_LIST_ROWS],
            share_link_from_row,
        )?;
        let mut share_links = Vec::new();
        for row in rows {
            share_links.push(row?);
        }
        Ok(share_links)
    }

    /// Load a pending Share Link for its recipient only.
    pub fn load_available_share_link(
        &self,
        share_link_id: &str,
        recipient_npub: &UserId,
        now: &str,
    ) -> Result<StoredShareLink, StoreError> {
        let share_link = self.load_share_link(share_link_id)?;
        ensure_share_link_available(&share_link, recipient_npub, now)?;
        Ok(share_link)
    }

    /// Revoke a Share Link delivery handle. Accepted access is unchanged.
    pub fn revoke_share_link(
        &mut self,
        share_link_id: &str,
        actor_npub: &UserId,
        updated_at: &str,
    ) -> Result<StoredShareLink, StoreError> {
        let share_link = self.load_share_link(share_link_id)?;
        if share_link.status != LinkStatus::Pending {
            return Err(StoreError::UnavailableLink {
                kind: "Folder Invitation",
            });
        }
        let stored = self.load_brain(&share_link.brain_id)?;
        if !has_brain_operational_authority(&stored, actor_npub) {
            return Err(StoreError::BrokenInvariant {
                reason: "Folder Invitation revocation requires Brain operational authority"
                    .to_owned(),
            });
        }
        self.conn.execute(
            "UPDATE share_links SET status = 'revoked', updated_at = ?2 WHERE id = ?1",
            params![share_link_id, updated_at],
        )?;
        self.load_share_link(share_link_id)
    }

    /// Accept a pending legacy Share Link as Folder-limited Guest access.
    pub fn accept_share_link(
        &mut self,
        share_link_id: &str,
        recipient_npub: &UserId,
        control_records: &[SyncRecordInput],
        now: &str,
    ) -> Result<StoredShareLink, StoreError> {
        let mut share_link = self.load_share_link(share_link_id)?;
        if share_link.recipient_npub != *recipient_npub {
            return Err(StoreError::UnavailableLink {
                kind: "Folder Invitation",
            });
        }
        if share_link.status == LinkStatus::Accepted {
            share_link.duplicate_accept = true;
            return Ok(share_link);
        }
        ensure_share_link_available(&share_link, recipient_npub, now)?;

        let stored = self.load_brain(&share_link.brain_id)?;
        let folder = stored
            .brain
            .folders
            .iter()
            .find(|folder| folder.id == share_link.folder_id)
            .ok_or_else(|| StoreError::MissingFolder {
                folder_id: share_link.folder_id.to_string(),
            })?;
        validate_grant_metadata(&share_link.folder_key_grant)?;
        validate_grant_issuer(
            &stored.brain,
            &share_link.folder_key_grant,
            stored
                .personal_agent
                .as_ref()
                .map(|relationship| &relationship.agent_npub),
            has_brain_operational_authority(&stored, &share_link.folder_key_grant.issuer_npub),
        )?;
        if share_link.folder_key_grant.key_version != folder.current_key_version {
            return Err(StoreError::BrokenInvariant {
                reason: "Folder Invitation grant key version must match Folder current key version"
                    .to_owned(),
            });
        }
        validate_folder_key_grant_control_records(
            std::slice::from_ref(&share_link.folder_key_grant),
            control_records,
        )?;

        let tx = self.conn.transaction()?;
        insert_folder_access_if_missing(
            &tx,
            &share_link.brain_id,
            &share_link.folder_id,
            recipient_npub,
        )?;
        insert_folder_access_source(
            &tx,
            &share_link.brain_id,
            &share_link.folder_id,
            recipient_npub,
            "invitation",
            &share_link.id,
            now,
        )?;
        insert_grant(&tx, &share_link.brain_id, &share_link.folder_key_grant)?;
        sync_records::append_sync_records(&tx, &share_link.brain_id, control_records)?;

        tx.execute(
            r#"
            UPDATE share_links
            SET status = 'accepted', updated_at = ?2, accepted_at = ?2
            WHERE id = ?1 AND status = 'pending'
            "#,
            params![share_link_id, now],
        )?;
        tx.commit()?;

        self.load_share_link(share_link_id)
    }
}
