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
        assert_eq!(listed[0]["code"], code);
        assert_eq!(listed[0]["codeRevision"], 0);

        let code_path = format!("/api/core/v1/admin/trial-campaigns/{}/code", issued["id"].as_str().unwrap());
        for headers in [&member[..], &service[..], &[]] {
            let (status, _) = send_json(&app, "POST", &code_path, headers,
                Some(serde_json::json!({"code":"WORKSHOP2026","expectedCodeRevision":0}))).await;
            assert!(status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN);
        }
        let (status, _) = send_json(&app, "POST", &code_path, &operator,
            Some(serde_json::json!({"code":"WORKSHOP2026","expectedCodeRevision":0}))).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let response = app.clone().oneshot(Request::builder().uri("/api/core/v1/admin/trial-campaigns")
            .header("authorization", &operator[0].1).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.headers()["cache-control"], "no-store, private");
        let (status, offer) = send_json(&app, "POST", "/api/core/v1/me/billing/trial-offer", &operator,
            Some(serde_json::json!({"code":"workshop-2026"}))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(offer.get("code").is_none());
        assert!(offer.get("codeRevision").is_none());

        let capacity_path = format!("/api/core/v1/admin/trial-campaigns/{}/capacity", issued["id"].as_str().unwrap());
        for headers in [&member[..], &service[..], &[]] {
            let (status, _) = send_json(&app, "GET", "/api/core/v1/admin/trial-campaigns", headers, None).await;
            assert!(status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN);
            let (status, _) = send_json(&app, "POST", &capacity_path, headers,
                Some(serde_json::json!({"seatLimit":15,"expectedSeatLimit":10}))).await;
            assert!(status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN);
        }
        let (status, _) = send_json(&app, "POST", &capacity_path, &operator,
            Some(serde_json::json!({"seatLimit":15,"expectedSeatLimit":10}))).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, listed) = send_json(&app, "GET", "/api/core/v1/admin/trial-campaigns", &operator, None).await;
        assert_eq!(listed[0]["seatLimit"], 15);
        assert_eq!(listed[0]["seatsRemaining"], 15);
        let (status, _) = send_json(&app, "POST", &capacity_path, &operator,
            Some(serde_json::json!({"seatLimit":20,"expectedSeatLimit":10}))).await;
        assert!(status.is_client_error());

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
