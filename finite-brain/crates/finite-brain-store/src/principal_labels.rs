//! Brain-local display notes. These never participate in identity or access resolution.
use crate::*;

#[derive(Debug, Clone, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrincipalLabel {
    pub text: String,
    pub source: PrincipalLabelSource,
    pub recorded_by: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalLabelSource {
    AdminNote,
    InvitationEmail,
}

pub const PRINCIPAL_LABEL_PAGE_SIZE: usize = 256;

impl BrainStore {
    /// Only an operational Brain admin can set a note, and only for a current
    /// principal. None clears the note. The canonical target is supplied by the
    /// signed HTTP request, never inferred from a roster position or a note.
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
                     VALUES (?1, ?2, ?3, 'admin_note', ?4, ?5)
                     ON CONFLICT(brain_id, user_id) DO UPDATE SET text=excluded.text,
                     source=excluded.source, recorded_by=excluded.recorded_by, updated_at=excluded.updated_at",
                    params![brain_id.as_str(), target.as_str(), text, actor.as_str(), now],
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

    /// Read at most one page plus lookahead, optionally restricted to one viewer.
    pub fn principal_labels_page(
        &self,
        brain_id: &BrainId,
        visible_user: Option<&UserId>,
        after: &str,
    ) -> Result<BTreeMap<String, PrincipalLabel>, StoreError> {
        let mut query = self.conn.prepare(
            "SELECT user_id, text, source, recorded_by, updated_at
             FROM brain_principal_labels WHERE brain_id=?1 AND user_id>?2
             AND (?3 IS NULL OR user_id=?3) ORDER BY user_id LIMIT ?4",
        )?;
        let rows = query.query_map(
            params![
                brain_id.as_str(),
                after,
                visible_user.map(UserId::as_str),
                (PRINCIPAL_LABEL_PAGE_SIZE + 1) as i64
            ],
            |row| {
                let source: String = row.get(2)?;
                let source = match source.as_str() {
                    "admin_note" => PrincipalLabelSource::AdminNote,
                    "invitation_email" => PrincipalLabelSource::InvitationEmail,
                    _ => return Err(rusqlite::Error::InvalidQuery),
                };
                Ok((
                    row.get(0)?,
                    PrincipalLabel {
                        text: row.get(1)?,
                        source,
                        recorded_by: row.get(3)?,
                        updated_at: row.get(4)?,
                    },
                ))
            },
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

pub(crate) fn validate_principal_label(text: &str) -> Result<(), StoreError> {
    if text.is_empty()
        || text.len() > 320
        || text.chars().any(|ch| {
            ch.is_control() || matches!(ch, '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
    {
        return Err(StoreError::InvalidRecord {
            reason: "principal label must contain 1-320 UTF-8 bytes without control characters"
                .to_owned(),
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
                .principal_labels_page(&brain, None, "")
                .unwrap()
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
        let labels = store.principal_labels_page(&brain, None, "").unwrap();
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
            restored.principal_labels_page(&brain, None, "").unwrap(),
            labels
        );
        assert_eq!(restored.load_brain(&brain).unwrap(), before);
        restored
            .set_principal_label(&brain, &admin, &member, None, "later")
            .unwrap();
        assert!(
            restored
                .principal_labels_page(&brain, None, "")
                .unwrap()
                .is_empty()
        );

        // Exact older membership SQL, with no new label API, must clear notes.
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
                .principal_labels_page(&brain, None, "")
                .unwrap()
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
                .principal_labels_page(&brain, None, "")
                .unwrap()
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
                .principal_labels_page(&brain, None, "")
                .unwrap()
                .is_empty()
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
                .principal_labels_page(&brain, None, "")
                .unwrap()
                .is_empty()
        );
        store
            .redeem_brain_invite_token(&first, &redeemer, now)
            .unwrap();
        let labels = store.principal_labels_page(&brain, None, "").unwrap();
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
            store.principal_labels_page(&brain, None, "").unwrap()[redeemer.as_str()].text,
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
                .principal_labels_page(&brain, None, "")
                .unwrap()
                .is_empty(),
            "retries must not resurrect a cleared note"
        );
    }
}
