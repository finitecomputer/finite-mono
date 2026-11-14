//! The owner's sign-in proof for operator-created Agents, through Core's
//! WorkOS client seam: the operator path creates the person's WorkOS user
//! and reads its hosted-chat account; the Agent launches admitting only
//! that account; the person's later sign-in with that email is the same
//! WorkOS user, so Core links them and the chat identity matches.

use super::operator_creation::*;
use super::*;
use crate::auth::test_support::{
    registry_put_user, registry_users_by_email, registry_verify_email,
};
use crate::owner_identity::test_support::test_chat_account_id;

async fn seed_pending_user(db: &crate::test_support::TestDb, email: &str) -> String {
    db.query_json(
        "INSERT INTO users (id, normalized_email, link_status, created_at, updated_at)
         VALUES ('user_pending_seed', $1, 'pending', NOW(), NOW())
         RETURNING to_jsonb(id)",
        &[&email],
    )
    .await[0]
        .as_str()
        .unwrap()
        .to_string()
}

async fn lease_and_complete(app: &Router, request_id: &str) -> serde_json::Value {
    let (status, lease) = send_json(
        app,
        "POST",
        "/api/core/v1/agent-creation-requests/lease",
        &service(),
        Some(serde_json::json!({
            "runnerId": "runner-oslo-1",
            "leaseToken": "lease-token-operator",
            "leaseSeconds": 300,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{lease}");
    assert_eq!(lease["request"]["id"], request_id);
    // The Runner's Finite Private key step works for a never-signed-in owner.
    let (status, key) = send_json(
        app,
        "POST",
        &format!("/api/core/v1/agent-creation-requests/{request_id}/finite-private-key"),
        &service(),
        Some(serde_json::json!({
            "runnerId": "runner-oslo-1",
            "leaseToken": "lease-token-operator",
            "sourceHostId": "oslo-host-1",
            "sourceMachineId": "oslo-agent-operator",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{key}");
    assert_eq!(key["grant"]["user_id"], lease["request"]["owner_user_id"]);
    let (status, completed) = send_json(
        app,
        "POST",
        &format!("/api/core/v1/agent-creation-requests/{request_id}/complete"),
        &service(),
        Some(serde_json::json!({
            "runnerId": "runner-oslo-1",
            "leaseToken": "lease-token-operator",
            "sourceHostId": "oslo-host-1",
            "sourceMachineId": "oslo-agent-operator",
            "runtimeArtifactId": "artifact-postgres-fixture",
            "runtimeHost": "oslo-host-1",
            "runtimeStatus": "online",
            "activeInferenceProfile": "finite-private",
            "hermesAvailable": true,
            "publishedAppUrls": [],
            "runtimeCapabilities": runtime_capabilities_json(true),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["request"]["status"], "running");
    lease
}

/// The exact sender set the Agent's chat sidecar is born with.
fn owner_npubs(lease: &serde_json::Value) -> &serde_json::Value {
    &lease["request"]["runtime_spec"]["spec"]["environment"]["FINITECHAT_OWNER_NPUBS"]
}

#[tokio::test]
async fn operator_creates_for_pending_account_and_the_owner_claims_it_at_sign_in() {
    with_isolated_postgres(|db| async move {
        let (app, backend) = operator_app(&db).await;
        let email = "op-frankie@finite.example";
        let pending_id = seed_pending_user(&db, email).await;

        let (status, created) = send_json(
            &app,
            "POST",
            PATH,
            &admin(),
            Some(create_body(" OP-Frankie@Finite.Example ", "box1-frankie-1")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        assert_eq!(created["reused"], false);
        assert_eq!(created["ownerLinked"], false);
        let record = &created["record"];
        assert_eq!(record["owner"]["email"], email);
        assert_eq!(record["owner"]["userId"], pending_id.as_str());
        assert_eq!(record["owner"]["accountCreated"], false);
        assert_eq!(record["owner"]["customerOrgCreated"], true);
        assert_eq!(record["admission"], "operator_sponsored");
        assert_eq!(record["actor"]["surface"], "admin_api");
        assert_eq!(record["actor"]["cliOsUser"], serde_json::Value::Null);
        assert_eq!(record["actor"]["admin"]["email"], "admin@finite.vip");
        assert_eq!(record["placement"]["runnerClass"], "kata");

        // The operator path created exactly one WorkOS user for the email,
        // unverified, and read its hosted-chat account.
        let created_users = backend.created();
        assert_eq!(created_users.len(), 1);
        let workos_user = created_users[0].clone();
        let registered = registry_users_by_email(email);
        assert_eq!(registered.len(), 1);
        assert!(!registered[0].email_verified, "no mailbox access was used");
        let chat = test_chat_account_id(&workos_user);
        assert_eq!(record["owner"]["workosUserId"], workos_user.as_str());
        assert_eq!(record["owner"]["chatAccountId"], chat.as_str());
        assert_eq!(created["request"]["owner_chat_account_id"], chat.as_str());
        assert_eq!(
            created["request"]["idempotency_key"],
            "operator:box1-frankie-1"
        );
        assert_eq!(
            created["request"]["requested_launch_code"],
            serde_json::Value::Null
        );
        let project_id = created["project"]["id"].as_str().unwrap().to_string();
        let request_id = created["request"]["id"].as_str().unwrap().to_string();
        let org = db
            .row(
                "customer_orgs",
                record["owner"]["customerOrgId"].as_str().unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(org["billing_class"], "sponsored");

        // The normal Runner path leases and launches it, locked to the owner.
        let lease = lease_and_complete(&app, &request_id).await;
        assert_eq!(owner_npubs(&lease), chat.as_str());

        // Before the person proves the inbox, a session for that WorkOS user
        // cannot claim anything and the account stays pending.
        let (status, _) =
            send_json(&app, "GET", "/api/core/v1/me", &session(&workos_user), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            db.row("users", &pending_id).await.unwrap()["link_status"],
            "pending"
        );

        // A different account sees nothing and cannot control it.
        let stranger = identity_headers("mallory@example.com", "true");
        let (status, me) = send_json(&app, "GET", "/api/core/v1/me", &stranger, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(project_ids(&me).is_empty());
        let restart = format!("/api/core/v1/me/projects/{project_id}/runtime/restart");
        let (status, refused) = send_json(
            &app,
            "POST",
            &restart,
            &stranger,
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{refused}");

        // The person signs in with that email. WorkOS keeps one user per
        // email and attaches the new credential to it once the inbox is
        // verified: the same user id. Core links the pending account.
        registry_verify_email(&workos_user);
        let owner = session(&workos_user);
        let (status, me) = send_json(&app, "GET", "/api/core/v1/me", &owner, None).await;
        assert_eq!(status, StatusCode::OK, "{me}");
        assert_eq!(project_ids(&me), vec![project_id.clone()]);
        let linked = db.row("users", &pending_id).await.unwrap();
        assert_eq!(linked["link_status"], "linked");
        assert_eq!(linked["workos_user_id"], workos_user.as_str());
        // The hosted chat account the dashboard reads for this user is the
        // one the Agent admits.
        assert_eq!(
            test_chat_account_id(linked["workos_user_id"].as_str().unwrap()),
            chat
        );
        // The owner can control it; the same call the stranger could not.
        let (status, restarted) =
            send_json(&app, "POST", &restart, &owner, Some(serde_json::json!({}))).await;
        assert_eq!(status, StatusCode::OK, "{restarted}");
        let (status, billing) =
            send_json(&app, "GET", "/api/core/v1/me/billing", &owner, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(billing["requires_billing"], false);
        assert_eq!(billing["customer_org"]["billing_class"], "sponsored");

        // Operators read the record back and see the sign-in landed.
        let (status, listed) = send_json(&app, "GET", PATH, &admin(), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert_eq!(listed[0]["record"]["projectId"], project_id.as_str());
        assert_eq!(listed[0]["ownerLinked"], true);
        assert_eq!(listed[0]["signedInAsProvisionedUser"], true);
        let (_, me) = send_json(&app, "GET", "/api/core/v1/me", &stranger, None).await;
        assert!(project_ids(&me).is_empty());
    })
    .await;
}

#[tokio::test]
async fn operator_creates_the_pending_account_and_sign_in_user_for_an_unknown_email() {
    with_isolated_postgres(|db| async move {
        let (app, backend) = operator_app(&db).await;
        let email = "op-new-person@example.test";
        let (status, created) = send_json(
            &app,
            "POST",
            PATH,
            &admin(),
            Some(create_body(email, "box1-new-1")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        assert_eq!(created["record"]["owner"]["accountCreated"], true);
        let owner_id = created["record"]["owner"]["userId"]
            .as_str()
            .unwrap()
            .to_string();
        let owner = db.row("users", &owner_id).await.unwrap();
        assert_eq!(owner["link_status"], "pending");
        assert_eq!(owner["workos_user_id"], serde_json::Value::Null);

        // An existing WorkOS user for the email is reused, never duplicated.
        let reuse_email = "op-already-at-workos@example.test";
        registry_put_user("workos_op_existing", reuse_email, false);
        let (status, reused) = send_json(
            &app,
            "POST",
            PATH,
            &admin(),
            Some(create_body(reuse_email, "box1-new-2")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{reused}");
        assert_eq!(
            reused["record"]["owner"]["workosUserId"],
            "workos_op_existing"
        );
        assert_eq!(
            backend.created().len(),
            1,
            "only the unknown email got a new user"
        );

        let workos_user = backend.created()[0].clone();
        registry_verify_email(&workos_user);
        let (_, me) = send_json(&app, "GET", "/api/core/v1/me", &session(&workos_user), None).await;
        assert_eq!(
            project_ids(&me),
            vec![created["project"]["id"].as_str().unwrap().to_string()]
        );
    })
    .await;
}

/// WorkOS never makes a second user for one email, but if the person ever
/// signed in as another WorkOS user, the Agent would still admit only the
/// provisioned chat account: closed, never open. Operators see it.
#[tokio::test]
async fn a_sign_in_as_another_workos_user_never_opens_the_agent() {
    with_isolated_postgres(|db| async move {
        let (app, backend) = operator_app(&db).await;
        let email = "op-elsewhere@example.test";
        let (status, created) = send_json(
            &app,
            "POST",
            PATH,
            &admin(),
            Some(create_body(email, "elsewhere")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let provisioned = backend.created()[0].clone();
        registry_put_user("workos_op_other_user", email, true);
        let (status, _) = send_json(
            &app,
            "GET",
            "/api/core/v1/me",
            &session("workos_op_other_user"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let request = db
            .row(
                "agent_creation_requests",
                created["request"]["id"].as_str().unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            request["owner_chat_account_id"],
            test_chat_account_id(&provisioned)
        );
        let (_, listed) = send_json(&app, "GET", PATH, &admin(), None).await;
        assert_eq!(listed[0]["ownerLinked"], true);
        assert_eq!(listed[0]["signedInAsProvisionedUser"], false);
        assert_eq!(
            listed[0]["ownerCurrentWorkosUserId"],
            "workos_op_other_user"
        );
    })
    .await;
}
