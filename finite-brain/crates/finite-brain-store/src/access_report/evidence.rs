//! Participation evidence: did this exact key act on this Brain itself?
//!
//! Admin-written membership, guest, or grant rows naming a key are not
//! participation by that key. Each lookup is an indexed point read for one
//! page key; nothing scans or groups the Brain's whole history.

use crate::*;

/// How a key proved it acted on this Brain itself.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub enum ParticipationKind {
    /// The key redeemed a capability Invite Token for this Brain.
    InviteTokenRedemption,
    /// The key accepted a Brain Invitation addressed to that exact npub.
    InvitationAcceptance,
    /// The exact recipient accepted a Folder Invitation in this Brain.
    FolderInvitationAcceptance,
    /// The addressed destination controller accepted a Mount Offer from this
    /// source Brain. Other initial participants did not accept the offer.
    MountOfferAcceptance,
    /// A Brain record was accepted from a request authenticated (NIP-98) by
    /// this key. This is request authentication, not a durable signature over
    /// the stored record: Folder Key Grant records are ephemeral-key gift
    /// wraps.
    AuthenticatedBrainAction,
}

impl ParticipationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InviteTokenRedemption => "inviteTokenRedemption",
            Self::InvitationAcceptance => "invitationAcceptance",
            Self::FolderInvitationAcceptance => "folderInvitationAcceptance",
            Self::MountOfferAcceptance => "mountOfferAcceptance",
            Self::AuthenticatedBrainAction => "authenticatedBrainAction",
        }
    }
}

/// Earliest recorded participation by one exact key across every kind,
/// ordered by parsed RFC 3339 time. A row whose time does not parse is not
/// evidence, so an unreadable legacy row can only withhold name lookup.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ParticipationEvidence {
    pub kind: ParticipationKind,
    pub recorded_at: String,
}

pub(super) const TOKEN_REDEMPTION_SQL: &str = "SELECT MIN(redeemed_at) FROM brain_invite_tokens
 WHERE brain_id = ?1 AND redeemed_by_npub = ?2 AND redeemed_at IS NOT NULL";

/// Only npub-addressed invitations: the accepting key must equal the
/// addressed key. Email-bootstrap claims and any legacy row whose claimant
/// differs are not evidence.
pub(super) const INVITATION_ACCEPTANCE_SQL: &str = "SELECT MIN(accepted_at) FROM brain_invitations
 WHERE brain_id = ?1 AND user_id = ?2 AND status = 'accepted' AND target_kind = 'npub'
   AND accepted_at IS NOT NULL
   AND (claimed_by_npub IS NULL OR claimed_by_npub = user_id)";

// Older writers could revoke an accepted offer while retaining accepted_at.
// Revocation removes authority, not the historical fact that this exact key
// accepted. Never-accepted revoked offers have no acceptance timestamp.
pub(super) const FOLDER_INVITATION_ACCEPTANCE_SQL: &str = "SELECT MIN(accepted_at) FROM share_links
 WHERE brain_id = ?1 AND recipient_npub = ?2
   AND status IN ('accepted', 'revoked') AND accepted_at IS NOT NULL";

// Scope is deliberately the source Brain. Do not infer participation for a
// Personal Brain's automatically added owner/agent or other Mount members.
pub(super) const MOUNT_OFFER_ACCEPTANCE_SQL: &str =
    "SELECT MIN(accepted_at) FROM shared_folder_invitations
 WHERE source_brain_id = ?1 AND destination_admin_npub = ?2
   AND status IN ('accepted', 'revoked') AND accepted_at IS NOT NULL";

pub(super) const AUTHENTICATED_ACTION_SQL: &str = "SELECT MIN(accepted_at) FROM brain_record_index
 WHERE brain_id = ?1 AND actor_npub = ?2";

/// An applied Approval (for example a hosted admin approving a
/// delegation-grant) records its exact signer. Approval targets are not
/// signers and gain nothing here.
pub(super) const APPLIED_APPROVAL_SQL: &str = "SELECT MIN(applied_at) FROM brain_approval_nonces
 WHERE brain_id = ?1 AND signer_npub = ?2";

