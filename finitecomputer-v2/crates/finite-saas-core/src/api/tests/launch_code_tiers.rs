use super::*;

#[tokio::test]
async fn core_api_launch_code_selects_issued_tier_and_retries_without_redeeming_twice() {
    for tier in [
        crate::HostingTier::Standard,
        crate::HostingTier::Confidential,
    ] {
        with_isolated_postgres(|db| async move {
            let issued = db
                .store
                .issue_launch_code_batch(crate::launch_codes::IssueLaunchCodeBatchInput {
                    name: "Organization code test".into(),
                    code_count: 1,
                    expires_in_hours: Some(1),
                    hosting_tier: Some(tier),
                    created_by_workos_user_id: "test_admin".into(),
                    now: None,
                })
                .await
                .unwrap();
            let app = router(db.store.clone(), test_auth());
            let submit = |code: String, requested: Option<crate::HostingTier>| {
                let app = app.clone();
                async move {
                    app.oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/api/core/v1/me/agent-creation-requests")
                            .header(
                                "authorization",
                                format!(
                                    "Bearer {}",
                                    access_token_with_subject(
                                        "user_code_tier",
                                        "code-tier@example.test",
                                        true,
                                        None
                                    )
                                ),
                            )
                            .header("content-type", "application/json")
                            .body(Body::from(
                                serde_json::json!({
                                    "displayName": "Organization Agent",
                                    "launchCode": code,
                                    "idempotencyKey": "organization-code-submit",
                                    "hostingTier": requested,
                                    "ownerChatAccountId": "a".repeat(64),
                                })
                                .to_string(),
                            ))
                            .unwrap(),
                    )
                    .await
                    .unwrap()
                }
            };
            assert_eq!(
                submit("invalid-code".into(), None).await.status(),
                StatusCode::BAD_REQUEST
            );
            let wrong_tier = if tier == crate::HostingTier::Standard {
                crate::HostingTier::Confidential
            } else {
                crate::HostingTier::Standard
            };
            assert!(
                !submit(issued.codes[0].code.clone(), Some(wrong_tier))
                    .await
                    .status()
                    .is_success()
            );
            assert!(db.all("agent_creation_requests").await.is_empty());
            let mut identity = None;
            for retry in [false, true] {
                let response = submit(issued.codes[0].code.clone(), None).await;
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let result: RequestAgentCreationResult = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(result.project.hosting_tier, Some(tier));
                assert_eq!(result.request.hosting_tier, Some(tier));
                assert_eq!(result.request.owner_chat_account_id, Some("a".repeat(64)));
                assert_eq!(result.reused, retry);
                if let Some(id) = identity.as_ref() {
                    assert_eq!(&result.request.id, id);
                }
                identity = Some(result.request.id);
            }
            assert_eq!(db.all("agent_creation_requests").await.len(), 1);
        })
        .await;
    }
}
