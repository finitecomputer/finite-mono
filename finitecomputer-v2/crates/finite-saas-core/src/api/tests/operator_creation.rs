//! Operator creation of an Agent for an account named by email, through the
//! admin route: authorization, input checks, retries, the confirm flag and
//! provisioning failures. The sign-in proof is in `operator_sign_in`.

use super::*;
use crate::auth::test_support::{registry_put_user, session_token_for};
use crate::owner_identity::test_support::{
    TestOwnerIdentityBackend, test_chat_account_id, test_provisioner,
};
use std::sync::Arc;

pub(super) const PATH: &str = "/api/core/v1/admin/operator-agent-creations";
const ADMIN_SUBJECT: &str = "workos_operator_admin";
const ADMIN_EMAIL: &str = "admin@finite.vip";

pub(super) fn admin() -> Vec<(String, String)> {
    vec![(
        "authorization".to_string(),
        format!(
            "Bearer {}",
            access_token_with_subject(ADMIN_SUBJECT, ADMIN_EMAIL, true, Some(OPERATOR_ORG_ID))
        ),
    )]
}

pub(super) fn service() -> Vec<(String, String)> {
    vec![("authorization".to_string(), format!("Bearer {TOKEN}"))]
}

/// A session for an existing WorkOS registry user, as AuthKit issues it.
pub(super) fn session(subject: &str) -> Vec<(String, String)> {
    vec![(
        "authorization".to_string(),
        format!("Bearer {}", session_token_for(subject, None)),
    )]
}

pub(super) fn create_body(owner_email: &str, key: &str) -> serde_json::Value {
    serde_json::json!({
        "ownerEmail": owner_email,
        "displayName": "Frankie Bot",
        "reason": "box1 migration: Frankie's bot moves to a finite-mono Agent",
        "idempotencyKey": key,
    })
}

/// The admin router with a test owner-identity backend on Core's WorkOS
/// test registry, and the admin's own account linked as the dashboard
/// links it on first load.
pub(super) async fn operator_app(
    db: &crate::test_support::TestDb,
) -> (Router, Arc<TestOwnerIdentityBackend>) {
    let (provisioner, backend) = test_provisioner();
    let app = crate::api::router_with_owner_identity(db.store.clone(), test_auth(), provisioner);
    let (status, _) = send_json(&app, "GET", "/api/core/v1/me", &admin(), None).await;
    assert_eq!(status, StatusCode::OK);
    (app, backend)
}

