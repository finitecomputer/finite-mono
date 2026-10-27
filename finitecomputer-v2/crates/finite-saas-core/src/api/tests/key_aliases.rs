//! Private key aliases: viewer isolation, strict inputs, upsert, clear and
//! the per-account bound, over HTTP against real Postgres.

use super::*;
use crate::account_labels::MAX_ALIASES_PER_ACCOUNT;
use crate::brain_identity::npub_for_hex;

pub(super) fn valid_key(seed: u8) -> String {
    let secp = secp256k1::Secp256k1::new();
    let secret = secp256k1::SecretKey::from_slice(&[seed; 32]).unwrap();
    let (key, _) = secret.x_only_public_key(&secp);
    key.serialize().iter().map(|b| format!("{b:02x}")).collect()
}

fn viewer(email: &str) -> Vec<(String, String)> {
    workos_headers(email, true, None)
}

async fn enroll(app: &Router, email: &str) {
    let (status, _) = send_json(app, "GET", "/api/core/v1/me", &viewer(email), None).await;
    assert_eq!(status, StatusCode::OK);
}

async fn lookup(app: &Router, email: &str, keys: &[String]) -> (StatusCode, serde_json::Value) {
    send_json(
        app,
        "POST",
        "/api/core/v1/me/key-aliases/lookup",
        &viewer(email),
        Some(serde_json::json!({ "publicKeys": keys })),
    )
    .await
}

async fn put(app: &Router, email: &str, key: &str, alias: &str) -> (StatusCode, serde_json::Value) {
    send_json(
        app,
        "PUT",
        &format!("/api/core/v1/me/key-aliases/{key}"),
        &viewer(email),
        Some(serde_json::json!({ "alias": alias })),
    )
    .await
}

#[tokio::test]
async fn aliases_are_private_to_the_verified_viewer() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), test_auth());
        let maya = valid_key(1);
        let maya_npub = npub_for_hex(&maya).unwrap();

        // No linked account: nothing to read, and writes enroll nobody.
        let (status, empty) = lookup(&app, "ana@example.org", std::slice::from_ref(&maya)).await;
        assert_eq!(
            (status, empty),
            (StatusCode::OK, serde_json::json!({"aliases": []}))
        );
        let (status, refused) = put(&app, "ana@example.org", &maya, "Maya").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        assert!(db.all("users").await.is_empty());

        enroll(&app, "ana@example.org").await;
        enroll(&app, "ben@example.org").await;
        let (status, saved) = put(&app, "ana@example.org", &maya_npub, "  Maya   (design) ").await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["alias"], "Maya (design)");
        assert_eq!(saved["publicKeyHex"], maya);
        assert_eq!(saved["npub"], maya_npub);

        // Same key by hex: the conflict branch replaces the one row.
        let (status, replaced) =
            put(&app, "ana@example.org", &maya.to_uppercase(), "Maya R.").await;
        assert_eq!(status, StatusCode::OK, "{replaced}");
        assert_eq!(replaced["alias"], "Maya R.");
        let rows = db
            .query_json("SELECT to_jsonb(a) FROM account_key_aliases a", &[])
            .await;
        assert_eq!(rows.len(), 1);

        // Ben sees nothing of Ana's, and his own alias is separate.
        let (_, bens) = lookup(&app, "ben@example.org", std::slice::from_ref(&maya_npub)).await;
        assert_eq!(bens, serde_json::json!({"aliases": []}));
        let (status, _) = put(&app, "ben@example.org", &maya, "Designer").await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send_json(
            &app,
            "DELETE",
            &format!("/api/core/v1/me/key-aliases/{maya}"),
            &viewer("ben@example.org"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        // Ana's survives Ben's clear, a fresh connection pool and reapplying
        // startup migrations, including the new alias schema.
        let reopened = CoreStore::connect(&db.url).await.unwrap();
        reopened.migrate().await.unwrap();
        let restarted = router(reopened, test_auth());
        let other = valid_key(2);
        let (status, anas) =
            lookup(&restarted, "ana@example.org", &[other, maya_npub.clone()]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(anas["aliases"].as_array().unwrap().len(), 1);
        assert_eq!(anas["aliases"][0]["alias"], "Maya R.");

        // Clear returns to no alias; clearing again is harmless.
        for _ in 0..2 {
            let (status, _) = send_json(
                &restarted,
                "DELETE",
                &format!("/api/core/v1/me/key-aliases/{maya_npub}"),
                &viewer("ana@example.org"),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT);
        }
        let (_, cleared) = lookup(&restarted, "ana@example.org", &[maya]).await;
        assert_eq!(cleared, serde_json::json!({"aliases": []}));
    })
    .await;
}

