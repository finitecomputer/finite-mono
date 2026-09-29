use super::*;

#[tokio::test]
async fn trial_codes_require_operator_and_reservations_require_service() {
    with_isolated_postgres(|db| async move {
        let app = admin_router(db.store.clone());
        let member = identity_headers("member@example.com", "true");
        let service = [("authorization".to_string(), format!("Bearer {TOKEN}"))];
        for headers in [&member[..], &service[..]] {
            let (status, _) = send_json(&app, "POST", "/api/core/v1/admin/trial-campaigns", headers,
                Some(serde_json::json!({"name":"Event", "seatLimit":10}))).await;
            assert!(status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN);
        }
        let operator = operator_identity_headers("operator@finite.vip");
        let response = app.clone().oneshot(Request::builder()
            .method("POST").uri("/api/core/v1/admin/trial-campaigns")
            .header("authorization", &operator[0].1).header("content-type", "application/json")
            .body(Body::from(r#"{"name":"Event", "seatLimit":10}"#)).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store, private");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let issued: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let code = issued["code"].as_str().unwrap();
        let (status, listed) = send_json(&app, "GET", "/api/core/v1/admin/trial-campaigns", &operator, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed[0]["trialDays"], 7);
        assert!(!listed.to_string().contains(code));

        for headers in [&member[..], &operator[..]] {
            let (status, _) = send_json(&app, "POST", "/api/core/v1/billing/trial-reservation", headers,
                Some(serde_json::json!({"workosUserId":"user", "reservation": {
                    "code":code, "customerOrgId":"org", "stripeCustomerId":"cus_fake",
                    "stripeSessionId":"cs_fake", "attemptId":"fake", "checkoutExpiresAt":9999999999i64, "trialDays":7
                }}))).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            let (status, _) = send_json(&app, "POST", "/api/core/v1/billing/trial-expired", headers,
                Some(serde_json::json!({"stripeSessionId":"cs_fake", "stripeCustomerId":"cus_fake"}))).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, _) = send_json(&app, "POST", "/api/core/v1/billing/trial-expired", &service,
            Some(serde_json::json!({"stripeSessionId":"cs_fake", "stripeCustomerId":"cus_fake"}))).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }).await;
}
