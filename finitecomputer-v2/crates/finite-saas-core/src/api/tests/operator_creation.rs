//! Operator creation of an Agent for an account named by email, through the
//! admin route, the Runner lease path, and the owner's later WorkOS sign-in.

use super::*;

const PATH: &str = "/api/core/v1/admin/operator-agent-creations";
const ADMIN_SUBJECT: &str = "workos_operator_admin";
const ADMIN_EMAIL: &str = "admin@finite.vip";

fn admin() -> Vec<(String, String)> {
    vec![(
        "authorization".to_string(),
        format!(
            "Bearer {}",
            access_token_with_subject(ADMIN_SUBJECT, ADMIN_EMAIL, true, Some(OPERATOR_ORG_ID))
        ),
    )]
}

fn service() -> Vec<(String, String)> {
    vec![("authorization".to_string(), format!("Bearer {TOKEN}"))]
}

pub(super) fn create_body(owner_email: &str, key: &str) -> serde_json::Value {
    serde_json::json!({
        "ownerEmail": owner_email,
        "displayName": "Frankie Bot",
        "reason": "box1 migration: Frankie's bot moves to a finite-mono Agent",
        "idempotencyKey": key,
    })
}

/// Link the admin's own account the way the dashboard does on first load.
async fn link_admin(app: &Router) {
    let (status, _) = send_json(app, "GET", "/api/core/v1/me", &admin(), None).await;
    assert_eq!(status, StatusCode::OK);
}

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

