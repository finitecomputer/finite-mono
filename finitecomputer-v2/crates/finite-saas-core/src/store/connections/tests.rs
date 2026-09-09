use super::*;
use crate::test_support::{TestDb, with_isolated_postgres};

fn key() -> Zeroizing<[u8; 32]> {
    let mut value = Zeroizing::new([0; 32]);
    getrandom::getrandom(value.as_mut()).unwrap();
    value
}
fn test_keys() -> ConnectionsKeys {
    ConnectionsKeys::new("key-1".into(), BTreeMap::from([("key-1".into(), key())])).unwrap()
}
fn input(id: &str, expected: i64) -> SetInference {
    SetInference {
        change_id: id.into(),
        expected_revision: expected,
        profile: InferenceProfile::Openrouter,
        model: Some("provider/model".into()),
        credential: InferenceCredentialChange::Keep,
    }
}
async fn seed(db: &TestDb) {
    db.exec("INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at) VALUES ('owner', 'owner@example.test', 'linked', 'workos-owner', now(), now()), ('other', 'other@example.test', 'linked', 'workos-other', now(), now());
        INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at) VALUES ('org','owner','Test','grandfathered',now(),now());
        INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, created_at, updated_at) VALUES ('project','org','owner','Test',now(),now()), ('other-project','org','other','Other',now(),now());").await;
}

#[tokio::test]
async fn postgres_connections_retry_after_newer_change_and_key_rotation() {
    with_isolated_postgres(|db| async move {
        seed(&db).await;
        let mut keys = test_keys();
        assert_eq!(
            db.connection_inference("workos-owner", "project")
                .await
                .unwrap(),
            None
        );
        let first = db
            .set_connection_inference("workos-owner", "project", input("first", 0), &keys)
            .await
            .unwrap();
        keys.keys.insert("key-2".into(), key());
        keys.active = "key-2".into();
        let second = db
            .set_connection_inference("workos-owner", "project", input("second", 1), &keys)
            .await
            .unwrap();
        let reopened = CoreStore::connect(&db.url).await.unwrap();
        assert_eq!(
            reopened
                .set_connection_inference("workos-owner", "project", input("first", 0), &keys)
                .await
                .unwrap(),
            first
        );
        assert_eq!(
            reopened
                .connection_inference("workos-owner", "project")
                .await
                .unwrap(),
            Some(second)
        );
        let mut changed = input("first", 0);
        changed.model = Some("different/model".into());
        assert!(matches!(
            db.set_connection_inference("workos-owner", "project", changed, &keys)
                .await,
            Err(ConnectionsError::ChangeConflict)
        ));
        assert!(matches!(
            db.set_connection_inference("workos-owner", "project", input("stale", 0), &keys)
                .await,
            Err(ConnectionsError::RevisionConflict)
        ));
        db.migrate().await.unwrap();
        assert_eq!(
            db.connection_inference("workos-owner", "project")
                .await
                .unwrap()
                .unwrap()
                .revision,
            2
        );
        assert!(
            db.connection_inference("workos-other", "other-project")
                .await
                .unwrap()
                .is_none()
        );
    })
    .await;
}

#[tokio::test]
async fn postgres_connections_concurrent_first_writes_have_one_winner() {
    with_isolated_postgres(|db| async move {
        seed(&db).await;
        let keys = test_keys();
        let (a, b) = tokio::join!(
            db.set_connection_inference("workos-owner", "project", input("a", 0), &keys),
            db.set_connection_inference("workos-owner", "project", input("b", 0), &keys)
        );
        assert!(matches!(
            (&a, &b),
            (Ok(_), Err(ConnectionsError::RevisionConflict))
                | (Err(ConnectionsError::RevisionConflict), Ok(_))
        ));
        let (a, b) = tokio::join!(
            db.set_connection_inference("workos-owner", "project", input("same", 1), &keys),
            db.set_connection_inference("workos-owner", "project", input("same", 1), &keys)
        );
        assert_eq!(a.unwrap(), b.unwrap());
        assert_eq!(
            db.query_json(
                "SELECT to_jsonb(count(*)) FROM connection_inference_changes",
                &[]
            )
            .await,
            vec![serde_json::json!(2)]
        );
    })
    .await;
}

#[tokio::test]
async fn postgres_connections_authorization_is_checked_before_replay() {
    with_isolated_postgres(|db| async move {
        seed(&db).await;
        let keys = test_keys();
        db.set_connection_inference("workos-owner", "project", input("first", 0), &keys)
            .await
            .unwrap();
        for (actor, project) in [
            ("workos-other", "project"),
            ("workos-owner", "other-project"),
            ("workos-owner", "missing"),
        ] {
            assert!(matches!(
                db.connection_inference(actor, project).await,
                Err(ConnectionsError::Unauthorized)
            ));
            assert!(matches!(
                db.set_connection_inference(actor, project, input("first", 0), &keys)
                    .await,
                Err(ConnectionsError::Unauthorized)
            ));
        }
        db.exec("UPDATE projects SET owner_user_id='other' WHERE id='project'")
            .await;
        assert!(matches!(
            db.set_connection_inference("workos-owner", "project", input("first", 0), &keys)
                .await,
            Err(ConnectionsError::Unauthorized)
        ));
        assert!(matches!(
            db.set_connection_inference("workos-other", "project", input("first", 0), &keys)
                .await,
            Err(ConnectionsError::ChangeConflict)
        ));
    })
    .await;
}

