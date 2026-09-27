//! Brain-local display labels. These never participate in identity or access resolution.
use crate::*;

/// Principal label bound in UTF-8 bytes; schema V30 enforces the same limit.
pub const PRINCIPAL_LABEL_MAX_BYTES: usize = 320;
pub const PRINCIPAL_LABEL_PAGE_SIZE: usize = 256;

const PRINCIPAL_LABEL_SELECT: &str =
    "SELECT user_id, text, source, recorded_by, updated_at FROM brain_principal_labels";

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PrincipalLabel {
    pub text: String,
    pub source: PrincipalLabelSource,
    pub recorded_by: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PrincipalLabelSource {
    /// An admin's unverified note.
    AdminNote,
    /// The requested destination of a redeemed email Invite Token.
    InvitationEmail,
}

impl PrincipalLabelSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AdminNote => "admin_note",
            Self::InvitationEmail => "invitation_email",
        }
    }
}

impl TryFrom<&str> for PrincipalLabelSource {
    type Error = StoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "admin_note" => Ok(Self::AdminNote),
            "invitation_email" => Ok(Self::InvitationEmail),
            _ => Err(StoreError::BrokenInvariant {
                reason: format!("unknown principal label source {value}"),
            }),
        }
    }
}

/// One keyset page of labels ordered by principal.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PrincipalLabelPage {
    /// At most `PRINCIPAL_LABEL_PAGE_SIZE` labels keyed by principal.
    pub labels: BTreeMap<String, PrincipalLabel>,
    /// Cursor for the next page; present only when more labels remain.
    pub next_after: Option<String>,
}

impl BrainStore {
    /// Only an operational Brain admin can record an admin note, and only for
    /// a current principal. None clears the label. The canonical target is
    /// supplied by the signed HTTP request, never inferred from a roster
    /// position or a label.
    pub fn set_principal_label(
        &mut self,
        brain_id: &BrainId,
        actor: &UserId,
        target: &UserId,
        text: Option<&str>,
        now: &str,
    ) -> Result<(), StoreError> {
        let text = text.map(str::trim);
        if let Some(text) = text {
            validate_principal_label(text)?;
        }
        let stored = self.load_brain(brain_id)?;
        if !has_brain_operational_authority(&stored, actor) {
            return Err(StoreError::BrokenInvariant {
                reason: "principal labels require brain operational authority".to_owned(),
            });
        }
        let present: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM brain_members WHERE brain_id=?1 AND user_id=?2)
                OR EXISTS(SELECT 1 FROM folder_access WHERE brain_id=?1 AND user_id=?2)
                OR EXISTS(SELECT 1 FROM brains WHERE id=?1 AND owner_user_id=?2)
                OR EXISTS(SELECT 1 FROM personal_agents WHERE brain_id=?1 AND agent_npub=?2 AND status='active')",
            params![brain_id.as_str(), target.as_str()], |row| row.get(0),
        )?;
        if !present {
            return Err(StoreError::BrokenInvariant {
                reason: "principal labels require current Brain access".to_owned(),
            });
        }
        match text {
            Some(text) => {
                self.conn.execute(
                    "INSERT INTO brain_principal_labels (brain_id, user_id, text, source, recorded_by, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(brain_id, user_id) DO UPDATE SET text=excluded.text,
                     source=excluded.source, recorded_by=excluded.recorded_by, updated_at=excluded.updated_at",
                    params![
                        brain_id.as_str(),
                        target.as_str(),
                        text,
                        PrincipalLabelSource::AdminNote.as_str(),
                        actor.as_str(),
                        now
                    ],
                )?;
            }
            None => {
                self.conn.execute(
                    "DELETE FROM brain_principal_labels WHERE brain_id=?1 AND user_id=?2",
                    params![brain_id.as_str(), target.as_str()],
                )?;
            }
        }
        Ok(())
    }

    /// Read one page of labels after the `after` principal. `only_user`
    /// restricts the page to that principal's own label.
    pub fn principal_labels_page(
        &self,
        brain_id: &BrainId,
        only_user: Option<&UserId>,
        after: Option<&UserId>,
    ) -> Result<PrincipalLabelPage, StoreError> {
        let mut query = self.conn.prepare(&format!(
            "{PRINCIPAL_LABEL_SELECT} WHERE brain_id=?1 AND user_id>?2
             AND (?3 IS NULL OR user_id=?3) ORDER BY user_id LIMIT ?4"
        ))?;
        // One lookahead row decides whether another page exists.
        let rows = query.query_map(
            params![
                brain_id.as_str(),
                after.map_or("", UserId::as_str),
                only_user.map(UserId::as_str),
                (PRINCIPAL_LABEL_PAGE_SIZE + 1) as i64
            ],
            principal_label_from_row,
        )?;
        let mut labels = rows.collect::<Result<BTreeMap<_, _>, _>>()?;
        let next_after = if labels.len() > PRINCIPAL_LABEL_PAGE_SIZE {
            labels.pop_last();
            labels.keys().next_back().cloned()
        } else {
            None
        };
        Ok(PrincipalLabelPage { labels, next_after })
    }

    /// Read one principal's label.
    pub fn principal_label(
        &self,
        brain_id: &BrainId,
        user: &UserId,
    ) -> Result<Option<PrincipalLabel>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("{PRINCIPAL_LABEL_SELECT} WHERE brain_id=?1 AND user_id=?2"),
                params![brain_id.as_str(), user.as_str()],
                principal_label_from_row,
            )
            .optional()?
            .map(|(_, label)| label))
    }
}

