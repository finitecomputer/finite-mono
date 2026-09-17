use super::*;

#[tokio::test]
async fn hosted_hermes_location_requires_current_owner_not_admin() {
    with_isolated_postgres(|db| async move {
        let owner_subject = "hosted-hermes-owner";
        let owner_email = "hosted-hermes-owner@example.test";
        let launch_code = issue_test_launch_code(&db).await;
        let creation = db.request_agent_creation(RequestAgentCreationInput {
            verified_email: owner_email.into(),
            workos_user_id: owner_subject.into(),
            display_name: "Hermes location fixture".into(),
            launch_code,
            idempotency_key: "hosted-hermes-location".into(),
            now: None,
        }).await.unwrap();
        let lease = db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
            runner_id: "location-runner".into(),
            source_host_id: None,
            lease_token: "location-lease".into(),
            lease_seconds: Some(300),
            runner_capacity: None,
            now: None,
        }).await.unwrap().unwrap();
        let completed = db.complete_agent_creation_request(CompleteAgentCreationRequestInput {
            request_id: lease.request.id,
            runner_id: "location-runner".into(),
            lease_token: "location-lease".into(),
            source_host_id: "location-host".into(),
            source_machine_id: "location-machine".into(),
            runtime_artifact_id: lease.request.desired_runtime_artifact_id,
            state_schema_version: Some("state-v1".into()),
            provider_runtime_handle: None,
            contact_endpoint: Some("https://untrusted-contact.example.test/contact".into()),
            runtime_capabilities: None,
            display_name: None,
            hostname: Some("untrusted-hostname.example.test".into()),
            runtime_host: Some("https://untrusted-runtime-host.example.test".into()),
            runtime_status: Some(RuntimeSummaryStatus::Online),
            active_inference_profile: None,
            hermes_available: Some(true),
            published_app_urls: vec!["https://untrusted-published.example.test/".into()],
            agent_npub: None,
            now: None,
        }).await.unwrap();
        let runtime_id = completed.request.agent_runtime_id.unwrap();
        let app = router_with_hosted_hermes_origins(
            db.store.clone(), test_auth(), None,
            HostedHermesOrigins::from_json(r#"{"location-host":"https://agents.example.test/"}"#).unwrap(),
        );
        let uri = format!("/api/core/v1/me/runtimes/{runtime_id}/hosted-hermes-location");
        let headers = |subject: &str, email: &str, org: Option<&str>, verified| vec![(
            "authorization".into(),
            format!("Bearer {}", access_token_with_subject(subject, email, verified, org)),
        )];
        let owner = headers(owner_subject, owner_email, Some(OPERATOR_ORG_ID), true);
        let response = app.clone().oneshot(Request::builder()
            .uri(format!("{uri}?host=attacker.example&target=https://attacker.example/"))
            .header("authorization", &owner[0].1)
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body: Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(body, json!({
            "runtimeId": runtime_id,
            "baseUrl": format!("https://agents.example.test/runtimes/{runtime_id}/"),
            "availability": "unqualified",
        }));
        // Even online/chat-ready facts above do not qualify native access.
        for (auth, expected) in [
            (Vec::new(), StatusCode::UNAUTHORIZED),
            (vec![("authorization".into(), format!("Bearer {TOKEN}"))], StatusCode::UNAUTHORIZED),
            (headers(owner_subject, owner_email, None, true), StatusCode::OK),
            (headers(owner_subject, owner_email, Some("customer-org"), true), StatusCode::OK),
            (headers("other-admin", "other-admin@example.test", Some(OPERATOR_ORG_ID), true), StatusCode::NOT_FOUND),
        ] {
            let (status, _) = send_json(&app, "GET", &uri, &auth, None).await;
            assert_eq!(status, expected);
        }
        let unverified = headers("unverified-admin", "unverified@example.test", Some(OPERATOR_ORG_ID), false);
        assert_eq!(send_json(&app, "GET", &uri, &unverified, None).await.0, StatusCode::FORBIDDEN);
        let mut spoofed = owner.clone();
        spoofed.push((WORKOS_USER_ID_HEADER.into(), owner_subject.into()));
        assert_eq!(send_json(&app, "GET", &uri, &spoofed, None).await.0, StatusCode::UNAUTHORIZED);
        // Stable Core runtime IDs only; neither Project nor provider names resolve.
        for alias in [&creation.project.id, "location-machine", "missing-runtime"] {
            let alias_uri = format!("/api/core/v1/me/runtimes/{alias}/hosted-hermes-location");
            assert_eq!(send_json(&app, "GET", &alias_uri, &owner, None).await.0, StatusCode::NOT_FOUND);
        }
        let unconfigured = router(db.store.clone(), test_auth());
        let (status, body) = send_json(&unconfigured, "GET", &uri, &owner, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"runtimeId": runtime_id, "baseUrl": null, "availability": "not_configured"}));

        // Ownership is read from durable current state, not a cached identity
        // or an admin bypass. An ownership change immediately removes discovery.
        let successor = db.link_verified_user(LinkVerifiedUserInput {
            verified_email: "successor@example.test".into(),
            workos_user_id: "successor".into(),
            now: None,
        }).await.unwrap();
        db.query_json("UPDATE projects SET owner_user_id=$2 WHERE id=$1 RETURNING to_jsonb(id)", &[&creation.project.id, &successor.id]).await;
        assert_eq!(send_json(&app, "GET", &uri, &owner, None).await.0, StatusCode::NOT_FOUND);
        let successor_headers = headers("successor", "successor@example.test", Some(OPERATOR_ORG_ID), true);
        assert_eq!(send_json(&app, "GET", &uri, &successor_headers, None).await.0, StatusCode::OK);
        db.query_json("UPDATE project_runtime_links SET active=FALSE WHERE agent_runtime_id=$1 RETURNING to_jsonb(id)", &[&runtime_id]).await;
        assert_eq!(send_json(&app, "GET", &uri, &successor_headers, None).await.0, StatusCode::NOT_FOUND);
    }).await;
}