#[tokio::test]
async fn alias_inputs_are_strict_and_owner_context_cannot_be_forged() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), test_auth());
        enroll(&app, "ana@example.org").await;
        let key = valid_key(3);
        let path = format!("/api/core/v1/me/key-aliases/{key}");

        for alias in [
            String::new(),
            "   ".to_string(),
            "x".repeat(65),
            "line\nbreak".to_string(),
            "nul\u{0}".to_string(),
            "\u{202E}reversed".to_string(),
        ] {
            let (status, _) = put(&app, "ana@example.org", &key, &alias).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{alias:?}");
        }
        for bad_key in ["npub1short", &"f".repeat(64), "not-a-key", &key[..62]] {
            let (status, _) = put(&app, "ana@example.org", bad_key, "Maya").await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_key}");
            let (status, _) = lookup(&app, "ana@example.org", &[bad_key.to_string()]).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_key}");
        }
        let (status, _) = lookup(&app, "ana@example.org", &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let many = (1..=101).map(valid_key).collect::<Vec<_>>();
        let (status, _) = lookup(&app, "ana@example.org", &many).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // The body cannot name another viewer, and identity headers or a
        // missing bearer are refused before any write.
        let (status, _) = send_json(
            &app,
            "PUT",
            &path,
            &viewer("ana@example.org"),
            Some(serde_json::json!({"alias": "Maya", "viewerEmail": "ben@example.org"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let mut forged = viewer("ana@example.org");
        forged.push((
            WORKOS_EMAIL_HEADER.to_string(),
            "ben@example.org".to_string(),
        ));
        let (status, _) = send_json(
            &app,
            "PUT",
            &path,
            &forged,
            Some(serde_json::json!({"alias": "Maya"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = send_json(
            &app,
            "PUT",
            &path,
            &[],
            Some(serde_json::json!({"alias": "Maya"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let service = [("authorization".to_string(), format!("Bearer {TOKEN}"))];
        let (status, _) = send_json(
            &app,
            "PUT",
            &path,
            &service,
            Some(serde_json::json!({"alias": "Maya"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = send_json(
            &app,
            "PUT",
            &path,
            &viewer("ana@example.org"),
            Some(serde_json::json!({"alias": "x".repeat(4096)})),
        )
        .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(
            db.query_json("SELECT to_jsonb(a) FROM account_key_aliases a", &[])
                .await
                .is_empty()
        );
    })
    .await;
}

#[tokio::test]
async fn one_account_keeps_a_bounded_number_of_aliases() {
    with_isolated_postgres(|db| async move {
        let app = router(db.store.clone(), test_auth());
        enroll(&app, "ana@example.org").await;
        // Fill to one below the limit directly (synthetic keys), then reach
        // it through the route.
        db.query_json(
            "WITH filled AS (
               INSERT INTO account_key_aliases (viewer_user_id, public_key_hex, alias, created_at, updated_at)
               SELECT users.id, lpad(to_hex(n), 64, '0'), 'Alias ' || n, NOW(), NOW()
               FROM users, generate_series(1, $1::bigint - 1) AS n
               RETURNING 1)
             SELECT to_jsonb(count(*)) FROM filled",
            &[&MAX_ALIASES_PER_ACCOUNT],
        )
        .await;
        let (status, _) = put(&app, "ana@example.org", &valid_key(5), "Last one").await;
        assert_eq!(status, StatusCode::OK);
        let (status, refused) = put(&app, "ana@example.org", &valid_key(4), "One more").await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert_eq!(refused["error"], "alias_limit_reached");
        // Renaming an existing alias is still allowed at the limit.
        let (status, renamed) = put(&app, "ana@example.org", &valid_key(5), "Renamed").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(renamed["alias"], "Renamed");
        let count = db
            .query_json("SELECT to_jsonb(count(*)) FROM account_key_aliases", &[])
            .await;
        assert_eq!(count, vec![serde_json::json!(MAX_ALIASES_PER_ACCOUNT)]);
    })
    .await;
}