fn principal_label_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<(String, PrincipalLabel), rusqlite::Error> {
    let source: String = row.get(2)?;
    Ok((
        row.get(0)?,
        PrincipalLabel {
            text: row.get(1)?,
            source: PrincipalLabelSource::try_from(source.as_str())
                .map_err(to_store_from_sql_error(2, rusqlite::types::Type::Text))?,
            recorded_by: row.get(3)?,
            updated_at: row.get(4)?,
        },
    ))
}

pub(crate) fn validate_principal_label(text: &str) -> Result<(), StoreError> {
    if text.is_empty()
        || text.len() > PRINCIPAL_LABEL_MAX_BYTES
        || text.chars().any(|ch| {
            ch.is_control() || matches!(ch, '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
    {
        return Err(StoreError::InvalidRecord {
            reason: format!(
                "principal label must contain 1-{PRINCIPAL_LABEL_MAX_BYTES} UTF-8 bytes without control characters"
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_upgrade_restore_and_clear_through_retained_writers() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("brain.sqlite3");
        let brain = BrainId::new("labels").unwrap();
        let admin = UserId::new("admin").unwrap();
        let member = UserId::new("member").unwrap();
        let mut store = BrainStore::open(&path).unwrap();
        store
            .create_brain_bootstrap(
                &finite_brain_core::bootstrap_organization_brain("labels", "Labels", "admin")
                    .unwrap(),
                &[],
            )
            .unwrap();
        store.add_member(&brain, &member).unwrap();
        // Retained V29 schema, populated before the additive migration.
        store
            .conn
            .execute_batch(
                "DROP TRIGGER label_invite_token_redeemer;
            DROP TRIGGER clear_member_principal_label;
            DROP TRIGGER clear_departed_guest_principal_label;
            DROP TABLE brain_principal_labels;
            ALTER TABLE brain_invite_tokens DROP COLUMN requested_email;
            DELETE FROM schema_migrations WHERE version=30;",
            )
            .unwrap();
        let before = store.load_brain(&brain).unwrap();
        drop(store);
        let mut store = BrainStore::open(&path).unwrap();
        assert_eq!(store.load_brain(&brain).unwrap(), before);
        assert!(
            store
                .principal_labels_page(&brain, None, None)
                .unwrap()
                .labels
                .is_empty()
        );
        assert!(
            store
                .set_principal_label(&brain, &member, &member, Some("spoof"), "now")
                .is_err()
        );
        assert!(
            store
                .set_principal_label(
                    &brain,
                    &admin,
                    &UserId::new("absent").unwrap(),
                    Some("absent"),
                    "now"
                )
                .is_err()
        );
        for bad in [
            "",
            "  ",
            "name\nforged output",
            "name\u{202e}spoof",
            &"😀".repeat(81),
            &"x".repeat(321),
        ] {
            assert!(
                store
                    .set_principal_label(&brain, &admin, &member, Some(bad), "now")
                    .is_err()
            );
        }
        store
            .set_principal_label(&brain, &admin, &member, Some("CK (human)"), "now")
            .unwrap();
        let labels = store
            .principal_labels_page(&brain, None, None)
            .unwrap()
            .labels;
        assert_eq!(labels["member"].source, PrincipalLabelSource::AdminNote);
        assert_eq!(labels["member"].recorded_by, "admin");
        assert_eq!(store.load_brain(&brain).unwrap(), before);
        assert!(
            store
                .load_identity_aliases(std::slice::from_ref(&member))
                .unwrap()
                .is_empty()
        );
        // A consistent whole-database Recovery Set restores on an empty target.
        let recovery = scratch.path().join("restored.sqlite3");
        store
            .conn
            .execute("VACUUM INTO ?1", [recovery.to_str().unwrap()])
            .unwrap();
        drop(store);
        let mut restored = BrainStore::open(&recovery).unwrap();
        assert_eq!(
            restored
                .principal_labels_page(&brain, None, None)
                .unwrap()
                .labels,
            labels
        );
        assert_eq!(restored.load_brain(&brain).unwrap(), before);
        restored
            .set_principal_label(&brain, &admin, &member, None, "later")
            .unwrap();
        assert!(
            restored
                .principal_labels_page(&brain, None, None)
                .unwrap()
                .labels
                .is_empty()
        );

        // Exact older membership SQL, with no new label API, must clear labels.
        let legacy = Connection::open(&path).unwrap();
        legacy
            .execute_batch(
                "PRAGMA foreign_keys=ON;
            DELETE FROM brain_members WHERE brain_id='labels' AND user_id='member';
            INSERT INTO brain_members (brain_id,user_id) VALUES ('labels','member');",
            )
            .unwrap();
        drop(legacy);
        let mut store = BrainStore::open(&path).unwrap();
        assert!(
            store
                .principal_labels_page(&brain, None, None)
                .unwrap()
                .labels
                .is_empty()
        );
        store.conn.execute_batch("INSERT INTO folders (brain_id,id,name,role,access,parent_folder_key,path,current_key_version,shared_folder_source,setup_incomplete,created_at)
            VALUES ('labels','one','One','folder','restricted','','One',1,0,0,'now'),
                   ('labels','two','Two','folder','restricted','','Two',1,0,0,'now');
            INSERT INTO folder_access (brain_id,folder_id,user_id) VALUES ('labels','one','guest'),('labels','two','guest');").unwrap();
        let guest = UserId::new("guest").unwrap();
        store
            .set_principal_label(&brain, &admin, &guest, Some("Guest"), "now")
            .unwrap();
        store
            .conn
            .execute(
                "DELETE FROM folder_access WHERE brain_id='labels' AND folder_id='one'",
                [],
            )
            .unwrap();
        assert!(
            store
                .principal_labels_page(&brain, None, None)
                .unwrap()
                .labels
                .contains_key("guest")
        );
        store
            .conn
            .execute(
                "DELETE FROM folder_access WHERE brain_id='labels' AND folder_id='two'",
                [],
            )
            .unwrap();
        assert!(
            store
                .principal_labels_page(&brain, None, None)
                .unwrap()
                .labels
                .is_empty()
        );
    }

    #[test]
    fn label_pages_are_bounded_keyset_pages_and_receipts_read_one_row() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let brain = BrainId::new("labels").unwrap();
        store
            .create_brain_bootstrap(
                &finite_brain_core::bootstrap_organization_brain("labels", "Labels", "admin")
                    .unwrap(),
                &[],
            )
            .unwrap();
        for index in 0..=PRINCIPAL_LABEL_PAGE_SIZE {
            store
                .conn
                .execute(
                    "INSERT INTO brain_principal_labels (brain_id,user_id,text,source,recorded_by,updated_at)
                     VALUES ('labels',?1,'label','admin_note','admin','now')",
                    [format!("user-{index:04}")],
                )
                .unwrap();
        }
        let first = store.principal_labels_page(&brain, None, None).unwrap();
        assert_eq!(first.labels.len(), PRINCIPAL_LABEL_PAGE_SIZE);
        let cursor = UserId::new(first.next_after.clone().unwrap()).unwrap();
        assert_eq!(first.labels.keys().next_back(), Some(&cursor.to_string()));
        let last = store
            .principal_labels_page(&brain, None, Some(&cursor))
            .unwrap();
        assert_eq!(last.labels.len(), 1);
        assert_eq!(last.next_after, None);
        assert!(
            !first
                .labels
                .contains_key(last.labels.keys().next().unwrap())
        );

        let viewer = UserId::new("user-0003").unwrap();
        let own = store
            .principal_labels_page(&brain, Some(&viewer), None)
            .unwrap();
        assert_eq!(own.labels.keys().collect::<Vec<_>>(), ["user-0003"]);
        assert_eq!(own.next_after, None);
        assert_eq!(
            store.principal_label(&brain, &viewer).unwrap().as_ref(),
            own.labels.get("user-0003")
        );
        assert_eq!(
            store
                .principal_label(&brain, &UserId::new("absent").unwrap())
                .unwrap(),
            None
        );
    }

    #[test]
    fn invitation_email_labels_actual_redeemer_without_claiming_identity_or_overwriting_notes() {
        let mut store = BrainStore::open_in_memory().unwrap();
        let brain = BrainId::new("labels").unwrap();
        let admin = UserId::new("admin").unwrap();
        let redeemer = UserId::new("forwarded-link-recipient").unwrap();
        store
            .create_brain_bootstrap(
                &finite_brain_core::bootstrap_organization_brain("labels", "Labels", "admin")
                    .unwrap(),
                &[],
            )
            .unwrap();
        let now = "2026-09-26T00:00:00.000Z";
        let expires = "2026-09-27T00:00:00.000Z";
        let first = "a".repeat(64);
        store
            .create_brain_invite_token(
                &brain,
                &first,
                BrainInviteTokenRole::Member,
                &admin,
                expires,
                now,
                Some("invitee@example.com"),
            )
            .unwrap();
        assert!(
            store
                .principal_labels_page(&brain, None, None)
                .unwrap()
                .labels
                .is_empty()
        );
        store
            .redeem_brain_invite_token(&first, &redeemer, now)
            .unwrap();
        let labels = store
            .principal_labels_page(&brain, None, None)
            .unwrap()
            .labels;
        assert_eq!(
            labels[redeemer.as_str()].source,
            PrincipalLabelSource::InvitationEmail
        );
        assert_eq!(labels[redeemer.as_str()].text, "invitee@example.com");
        assert!(
            store
                .load_identity_aliases(std::slice::from_ref(&redeemer))
                .unwrap()
                .is_empty()
        );
        store
            .set_principal_label(
                &brain,
                &admin,
                &redeemer,
                Some("Actual recipient (agent)"),
                now,
            )
            .unwrap();
        store
            .redeem_brain_invite_token(&first, &redeemer, now)
            .unwrap();
        // An older redeemer's SQL still records new email provenance atomically,
        // but cannot overwrite a deliberate admin note.
        let second = "b".repeat(64);
        store
            .create_brain_invite_token(
                &brain,
                &second,
                BrainInviteTokenRole::Member,
                &admin,
                expires,
                now,
                Some("another@example.com"),
            )
            .unwrap();
        store.conn.execute("UPDATE brain_invite_tokens SET redeemed_by_npub=?2, redeemed_at=?3 WHERE token_hash=?1 AND redeemed_by_npub IS NULL AND revoked_at IS NULL", params![second, redeemer.as_str(), now]).unwrap();
        assert_eq!(
            store
                .principal_labels_page(&brain, None, None)
                .unwrap()
                .labels[redeemer.as_str()]
            .text,
            "Actual recipient (agent)"
        );
        store
            .set_principal_label(&brain, &admin, &redeemer, None, now)
            .unwrap();
        store
            .redeem_brain_invite_token(&first, &redeemer, now)
            .unwrap();
        assert!(
            store
                .principal_labels_page(&brain, None, None)
                .unwrap()
                .labels
                .is_empty(),
            "retries must not resurrect a cleared note"
        );
    }
}
