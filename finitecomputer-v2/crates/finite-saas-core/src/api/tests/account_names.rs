//! WorkOS account names: captured from the verified user Core already
//! fetches, cleared when WorkOS drops them, and released only on human
//! descriptions Core already releases. Aliases never reach descriptions.

use super::brain_identity::{
    DESCRIPTION_TOKEN, OBSERVATION_TOKEN, description_body, identity_app, key, observation_body,
    post,
};
use super::*;
use crate::auth::test_support::access_token_with_profile;
use crate::brain_identity::{
    DESCRIPTION_CREDENTIAL_HEADER, DESCRIPTIONS_PATH, DESCRIPTIONS_VERSION_V2,
    OBSERVATION_CREDENTIAL_HEADER, OBSERVATION_PATH,
};
use serde_json::{Value, json};

const SUBJECT: &str = "user_named_account";
const EMAIL: &str = "ada@acme.example";

fn signed_in(first: Option<Value>, last: Option<Value>) -> String {
    format!(
        "Bearer {}",
        access_token_with_profile(SUBJECT, EMAIL, first, last)
    )
}

async fn me(app: &Router, bearer: &str) -> StatusCode {
    let headers = [("authorization".to_string(), bearer.to_string())];
    send_json(app, "GET", "/api/core/v1/me", &headers, None)
        .await
        .0
}

async fn stored_name(db: &crate::test_support::TestDb) -> Value {
    db.query_json(
        "SELECT to_jsonb(workos_display_name) FROM users WHERE normalized_email = $1",
        &[&EMAIL],
    )
    .await
    .into_iter()
    .next()
    .unwrap_or(Value::Null)
}

