//! Private display sharing; never an access or key-grant authority.
use crate::*;

impl BrainStore {
    /// Explicit choices override the default for target-accepted invitations.
    pub fn identity_label_shared_with_admins(
        &self,
        brain_id: &BrainId,
        user_id: &UserId,
    ) -> Result<bool, StoreError> {
        let preference = self.conn.query_row(
            "SELECT shared_with_admins FROM brain_identity_label_preferences WHERE brain_id = ?1 AND user_id = ?2",
            params![brain_id.as_str(), user_id.as_str()],
            |row| row.get::<_, bool>(0),
        ).optional()?;
        if let Some(shared) = preference {
            return Ok(shared);
        }
        Ok(self
            .member_provenance(brain_id, user_id)?
            .is_some_and(|origin| origin.origin_kind == ProvenanceOriginKind::Invitation))
    }

    /// The HTTP boundary supplies the authenticated principal, never a target
    /// selected by an admin. Retain no names or cross-product identity data.
    pub fn set_identity_label_sharing(
        &mut self,
        brain_id: &BrainId,
        actor: &UserId,
        shared: bool,
        now: &str,
    ) -> Result<(), StoreError> {
        self.require_brain_exists(brain_id)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let present: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM brain_members WHERE brain_id = ?1 AND user_id = ?2)
                OR EXISTS(SELECT 1 FROM folder_access WHERE brain_id = ?1 AND user_id = ?2)
                OR EXISTS(SELECT 1 FROM brains WHERE id = ?1 AND owner_user_id = ?2)
                OR EXISTS(SELECT 1 FROM personal_agents WHERE brain_id = ?1 AND agent_npub = ?2 AND status = 'active')",
            params![brain_id.as_str(), actor.as_str()],
            |row| row.get(0),
        )?;
        if !present {
            return Err(StoreError::BrokenInvariant {
                reason: "label sharing requires current Brain access".to_owned(),
            });
        }
        tx.execute(
            "INSERT INTO brain_identity_label_preferences (brain_id, user_id, shared_with_admins, updated_at)
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT(brain_id, user_id)
             DO UPDATE SET shared_with_admins = excluded.shared_with_admins, updated_at = excluded.updated_at",
            params![brain_id.as_str(), actor.as_str(), shared, now],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_preferences_upgrade_retained_state_and_reset_under_legacy_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brain.sqlite3");
        let brain = BrainId::new("labels").unwrap();
        let direct = UserId::new("direct").unwrap();
        let invited = UserId::new("invited").unwrap();
        let mut store = BrainStore::open(&path).unwrap();
        store
            .create_brain_bootstrap(
                &finite_brain_core::bootstrap_organization_brain("labels", "Labels", "owner")
                    .unwrap(),
                &[],
            )
            .unwrap();
        store.add_member(&brain, &direct).unwrap();
        store.add_member(&brain, &invited).unwrap();
        store
            .conn
            .execute(
                "UPDATE brain_members SET origin_kind='invitation' WHERE user_id='invited'",
                [],
            )
            .unwrap();
        // Retained V29 source, before display preferences existed.
        store
            .conn
            .execute_batch(
                "DROP TRIGGER clear_member_identity_label_preference;
            DROP TRIGGER clear_departed_guest_identity_label_preference;
            DROP TABLE brain_identity_label_preferences;
            DELETE FROM schema_migrations WHERE version=30;",
            )
            .unwrap();
        let before = store.load_brain(&brain).unwrap();
        drop(store);
        let mut store = BrainStore::open(&path).unwrap();
        assert_eq!(store.load_brain(&brain).unwrap().brain, before.brain);
        assert!(
            !store
                .identity_label_shared_with_admins(&brain, &direct)
                .unwrap()
        );
        assert!(
            store
                .identity_label_shared_with_admins(&brain, &invited)
                .unwrap()
        );
        assert!(
            store
                .set_identity_label_sharing(&brain, &UserId::new("absent").unwrap(), true, "now")
                .is_err()
        );
        store
            .set_identity_label_sharing(&brain, &direct, true, "now")
            .unwrap();
        store
            .set_identity_label_sharing(&brain, &invited, false, "now")
            .unwrap();
        drop(store);
        let store = BrainStore::open(&path).unwrap();
        assert!(
            store
                .identity_label_shared_with_admins(&brain, &direct)
                .unwrap()
        );
        assert!(
            !store
                .identity_label_shared_with_admins(&brain, &invited)
                .unwrap()
        );
        assert_eq!(store.load_brain(&brain).unwrap().brain, before.brain);
        drop(store);
        // Exact retained deletion/insertion forms, without any new preference
        // API: an older writer must also clear sharing when membership ends.
        let legacy_writer = Connection::open(&path).unwrap();
        legacy_writer
            .execute_batch(
                "DELETE FROM brain_members WHERE brain_id='labels' AND user_id='direct';
            INSERT INTO brain_members (brain_id, user_id) VALUES ('labels', 'direct');",
            )
            .unwrap();
        drop(legacy_writer);
        let mut store = BrainStore::open(&path).unwrap();
        assert!(
            !store
                .identity_label_shared_with_admins(&brain, &direct)
                .unwrap()
        );
        // A guest can share while any explicit Folder access remains. The last
        // retained access-row deletion clears the choice across binary rollback.
        store.conn.execute_batch("INSERT INTO folders (brain_id,id,name,role,access,parent_folder_key,path,current_key_version,shared_folder_source,setup_incomplete,created_at)
            VALUES ('labels','one','One','folder','restricted','','One',1,0,0,'now'),
                   ('labels','two','Two','folder','restricted','','Two',1,0,0,'now');
            INSERT INTO folder_access (brain_id,folder_id,user_id) VALUES ('labels','one','guest'),('labels','two','guest');").unwrap();
        let guest = UserId::new("guest").unwrap();
        store
            .set_identity_label_sharing(&brain, &guest, true, "now")
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
                .identity_label_shared_with_admins(&brain, &guest)
                .unwrap()
        );
        store
            .conn
            .execute(
                "DELETE FROM folder_access WHERE brain_id='labels' AND folder_id='two'",
                [],
            )
            .unwrap();
        assert!(
            !store
                .identity_label_shared_with_admins(&brain, &guest)
                .unwrap()
        );
    }
}