pub(super) fn project_ids(me: &serde_json::Value) -> Vec<String> {
    me["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|project| project["project"]["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn operator_retry_with_the_same_key_never_creates_a_second_agent() {
    with_isolated_postgres(|db| async move {
        let (app, backend) = operator_app(&db).await;
        let body = create_body("op-retry@example.test", "box1-retry-1");
        let (status, first) = send_json(&app, "POST", PATH, &admin(), Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        let (status, second) = send_json(&app, "POST", PATH, &admin(), Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{second}");
        assert_eq!(second["reused"], true);
        assert_eq!(second["record"], first["record"]);
        assert_eq!(second["request"]["id"], first["request"]["id"]);
        assert_eq!(db.all("projects").await.len(), 1);
        assert_eq!(db.all("agent_creation_requests").await.len(), 1);
        assert_eq!(db.all("operator_agent_creations").await.len(), 1);
        // The admission never writes the entitlement Stripe sync owns.
        assert!(db.all("agent_creation_entitlements").await.is_empty());
        assert_eq!(backend.created().len(), 1, "one WorkOS user, reused");

        // The same key with any different input fails closed.
        for (field, value) in [
            ("displayName", serde_json::json!("Other Bot")),
            (
                "ownerEmail",
                serde_json::json!("op-someone-else@example.test"),
            ),
            ("reason", serde_json::json!("a different reason")),
            ("hostingTier", serde_json::json!("confidential")),
        ] {
            let mut changed = body.clone();
            changed[field] = value;
            let (status, refused) = send_json(&app, "POST", PATH, &admin(), Some(changed)).await;
            assert_eq!(status, StatusCode::CONFLICT, "{field}: {refused}");
        }
        assert_eq!(db.all("projects").await.len(), 1);

        // A new key is a deliberate second Agent for the same person, bound
        // to the same WorkOS user and chat account.
        let (status, other) = send_json(
            &app,
            "POST",
            PATH,
            &admin(),
            Some(create_body("op-retry@example.test", "box1-retry-2")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{other}");
        assert_eq!(other["record"]["owner"], {
            let mut owner = first["record"]["owner"].clone();
            owner["accountCreated"] = serde_json::json!(false);
            owner["customerOrgCreated"] = serde_json::json!(false);
            owner
        });
        assert_ne!(other["project"]["id"], first["project"]["id"]);
        assert_eq!(backend.created().len(), 1);
    })
    .await;
}

#[tokio::test]
async fn operator_creation_is_admin_only_and_needs_the_hosted_device() {
    with_isolated_postgres(|db| async move {
        let (app, _) = operator_app(&db).await;
        let body = create_body("op-victim@example.test", "not-admin-1");
        // A linked, verified account outside the operator org is refused
        // at the route, before Core looks at who it is.
        let member = identity_headers("member@example.com", "true");
        let (status, _) = send_json(&app, "GET", "/api/core/v1/me", &member, None).await;
        assert_eq!(status, StatusCode::OK);
        for (headers, expected) in [
            (member.clone(), StatusCode::FORBIDDEN),
            (Vec::new(), StatusCode::UNAUTHORIZED),
        ] {
            let (status, _) = send_json(&app, "POST", PATH, &headers, Some(body.clone())).await;
            assert_eq!(status, expected);
            let (status, _) = send_json(&app, "GET", PATH, &headers, None).await;
            assert_eq!(status, expected);
        }
        // An operator-org session whose account Core has never linked is
        // refused too: the audit names one existing admin account.
        let unlinked = vec![(
            "authorization".to_string(),
            format!(
                "Bearer {}",
                access_token_with_subject(
                    "workos_unlinked_admin",
                    "unlinked-admin@finite.vip",
                    true,
                    Some(OPERATOR_ORG_ID)
                )
            ),
        )];
        let (status, refused) = send_json(&app, "POST", PATH, &unlinked, Some(body.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");

        // Without the hosted chat device Core cannot lock chat to the owner,
        // so it refuses instead of creating an open Agent.
        let unconfigured = admin_router(db.store.clone());
        let (status, refused) = send_json(&unconfigured, "POST", PATH, &admin(), Some(body)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{refused}");
        assert!(db.all("projects").await.is_empty());
        assert!(db.all("operator_agent_creations").await.is_empty());
        assert!(
            db.all("users")
                .await
                .iter()
                .all(|user| user["normalized_email"] != "op-victim@example.test")
        );
    })
    .await;
}

#[tokio::test]
async fn operator_creation_validates_input_and_target_host() {
    with_isolated_postgres(|db| async move {
        let (app, backend) = operator_app(&db).await;
        for owner_email in [
            "no-at-sign",
            "two@@example.com",
            "a b@example.com",
            "@example.com",
        ] {
            let (status, _) = send_json(
                &app,
                "POST",
                PATH,
                &admin(),
                Some(create_body(owner_email, "invalid-email")),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{owner_email}");
        }
        let mut no_reason = create_body("op-person@example.test", "no-reason");
        no_reason["reason"] = serde_json::json!("   ");
        let (status, _) = send_json(&app, "POST", PATH, &admin(), Some(no_reason)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let mut long_reason = create_body("op-person@example.test", "long-reason");
        long_reason["reason"] = serde_json::json!("x".repeat(2_001));
        let (status, _) = send_json(&app, "POST", PATH, &admin(), Some(long_reason)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let long_key = create_body("op-person@example.test", &"k".repeat(129));
        let (status, _) = send_json(&app, "POST", PATH, &admin(), Some(long_key)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let mut unknown_host = create_body("op-person@example.test", "unknown-host");
        unknown_host["targetSourceHostId"] = serde_json::json!("not-a-registered-host");
        let (status, _) = send_json(&app, "POST", PATH, &admin(), Some(unknown_host)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        // The chat account is never taken from the body, nor is anything
        // else the route does not name.
        for (field, value) in [
            ("ownerChatAccountId", serde_json::json!("ab".repeat(32))),
            ("launchCode", serde_json::json!("ignored?")),
        ] {
            let mut unknown = create_body("op-person@example.test", "unknown-field");
            unknown[field] = value;
            let (status, _) = send_json(&app, "POST", PATH, &admin(), Some(unknown)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{field}");
        }
        assert!(db.all("projects").await.is_empty());
        assert_eq!(db.all("users").await.len(), 1, "only the admin exists");
        assert!(
            backend.created().is_empty(),
            "nothing was created at WorkOS"
        );
    })
    .await;
}

#[tokio::test]
async fn adding_an_agent_to_a_signed_in_account_needs_the_confirm_flag() {
    with_isolated_postgres(|db| async move {
        let (app, backend) = operator_app(&db).await;
        let subject = "workos_op_known";
        registry_put_user(subject, "op-known@example.test", true);
        let (status, _) = send_json(&app, "GET", "/api/core/v1/me", &session(subject), None).await;
        assert_eq!(status, StatusCode::OK);

        let body = create_body("op-known@example.test", "known-1");
        let (status, refused) = send_json(&app, "POST", PATH, &admin(), Some(body.clone())).await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert!(
            refused["error"]
                .as_str()
                .unwrap_or_default()
                .contains("confirm"),
            "{refused}"
        );
        assert!(backend.chat_reads.lock().unwrap().is_empty());
        assert!(db.all("operator_agent_creations").await.is_empty());

        let mut confirmed = body;
        confirmed["existingAccountConfirmed"] = serde_json::json!(true);
        let (status, created) = send_json(&app, "POST", PATH, &admin(), Some(confirmed)).await;
        assert_eq!(status, StatusCode::OK, "{created}");
        assert_eq!(created["ownerLinked"], true);
        assert_eq!(created["record"]["owner"]["workosUserId"], subject);
        assert_eq!(created["record"]["owner"]["existingAccountConfirmed"], true);
        assert_eq!(
            created["request"]["owner_chat_account_id"],
            test_chat_account_id(subject)
        );
        assert!(
            backend.created().is_empty(),
            "a linked owner keeps their WorkOS user"
        );
        let (_, me) = send_json(&app, "GET", "/api/core/v1/me", &session(subject), None).await;
        assert_eq!(
            project_ids(&me),
            vec![created["project"]["id"].as_str().unwrap().to_string()]
        );
    })
    .await;
}

#[tokio::test]
async fn an_unreachable_hosted_device_creates_no_agent() {
    with_isolated_postgres(|db| async move {
        let (app, backend) = operator_app(&db).await;
        backend
            .hosted_unavailable
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let (status, refused) = send_json(
            &app,
            "POST",
            PATH,
            &admin(),
            Some(create_body("op-offline@example.test", "offline")),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{refused}");
        assert!(db.all("projects").await.is_empty());
        assert!(db.all("operator_agent_creations").await.is_empty());
        // The WorkOS user it made is the person's own sign-in record; the
        // retry reuses it rather than creating another.
        assert_eq!(backend.created().len(), 1);
        backend
            .hosted_unavailable
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let (status, created) = send_json(
            &app,
            "POST",
            PATH,
            &admin(),
            Some(create_body("op-offline@example.test", "offline")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        assert_eq!(backend.created().len(), 1);
        assert_eq!(
            created["record"]["owner"]["workosUserId"],
            backend.created()[0]
        );
    })
    .await;
}