#[tokio::test]
async fn postgres_connections_encrypts_and_binds_credentials_without_leaking_to_receipts() {
    with_isolated_postgres(|db| async move {
        seed(&db).await;
        let keys = test_keys();
        let secret = Zeroizing::new(crate::generate_finite_private_api_key().unwrap());
        let mut request = input("first", 0);
        request.credential = InferenceCredentialChange::Replace { api_key: secret.clone() };
        let first = db.set_connection_inference("workos-owner", "project", request, &keys).await.unwrap();
        let client = db.connection().await.unwrap();
        let row = client.query_one("SELECT * FROM connection_inference_settings WHERE project_id='project'", &[]).await.unwrap();
        let nonce: Vec<u8> = row.get("credential_nonce");
        let ciphertext: Vec<u8> = row.get("credential_ciphertext");
        assert_ne!(ciphertext, secret.as_bytes());
        let cipher = XChaCha20Poly1305::new(keys.key("key-1").unwrap().into());
        let opened = Zeroizing::new(cipher.decrypt(XNonce::from_slice(&nonce), Payload {msg: &ciphertext, aad: &credential_context("project",1)}).unwrap());
        assert_eq!(opened.as_slice(), secret.as_bytes());
        for context in [credential_context("other-project",1),credential_context("project",2)] {
            assert!(cipher.decrypt(XNonce::from_slice(&nonce), Payload {msg: &ciphertext, aad: &context}).is_err());
        }
        let mut altered = ciphertext.clone(); altered[0] ^= 1;
        assert!(cipher.decrypt(XNonce::from_slice(&nonce), Payload {msg:&altered,aad:&credential_context("project",1)}).is_err());
        let wrong = test_keys();
        let wrong_cipher = XChaCha20Poly1305::new(wrong.key("key-1").unwrap().into());
        assert!(wrong_cipher.decrypt(XNonce::from_slice(&nonce), Payload {msg:&ciphertext,aad:&credential_context("project",1)}).is_err());
        let second = db.set_connection_inference("workos-owner", "project", input("keep",1), &keys).await.unwrap();
        assert_eq!(second.credential_version, first.credential_version);
        let after = client.query_one("SELECT credential_nonce, credential_ciphertext FROM connection_inference_settings WHERE project_id='project'", &[]).await.unwrap();
        assert_eq!(after.get::<_,Vec<u8>>(0), nonce);
        assert_eq!(after.get::<_,Vec<u8>>(1), ciphertext);
        let mut retry = input("first", 0);
        retry.credential = InferenceCredentialChange::Replace { api_key: secret.clone() };
        assert_eq!(db.set_connection_inference("workos-owner", "project", retry, &keys).await.unwrap(), first);
        let mut altered_request = input("first", 0);
        altered_request.credential = InferenceCredentialChange::Replace {
            api_key: Zeroizing::new(crate::generate_finite_private_api_key().unwrap()),
        };
        assert!(matches!(db.set_connection_inference("workos-owner", "project", altered_request, &keys).await, Err(ConnectionsError::ChangeConflict)));
        let mut replacement = input("replace", 2);
        replacement.credential = InferenceCredentialChange::Replace { api_key: secret.clone() };
        assert_eq!(db.set_connection_inference("workos-owner", "project", replacement, &keys).await.unwrap().credential_version, 2);
        let replacement_row = client.query_one("SELECT credential_nonce FROM connection_inference_settings WHERE project_id='project'", &[]).await.unwrap();
        assert_ne!(replacement_row.get::<_,Vec<u8>>(0), nonce);
        let receipt = db.query_json("SELECT to_jsonb(c) FROM connection_inference_changes c", &[]).await;
        assert!(!serde_json::to_string(&receipt).unwrap().contains(secret.as_str()));
        assert!(!serde_json::to_string(&second).unwrap().contains(secret.as_str()));
    }).await;
}

#[tokio::test]
async fn postgres_connections_failed_write_and_dry_run_leave_no_partial_state() {
    with_isolated_postgres(|db| async move {
        seed(&db).await;
        let keys = test_keys();
        let dry = CoreStore::connect_dry_run(&db.url).await.unwrap();
        dry.set_connection_inference("workos-owner","project",input("dry",0),&keys).await.unwrap();
        assert!(db.connection_inference("workos-owner","project").await.unwrap().is_none());
        db.exec("ALTER TABLE connection_inference_changes ADD CONSTRAINT reject_test_write CHECK (change_id <> 'reject')").await;
        assert!(db.set_connection_inference("workos-owner","project",input("reject",0),&keys).await.is_err());
        assert!(db.connection_inference("workos-owner","project").await.unwrap().is_none());
        assert_eq!(db.query_json("SELECT to_jsonb(count(*)) FROM connection_inference_changes", &[]).await, vec![serde_json::json!(0)]);
    }).await;
}

#[test]
fn connections_rejects_unknown_fields_and_invalid_credential_operations() {
    assert!(serde_json::from_value::<SetInference>(serde_json::json!({"changeId":"a","expectedRevision":0,"profile":"openrouter","credential":{"operation":"keep"},"command":"shell"})).is_err());
    let mut request = input("first", 0);
    request.credential = InferenceCredentialChange::Replace {
        api_key: Zeroizing::new(String::new()),
    };
    assert!(matches!(validate(&request), Err(ConnectionsError::Invalid)));
    request = input("first", 0);
    request.profile = InferenceProfile::FinitePrivate;
    assert!(matches!(validate(&request), Err(ConnectionsError::Invalid)));
    request = input("first", -1);
    assert!(matches!(validate(&request), Err(ConnectionsError::Invalid)));
}
