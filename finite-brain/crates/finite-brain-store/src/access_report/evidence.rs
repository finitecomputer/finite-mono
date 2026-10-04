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

pub(super) const AUTHENTICATED_ACTION_SQL: &str = "SELECT MIN(accepted_at) FROM brain_record_index
 WHERE brain_id = ?1 AND actor_npub = ?2";

impl BrainStore {
    /// Participation for `keys` only (one report page), three indexed point
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
                ParticipationKind::AuthenticatedBrainAction,
                AUTHENTICATED_ACTION_SQL,
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
            (AUTHENTICATED_ACTION_SQL, "brain_record_index_by_actor"),
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
}
