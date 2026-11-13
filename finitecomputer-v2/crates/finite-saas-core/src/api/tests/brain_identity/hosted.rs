use super::*;
use crate::brain_identity::hosted::HOSTED_OBSERVATION_VERSION;
use crate::brain_identity::scoped::DESCRIPTIONS_VERSION_V3;

fn body() -> Value {
    serde_json::json!({"version": HOSTED_OBSERVATION_VERSION, "operationId": "hosted_http_00000000",
        "publicKeyHex": key(81), "observedAt": crate::current_time_iso().unwrap()})
}

#[tokio::test]
async fn hosted_http_requires_both_credentials_and_accepts_no_browser_account_or_brain() {
    with_isolated_postgres(|db| async move {
        let app = identity_app(db.store.clone());
        let credential = (OBSERVATION_CREDENTIAL_HEADER, OBSERVATION_TOKEN.to_string());
        let account = ("authorization", bearer("hosted@example.test"));
        for (index, headers) in [
            vec![account.clone()],
            vec![credential.clone()],
            vec![
                (OBSERVATION_CREDENTIAL_HEADER, DESCRIPTION_TOKEN.into()),
                account.clone(),
            ],
            vec![
                credential.clone(),
                ("authorization", format!("Bearer {TOKEN}")),
            ],
            vec![
                credential.clone(),
                (
                    "authorization",
                    format!(
                        "Bearer {}",
                        access_token("hosted@example.test", false, None)
                    ),
                ),
            ],
            vec![
                credential.clone(),
                account.clone(),
                (WORKOS_EMAIL_HEADER, "spoof@example.test".into()),
            ],
        ]
        .into_iter()
        .enumerate()
        {
            let (status, _) = post(
                &app,
                HOSTED_OBSERVATION_PATH,
                &headers,
                serde_json::to_vec(&body()).unwrap(),
            )
            .await;
            assert_eq!(
                status,
                if index == 4 {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::UNAUTHORIZED
                }
            );
        }
        let headers = [credential, account];
        for (field, value) in [
            ("version", serde_json::json!("old")),
            ("publicKeyHex", serde_json::json!("bad")),
            ("observedAt", serde_json::json!("yesterday")),
            ("operationId", serde_json::json!("short")),
            ("email", serde_json::json!("spoof@example.test")),
            ("brainId", serde_json::json!("invented")),
        ] {
            let mut request = body();
            request[field] = value;
            let (status, _) = post(
                &app,
                HOSTED_OBSERVATION_PATH,
                &headers,
                serde_json::to_vec(&request).unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{field}");
        }
        let (status, _) = post(
            &app,
            HOSTED_OBSERVATION_PATH,
            &headers,
            serde_json::to_vec(&body()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(db.all("users").await.is_empty());
        assert!(db.all("account_brain_principals").await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn hosted_http_registration_preserves_v1_and_v3_participation_gates() {
    with_isolated_postgres(|db| async move {
        let token = bearer("hosted@example.test");
        let me = router(db.store.clone(), scoped_test_auth())
            .oneshot(
                Request::builder()
                    .uri("/api/core/v1/me")
                    .header("authorization", &token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(me.status(), StatusCode::OK);
        let app = identity_app(db.store.clone());
        let headers = [
            (OBSERVATION_CREDENTIAL_HEADER, OBSERVATION_TOKEN.to_string()),
            ("authorization", token),
        ];
        let request = serde_json::to_vec(&body()).unwrap();
        let (status, first) = post(&app, HOSTED_OBSERVATION_PATH, &headers, request.clone()).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["version"], HOSTED_OBSERVATION_VERSION);
        assert_eq!(first["outcome"], "recorded");
        assert_eq!(
            post(&app, HOSTED_OBSERVATION_PATH, &headers, request)
                .await
                .1,
            first
        );
        let reader = [(DESCRIPTION_CREDENTIAL_HEADER, DESCRIPTION_TOKEN.to_string())];
        for (version, participating, expected) in [
            (DESCRIPTIONS_VERSION, false, "notShared"),
            (DESCRIPTIONS_VERSION_V2, true, "resolved"),
            (DESCRIPTIONS_VERSION_V3, false, "notShared"),
            (DESCRIPTIONS_VERSION_V3, true, "resolved"),
        ] {
            let mut request = description_body(&[key(81)]);
            request["version"] = serde_json::json!(version);
            if version == DESCRIPTIONS_VERSION_V3 {
                request["participatingPublicKeysHex"] = if participating {
                    serde_json::json!([key(81)])
                } else {
                    serde_json::json!([])
                };
            }
            let (status, response) = post(
                &app,
                DESCRIPTIONS_PATH,
                &reader,
                serde_json::to_vec(&request).unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{response}");
            assert_eq!(response["results"][0]["state"], expected, "{response}");
            if expected == "resolved" {
                assert_eq!(
                    response["results"][0]["accountEmail"],
                    "hosted@example.test"
                );
            } else {
                assert!(response["results"][0].get("accountEmail").is_none());
            }
        }
        assert!(db.all("account_brain_sharing_scopes").await.is_empty());
    })
    .await;
}