impl BrainStore {
    /// Participation for `keys` only (one report page), six indexed point
    /// reads per key.
    pub(super) fn participation_for_keys(
        &self,
        brain_id: &BrainId,
        keys: &BTreeSet<UserId>,
    ) -> Result<BTreeMap<UserId, ParticipationEvidence>, StoreError> {
        let sources = [
            (
                ParticipationKind::InviteTokenRedemption,
                TOKEN_REDEMPTION_SQL,
            ),
            (
                ParticipationKind::InvitationAcceptance,
                INVITATION_ACCEPTANCE_SQL,
            ),
            (
                ParticipationKind::FolderInvitationAcceptance,
                FOLDER_INVITATION_ACCEPTANCE_SQL,
            ),
            (
                ParticipationKind::MountOfferAcceptance,
                MOUNT_OFFER_ACCEPTANCE_SQL,
            ),
            (
                ParticipationKind::AuthenticatedBrainAction,
                AUTHENTICATED_ACTION_SQL,
            ),
            (
                ParticipationKind::AuthenticatedBrainAction,
                APPLIED_APPROVAL_SQL,
            ),
        ];
        let mut statements = Vec::with_capacity(sources.len());
        for (kind, sql) in sources {
            statements.push((kind, self.conn.prepare(sql)?));
        }
        let mut evidence = BTreeMap::new();
        for key in keys {
            let mut earliest: Option<(OffsetDateTime, ParticipationEvidence)> = None;
            for (kind, statement) in &mut statements {
                let recorded_at = statement
                    .query_row(params![brain_id.as_str(), key.as_str()], |row| {
                        row.get::<_, Option<String>>(0)
                    })?;
                let Some(recorded_at) = recorded_at else {
                    continue;
                };
                let Ok(at) = OffsetDateTime::parse(&recorded_at, &Rfc3339) else {
                    continue;
                };
                if earliest.as_ref().is_none_or(|(current, _)| at < *current) {
                    earliest = Some((
                        at,
                        ParticipationEvidence {
                            kind: *kind,
                            recorded_at,
                        },
                    ));
                }
            }
            if let Some((_, found)) = earliest {
                evidence.insert(key.clone(), found);
            }
        }
        Ok(evidence)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::query_plan;
    use super::*;

    #[test]
    fn participation_lookups_are_indexed_point_reads() {
        let store = BrainStore::open_in_memory().unwrap();
        for (sql, index) in [
            (TOKEN_REDEMPTION_SQL, "brain_invite_tokens_by_redeemer"),
            (
                INVITATION_ACCEPTANCE_SQL,
                "brain_invitations_accepted_by_user",
            ),
            (
                FOLDER_INVITATION_ACCEPTANCE_SQL,
                "share_links_accepted_by_recipient",
            ),
            (
                MOUNT_OFFER_ACCEPTANCE_SQL,
                "shared_folder_invitations_accepted_by_controller",
            ),
            (AUTHENTICATED_ACTION_SQL, "brain_record_index_by_actor"),
            (APPLIED_APPROVAL_SQL, "brain_approval_nonces_by_signer"),
        ] {
            let plan = query_plan(&store, sql, &[&"brain", &"npub1synthetic"]);
            assert!(
                plan.iter().all(|detail| !detail.starts_with("SCAN")),
                "{sql}: {plan:?}"
            );
            assert!(
                plan.iter().any(|detail| detail.contains(index)),
                "{sql}: {plan:?}"
            );
            assert!(!sql.contains("GROUP BY"));
        }
    }
    #[test]
    fn brain_invitation_lookup_work_is_constant_as_accepted_history_grows() {
        // Real create/accept/remove cycles: accepted rows are retained and
        // are not bounded by the pending-invitation capacity counter.
        let mut store = BrainStore::open_in_memory().unwrap();
        let output =
            finite_brain_core::bootstrap_organization_brain("source", "Source", "npub-admin")
                .unwrap();
        store.create_brain_bootstrap(&output, &[]).unwrap();
        let brain = BrainId::new("source").unwrap();
        let admin = UserId::new("npub-admin").unwrap();
        let recipient = UserId::new("npub-recipient").unwrap();
        let at = "2026-06-23T00:00:00Z";
        let measure = |store: &BrainStore| {
            let mut statement = store.conn.prepare(INVITATION_ACCEPTANCE_SQL).unwrap();
            let found: Option<String> = statement
                .query_row(params!["source", "npub-recipient"], |row| row.get(0))
                .unwrap();
            assert_eq!(found.as_deref(), Some(at));
            statement.get_status(rusqlite::StatementStatus::VmStep)
        };
        let mut cycles = 0;
        let mut baseline = None;
        for target in [1, 100, 1001] {
            while cycles < target {
                let id = format!("invitation-history-{cycles}");
                let code = format!("history-code-{cycles}");
                store
                    .create_brain_invitation(
                        &brain,
                        &id,
                        &recipient,
                        &code,
                        "/accept",
                        &[],
                        &admin,
                        "2026-06-30T00:00:00Z",
                        at,
                    )
                    .unwrap();
                store
                    .accept_brain_invitation_by_code(&code, &recipient, at)
                    .unwrap();
                store.remove_member(&brain, &recipient).unwrap();
                cycles += 1;
            }
            let steps = measure(&store);
            match baseline {
                None => baseline = Some(steps),
                Some(first) => assert_eq!(
                    steps, first,
                    "{target} accepted invitations must not add scan work"
                ),
            }
        }
    }

    #[test]
    fn acceptance_lookup_work_is_constant_as_same_key_history_grows() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let output =
            finite_brain_core::bootstrap_organization_brain("source", "Source", "npub-admin")
                .unwrap();
        store.create_brain_bootstrap(&output, &[]).unwrap();
        // Synthetic historic offers, with real schema, foreign keys, triggers,
        // and production indexes. No acceptance or authority writer is changed.
        store.conn.execute_batch("INSERT INTO folders
            (brain_id, id, name, role, access, parent_folder_id, parent_folder_key, path, current_key_version, shared_folder_source, setup_incomplete, created_at)
            VALUES ('source', 'folder', 'Folder', 'folder', 'restricted', NULL, '', 'Folder', 1, 0, 0, '2026-06-23T00:00:00Z');").unwrap();
        let measure = |store: &BrainStore, sql: &str| {
            let mut statement = store.conn.prepare(sql).unwrap();
            let found: Option<String> = statement
                .query_row(params!["source", "npub-recipient"], |row| row.get(0))
                .unwrap();
            assert_eq!(found.as_deref(), Some("2026-06-23T00:00:00Z"));
            statement.get_status(rusqlite::StatementStatus::VmStep)
        };
        let mut baseline = Vec::new();
        for count in [1, 1000] {
            for index in 0..count {
                let id = format!("history-{count}-{index}");
                let status = if index % 2 == 0 {
                    "accepted"
                } else {
                    "revoked"
                };
                store.conn.execute("INSERT INTO share_links
                    (id, brain_id, folder_id, recipient_npub, created_by_npub, status,
                     accept_path, expires_at, created_at, updated_at, accepted_at,
                     grant_id, grant_key_version, grant_wrapped_event_json, access_change_event_json,
                     create_personal_mount)
                    VALUES (?1, 'source', 'folder', 'npub-recipient', 'npub-admin', ?2,
                     '/accept', '2026-06-30T00:00:00Z', '2026-06-23T00:00:00Z', '2026-06-23T00:00:00Z',
                     '2026-06-23T00:00:00Z', ?1, 1, '{}', '{}', 0)", params![id, status]).unwrap();
                store.conn.execute("INSERT INTO shared_folder_invitations
                    (id, source_brain_id, source_folder_id, destination_brain_id, destination_admin_npub,
                     created_by_npub, status, current_key_version, accept_path, created_at, updated_at,
                     accepted_at, grant_id, grant_wrapped_event_json, access_change_event_json, expires_at)
                    VALUES (?1, 'source', 'folder', 'source', 'npub-recipient', 'npub-admin', ?2, 1,
                     '/accept', '2026-06-23T00:00:00Z', '2026-06-23T00:00:00Z', '2026-06-23T00:00:00Z',
                     ?1, '{}', '{}', '2026-06-30T00:00:00Z')", params![id, status]).unwrap();
            }
            let steps = [FOLDER_INVITATION_ACCEPTANCE_SQL, MOUNT_OFFER_ACCEPTANCE_SQL]
                .map(|sql| measure(&store, sql));
            if count == 1 {
                baseline.extend(steps);
            } else {
                assert_eq!(
                    steps.as_slice(),
                    baseline.as_slice(),
                    "same-key accepted history must not add scan work"
                );
            }
        }
    }
}