async fn describe(app: &Router, version: &str, brain_id: &str) -> Value {
    let mut body = description_body(&[key(1), key(2)]);
    body["version"] = json!(version);
    body["brainId"] = json!(brain_id);
    let describe = [(DESCRIPTION_CREDENTIAL_HEADER, DESCRIPTION_TOKEN.to_string())];
    let (status, response) = post(
        app,
        DESCRIPTIONS_PATH,
        &describe,
        serde_json::to_vec(&body).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    response
}

#[tokio::test]
async fn workos_names_follow_the_verified_user_and_reach_human_descriptions() {
    with_isolated_postgres(|db| async move {
        let account = router(db.store.clone(), test_auth());
        let identity = identity_app(db.store.clone());

        let bearer = signed_in(Some(json!(" Ada ")), Some(json!("Lovelace")));
        assert_eq!(me(&account, &bearer).await, StatusCode::OK);
        assert_eq!(stored_name(&db).await, json!("Ada Lovelace"));

        // Ada's hosted key acts in brain_alpha; Bob has an email but no name.
        let observe = [
            (OBSERVATION_CREDENTIAL_HEADER, OBSERVATION_TOKEN.to_string()),
            ("authorization", bearer.clone()),
        ];
        let body = serde_json::to_vec(&observation_body("ada", &key(1))).unwrap();
        let (status, outcome) = post(&identity, OBSERVATION_PATH, &observe, body).await;
        assert_eq!(status, StatusCode::OK, "{outcome}");
        let bob = workos_headers("bob@acme.example", true, None);
        assert_eq!(
            send_json(&account, "GET", "/api/core/v1/me", &bob, None)
                .await
                .0,
            StatusCode::OK
        );
        let bob_observe = [
            (OBSERVATION_CREDENTIAL_HEADER, OBSERVATION_TOKEN.to_string()),
            ("authorization", bob[0].1.clone()),
        ];
        let body = serde_json::to_vec(&observation_body("bob", &key(2))).unwrap();
        assert_eq!(
            post(&identity, OBSERVATION_PATH, &bob_observe, body)
                .await
                .0,
            StatusCode::OK
        );

        // An alias Bob keeps for Ada's key is private and never described.
        let (status, _) = send_json(
            &account,
            "PUT",
            &format!(
                "/api/core/v1/me/key-aliases/{}",
                super::key_aliases::valid_key(9)
            ),
            &bob,
            Some(json!({"alias": "Not Ada"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        for version in [
            crate::brain_identity::DESCRIPTIONS_VERSION,
            DESCRIPTIONS_VERSION_V2,
        ] {
            let response = describe(&identity, version, "brain_alpha").await;
            let ada = &response["results"][0];
            assert_eq!(ada["state"], "resolved", "{response}");
            assert_eq!(ada["kind"], "human");
            assert_eq!(ada["displayName"], "Ada Lovelace");
            assert_eq!(ada["accountEmail"], EMAIL);
            assert_eq!(ada["source"]["kind"], "hostedDeviceObservation");
            let bob = &response["results"][1];
            assert_eq!(bob["accountEmail"], "bob@acme.example");
            assert!(bob.get("displayName").is_none(), "{bob}");
        }
        // The v1 sharing gate withholds the name with the email.
        let other_brain = describe(
            &identity,
            crate::brain_identity::DESCRIPTIONS_VERSION,
            "brain_beta",
        )
        .await;
        assert_eq!(
            other_brain["results"][0],
            json!({"publicKeyHex": key(1), "state": "notShared"})
        );

        // A WorkOS rename replaces the name on the next verified request.
        assert_eq!(
            me(
                &account,
                &signed_in(Some(json!("Ada")), Some(json!("King")))
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(stored_name(&db).await, json!("Ada King"));
        let response = describe(&identity, DESCRIPTIONS_VERSION_V2, "brain_alpha").await;
        assert_eq!(response["results"][0]["displayName"], "Ada King");

        // Malformed or removed names never block sign-in; they clear the
        // stored name, and the description falls back to email only.
        for (first, last) in [
            (Some(json!({"given": "Ada"})), Some(json!("King"))),
            (Some(json!("A".repeat(500))), None),
            (Some(json!("Ada\u{0}")), None),
            (None, None),
            (Some(json!("   ")), Some(Value::Null)),
        ] {
            assert_eq!(
                me(
                    &account,
                    &signed_in(Some(json!("Ada")), Some(json!("King")))
                )
                .await,
                StatusCode::OK
            );
            assert_eq!(stored_name(&db).await, json!("Ada King"));
            assert_eq!(
                me(&account, &signed_in(first.clone(), last.clone())).await,
                StatusCode::OK
            );
            assert_eq!(stored_name(&db).await, Value::Null, "{first:?} {last:?}");
        }
        let response = describe(&identity, DESCRIPTIONS_VERSION_V2, "brain_alpha").await;
        let ada = &response["results"][0];
        assert_eq!(ada["state"], "resolved");
        assert_eq!(ada["accountEmail"], EMAIL);
        assert!(ada.get("displayName").is_none(), "{ada}");
    })
    .await;
}

#[tokio::test]
async fn unchanged_names_are_not_rewritten() {
    // Its own WorkOS subject: the test registry is shared across tests.
    const SUBJECT: &str = "user_named_unchanged";
    const EMAIL: &str = "grace@acme.example";
    with_isolated_postgres(|db| async move {
        let account = router(db.store.clone(), test_auth());
        let bearer = format!(
            "Bearer {}",
            access_token_with_profile(SUBJECT, EMAIL, Some(json!("Grace")), Some(json!("Hopper")))
        );
        assert_eq!(me(&account, &bearer).await, StatusCode::OK);
        let read = |column: &'static str| {
            let db = &db;
            async move {
                db.query_json(
                    &format!("SELECT to_jsonb({column}) FROM users WHERE normalized_email = $1"),
                    &[&EMAIL],
                )
                .await
            }
        };
        let first = read("workos_display_name_updated_at").await;
        assert_ne!(first, vec![Value::Null]);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(me(&account, &bearer).await, StatusCode::OK);
        assert_eq!(read("workos_display_name_updated_at").await, first);
        // A link that carries no name observation leaves the name alone.
        db.store
            .link_verified_user(LinkVerifiedUserInput {
                verified_email: EMAIL.to_string(),
                workos_user_id: SUBJECT.to_string(),
                now: None,
            })
            .await
            .unwrap();
        assert_eq!(
            read("workos_display_name").await,
            vec![json!("Grace Hopper")]
        );
    })
    .await;
}