fn project_ids(me: &serde_json::Value) -> Vec<String> {
    me["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|project| project["project"]["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn operator_creates_for_pending_account_and_the_owner_claims_it_at_sign_in() {
    with_isolated_postgres(|db| async move {
        let app = admin_router(db.store.clone());
        link_admin(&app).await;
        let pending_id = seed_pending_user(&db, "frankie@finite.vip").await;

        let (status, created) = send_json(
            &app,
            "POST",
            PATH,
            &admin(),
            Some(create_body(" Frankie@Finite.VIP ", "box1-frankie-1")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        assert_eq!(created["reused"], false);
        assert_eq!(created["ownerLinked"], false);
        let record = &created["record"];
        assert_eq!(record["ownerEmail"], "frankie@finite.vip");
        assert_eq!(record["ownerUserId"], pending_id.as_str());
        assert_eq!(record["ownerAccountCreated"], false);
        assert_eq!(record["customerOrgCreated"], true);
        assert_eq!(record["admission"], "operator_sponsored");
        assert_eq!(record["surface"], "admin_api");
        assert_eq!(record["admin"]["email"], ADMIN_EMAIL);
        assert_eq!(record["admin"]["workosUserId"], ADMIN_SUBJECT);
        assert_eq!(record["placement"]["runnerClass"], "kata");
        assert_eq!(
            record["placement"]["runtimeResourceClass"],
            "vcpu4_memory8_gib"
        );
        let project_id = created["project"]["id"].as_str().unwrap().to_string();
        let request_id = created["request"]["id"].as_str().unwrap().to_string();
        assert_eq!(created["project"]["owner_user_id"], pending_id.as_str());
        assert_eq!(created["request"]["status"], "requested");
        assert_eq!(
            created["request"]["idempotency_key"],
            "operator:box1-frankie-1"
        );
        assert_eq!(
            created["request"]["requested_launch_code"],
            serde_json::Value::Null
        );
        // The new org is sponsored: no Stripe, no trial clock.
        let org = db
            .row("customer_orgs", record["customerOrgId"].as_str().unwrap())
            .await
            .unwrap();
        assert_eq!(org["billing_class"], "sponsored");
        assert_eq!(org["owner_user_id"], pending_id.as_str());

        // The durable audit record reads back for operators.
        let (status, listed) = send_json(&app, "GET", PATH, &admin(), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert_eq!(listed[0]["projectId"], project_id.as_str());
        assert_eq!(listed[0]["agentCreationRequestId"], request_id.as_str());
        assert_eq!(listed[0]["reason"], create_body("", "")["reason"]);
        assert!(listed[0]["createdAt"].as_str().is_some());

        // The normal Runner path leases and launches it.
        lease_and_complete(&app, &request_id).await;

        // An unverified WorkOS user with the same email cannot claim it.
        let unverified = identity_headers("frankie@finite.vip", "false");
        let (status, _) = send_json(&app, "GET", "/api/core/v1/me", &unverified, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let pending = db.row("users", &pending_id).await.unwrap();
        assert_eq!(pending["link_status"], "pending");

        // A different account sees nothing and cannot control it.
        let stranger = identity_headers("mallory@example.com", "true");
        let (status, me) = send_json(&app, "GET", "/api/core/v1/me", &stranger, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(project_ids(&me).is_empty());
        let (status, _) = send_json(
            &app,
            "POST",
            &format!("/api/core/v1/me/projects/{project_id}/runtime/restart"),
            &stranger,
            Some(serde_json::json!({})),
        )
        .await;
        assert!(status.is_client_error(), "{status}");

        // The person signs in with that email and the Agent is already theirs.
        let owner = identity_headers("frankie@finite.vip", "true");
        let (status, me) = send_json(&app, "GET", "/api/core/v1/me", &owner, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(project_ids(&me), vec![project_id.clone()]);
        let linked = db.row("users", &pending_id).await.unwrap();
        assert_eq!(linked["link_status"], "linked");
        let (status, billing) =
            send_json(&app, "GET", "/api/core/v1/me/billing", &owner, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(billing["requires_billing"], false);
        assert_eq!(billing["customer_org"]["billing_class"], "sponsored");
        // The stranger still sees nothing.
        let (_, me) = send_json(&app, "GET", "/api/core/v1/me", &stranger, None).await;
        assert!(project_ids(&me).is_empty());
    })
    .await;
}

#[tokio::test]
async fn operator_creates_the_pending_account_for_an_unknown_email() {
    with_isolated_postgres(|db| async move {
        let app = admin_router(db.store.clone());
        link_admin(&app).await;
        let mut body = create_body("new-person@example.com", "box1-new-1");
        body["ownerChatAccountId"] = serde_json::json!("ab".repeat(32));
        let (status, created) = send_json(&app, "POST", PATH, &admin(), Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{created}");
        assert_eq!(created["record"]["ownerAccountCreated"], true);
        let owner_id = created["record"]["ownerUserId"].as_str().unwrap();
        let owner = db.row("users", owner_id).await.unwrap();
        assert_eq!(owner["normalized_email"], "new-person@example.com");
        assert_eq!(owner["link_status"], "pending");
        assert_eq!(owner["workos_user_id"], serde_json::Value::Null);
        let org_id = created["record"]["customerOrgId"].as_str().unwrap();
        let entitlement = db
            .query_json(
                "SELECT to_jsonb(e) FROM agent_creation_entitlements e WHERE customer_org_id = $1",
                &[&org_id],
            )
            .await;
        assert_eq!(entitlement[0]["allowed_new_agent_runtimes"], 1);

        // A known owner chat id pins chat admission at lease, as self-serve does.
        let request_id = created["request"]["id"].as_str().unwrap().to_string();
        let lease = lease_and_complete(&app, &request_id).await;
        assert!(lease.to_string().contains(&"ab".repeat(32)));

        let owner_headers = identity_headers("new-person@example.com", "true");
        let (_, me) = send_json(&app, "GET", "/api/core/v1/me", &owner_headers, None).await;
        assert_eq!(
            project_ids(&me),
            vec![created["project"]["id"].as_str().unwrap().to_string()]
        );
    })
    .await;
}

#[tokio::test]
async fn operator_retry_with_the_same_key_never_creates_a_second_agent() {
    with_isolated_postgres(|db| async move {
        let app = admin_router(db.store.clone());
        link_admin(&app).await;
        let body = create_body("retry@example.com", "box1-retry-1");
        let (status, first) = send_json(&app, "POST", PATH, &admin(), Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        let (status, second) = send_json(&app, "POST", PATH, &admin(), Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{second}");
        assert_eq!(second["reused"], true);
        assert_eq!(second["record"], first["record"]);
        assert_eq!(second["request"]["id"], first["request"]["id"]);
        assert_eq!(second["project"]["id"], first["project"]["id"]);
        assert_eq!(db.all("projects").await.len(), 1);
        assert_eq!(db.all("agent_creation_requests").await.len(), 1);
        assert_eq!(db.all("operator_agent_creations").await.len(), 1);
        let entitlements = db.all("agent_creation_entitlements").await;
        assert_eq!(entitlements[0]["allowed_new_agent_runtimes"], 1);

        // The same key with any different input fails closed.
        for (field, value) in [
            ("displayName", serde_json::json!("Other Bot")),
            ("ownerEmail", serde_json::json!("someone-else@example.com")),
            ("reason", serde_json::json!("a different reason")),
            ("hostingTier", serde_json::json!("confidential")),
        ] {
            let mut changed = body.clone();
            changed[field] = value;
            let (status, refused) = send_json(&app, "POST", PATH, &admin(), Some(changed)).await;
            assert_eq!(status, StatusCode::CONFLICT, "{field}: {refused}");
        }
        assert_eq!(db.all("projects").await.len(), 1);
        assert_eq!(db.all("users").await.len(), 2);

        // A new key is a deliberate second Agent for the same person.
        let (status, other) = send_json(
            &app,
            "POST",
            PATH,
            &admin(),
            Some(create_body("retry@example.com", "box1-retry-2")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{other}");
        assert_eq!(
            other["record"]["ownerUserId"],
            first["record"]["ownerUserId"]
        );
        assert_ne!(other["project"]["id"], first["project"]["id"]);
        let entitlements = db.all("agent_creation_entitlements").await;
        assert_eq!(entitlements[0]["allowed_new_agent_runtimes"], 2);
    })
    .await;
}

#[tokio::test]
async fn operator_creation_is_admin_only() {
    with_isolated_postgres(|db| async move {
        let app = admin_router(db.store.clone());
        let body = create_body("victim@example.com", "not-admin-1");
        // A linked, verified account outside the operator org is refused
        // at the route, before Core looks at who it is.
        let member = identity_headers("member@example.com", "true");
        let (status, _) = send_json(&app, "GET", "/api/core/v1/me", &member, None).await;
        assert_eq!(status, StatusCode::OK);
        for headers in [member.clone(), Vec::new()] {
            let (status, _) = send_json(&app, "POST", PATH, &headers, Some(body.clone())).await;
            assert!(
                matches!(status, StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED),
                "{status}"
            );
            let (status, _) = send_json(&app, "GET", PATH, &headers, None).await;
            assert!(
                matches!(status, StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED),
                "{status}"
            );
        }
        // An operator-org session whose account Core has never linked is
        // refused too: the audit names one existing admin account.
        let (status, refused) = send_json(&app, "POST", PATH, &admin(), Some(body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        assert!(db.all("projects").await.is_empty());
        assert!(db.all("operator_agent_creations").await.is_empty());
        let users = db.all("users").await;
        assert!(
            users
                .iter()
                .all(|user| user["normalized_email"] != "victim@example.com")
        );
    })
    .await;
}

#[tokio::test]
async fn operator_creation_validates_input_and_target_host() {
    with_isolated_postgres(|db| async move {
        let app = admin_router(db.store.clone());
        link_admin(&app).await;
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
        let mut no_reason = create_body("person@example.com", "no-reason");
        no_reason["reason"] = serde_json::json!("   ");
        let (status, _) = send_json(&app, "POST", PATH, &admin(), Some(no_reason)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let mut long_reason = create_body("person@example.com", "long-reason");
        long_reason["reason"] = serde_json::json!("x".repeat(2_001));
        let (status, _) = send_json(&app, "POST", PATH, &admin(), Some(long_reason)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let mut unknown_host = create_body("person@example.com", "unknown-host");
        unknown_host["targetSourceHostId"] = serde_json::json!("not-a-registered-host");
        let (status, _) = send_json(&app, "POST", PATH, &admin(), Some(unknown_host)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let mut unknown_field = create_body("person@example.com", "unknown-field");
        unknown_field["launchCode"] = serde_json::json!("ignored?");
        let (status, _) = send_json(&app, "POST", PATH, &admin(), Some(unknown_field)).await;
        assert!(status.is_client_error());
        assert!(db.all("projects").await.is_empty());
        assert_eq!(db.all("users").await.len(), 1, "only the admin exists");
    })
    .await;
}
