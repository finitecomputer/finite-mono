use super::*;
use finite_nostr::{HttpAuthValidation, NostrPublicKey, validate_http_auth_event};
use serde_json::json;

const AUTHORIZATION_PATH: &str = "/v1/chat/direct-authorization";
const AGENT_ID: &str = "agent_project_8d91";

async fn authorization_response(
    device: axum::Router,
    token: Option<&str>,
    user: Option<&str>,
    body: impl Into<Body>,
) -> axum::response::Response {
    let mut request = Request::post(AUTHORIZATION_PATH).header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(user) = user {
        request = request.header(WORKOS_USER_HEADER, user);
    }
    device
        .oneshot(request.body(body.into()).unwrap())
        .await
        .unwrap()
}

async fn authorize(device: axum::Router, user: &str, request: Value) -> axum::response::Response {
    authorization_response(device, Some(TOKEN), Some(user), request.to_string()).await
}

async fn response_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

fn get_request(path: &str) -> Value {
    json!({"version": 1, "agent_id": AGENT_ID, "method": "GET", "path": path, "body_sha256": null})
}

/// Signs, checks the response envelope, and returns the verified event.
async fn signed_event(device: axum::Router, user: &str, request: Value, account_id: &str) -> Event {
    let response = authorize(device, user, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = response_body(response).await;
    assert_eq!(body["account_id"], account_id);
    let mut fields = body["event"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    fields.sort();
    assert_eq!(
        fields,
        [
            "content",
            "created_at",
            "id",
            "kind",
            "pubkey",
            "sig",
            "tags"
        ]
    );
    let event: Event = serde_json::from_value(body["event"].clone()).unwrap();
    verify_event_integrity(&event).unwrap();
    assert_eq!(event.pubkey.to_hex(), account_id);
    assert_eq!(event.kind.as_u16(), 27_235);
    assert_eq!(event.content, "");
    assert!(event.created_at.as_secs().abs_diff(test_now_unix_seconds()) <= 5);
    event
}

#[tokio::test]
async fn direct_authorization_signs_bridge_requests_with_the_existing_chat_account_key() {
    let root = TempDir::new().unwrap();
    let device = test_app(&root);
    let state = state_for(device.clone(), "user_paul").await;
    let account_id = state["identity"]["account_id"].as_str().unwrap().to_owned();
    let signer = NostrPublicKey::from_hex(&account_id).unwrap();

    let path = "/v1/app/state?room_id=room-1&limit=50";
    let event = signed_event(device.clone(), "user_paul", get_request(path), &account_id).await;
    let url = format!("finitechat-direct://{AGENT_ID}{path}");
    let value = serde_json::to_value(&event.tags).unwrap();
    let tags = value.as_array().unwrap();
    assert_eq!(
        tags[..2],
        json!([["u", url], ["method", "GET"]]).as_array().unwrap()[..]
    );
    assert_eq!(tags.len(), 3);
    assert_nonce_tag(&tags[2]);
    let repeat = signed_event(device.clone(), "user_paul", get_request(path), &account_id).await;
    assert_ne!(
        repeat.id, event.id,
        "identical requests sign distinct events"
    );
    // The shared NIP-98 verifier accepts the assertion exactly as signed.
    let now = test_now_unix_seconds();
    validate_http_auth_event(
        &event,
        &HttpAuthValidation::new("GET", url.clone(), now, 120).with_expected_signer(signer),
    )
    .unwrap();

    let body = br#"{"SendMessage":{"room_id":"room-1","text":"hello"}}"#;
    let body_sha256 = hex::encode(sha2::Sha256::digest(body));
    let post_path = "/v1/app/actions";
    let event = signed_event(
        device.clone(),
        "user_paul",
        json!({
            "version": 1,
            "agent_id": AGENT_ID,
            "method": "POST",
            "path": post_path,
            "body_sha256": body_sha256,
        }),
        &account_id,
    )
    .await;
    let post_url = format!("finitechat-direct://{AGENT_ID}{post_path}");
    let value = serde_json::to_value(&event.tags).unwrap();
    let tags = value.as_array().unwrap();
    assert_eq!(
        tags[..3],
        json!([
            ["u", post_url],
            ["method", "POST"],
            ["payload", body_sha256]
        ])
        .as_array()
        .unwrap()[..]
    );
    assert_nonce_tag(&tags[3]);
    validate_http_auth_event(
        &event,
        &HttpAuthValidation::new("POST", post_url, now, 120)
            .with_body(body.to_vec())
            .with_expected_signer(signer),
    )
    .unwrap();

    // Contract boundaries are inclusive: a 120-byte Agent id suffix and a
    // 2048-byte path both sign.
    let long_agent_id = format!("agent_{}", "A1_-".repeat(30));
    let long_path = format!("/v1/app/{}", "a".repeat(2048 - "/v1/app/".len()));
    let mut boundary = get_request(&long_path);
    boundary["agent_id"] = json!(long_agent_id);
    signed_event(device.clone(), "user_paul", boundary, &account_id).await;

    // A restarted Device signs with the same durable account key.
    signed_event(test_app(&root), "user_paul", get_request(path), &account_id).await;
}

#[tokio::test]
async fn direct_authorization_rejects_requests_outside_the_contract() {
    let root = TempDir::new().unwrap();
    let device = test_app(&root);
    state_for(device.clone(), "user_paul").await;
    let digest = "ab".repeat(32);
    let with = |field: &str, value: Value| {
        let mut request = get_request("/v1/app/state");
        request[field] = value;
        request
    };
    let post = |body_sha256: Value| {
        let mut request = with("method", json!("POST"));
        request["body_sha256"] = body_sha256;
        request
    };
    let mut missing_version = get_request("/v1/app/state");
    missing_version.as_object_mut().unwrap().remove("version");
    let mut missing_post_digest = post(Value::Null);
    missing_post_digest
        .as_object_mut()
        .unwrap()
        .remove("body_sha256");

    let cases: Vec<(&str, String)> = vec![
        ("malformed JSON", "not json".to_owned()),
        ("missing version", missing_version.to_string()),
        ("unsupported version", with("version", json!(2)).to_string()),
        ("unknown field", with("url", json!("https://x")).to_string()),
        (
            "agent id prefix",
            with("agent_id", json!("project_1")).to_string(),
        ),
        (
            "empty agent id",
            with("agent_id", json!("agent_")).to_string(),
        ),
        (
            "long agent id",
            with("agent_id", json!(format!("agent_{}", "a".repeat(121)))).to_string(),
        ),
        (
            "agent id dot",
            with("agent_id", json!("agent_a.b")).to_string(),
        ),
        (
            "agent id slash",
            with("agent_id", json!("agent_a/b")).to_string(),
        ),
        ("lowercase method", with("method", json!("get")).to_string()),
        ("PUT", with("method", json!("PUT")).to_string()),
        (
            "other prefix",
            with("path", json!("/v1/hermes/state")).to_string(),
        ),
        ("bare prefix", with("path", json!("/v1/app")).to_string()),
        (
            "relative path",
            with("path", json!("v1/app/state")).to_string(),
        ),
        (
            "fragment",
            with("path", json!("/v1/app/state#top")).to_string(),
        ),
        (
            "space",
            with("path", json!("/v1/app/state?q=a b")).to_string(),
        ),
        ("tab", with("path", json!("/v1/app/st\tate")).to_string()),
        (
            "newline",
            with("path", json!("/v1/app/state\n")).to_string(),
        ),
        (
            "delete",
            with("path", json!("/v1/app/state\u{7f}")).to_string(),
        ),
        (
            "non-ASCII",
            with("path", json!("/v1/app/\u{e9}")).to_string(),
        ),
        (
            "dot-dot",
            with("path", json!("/v1/app/../hermes/x")).to_string(),
        ),
        ("dot", with("path", json!("/v1/app/./state")).to_string()),
        (
            "long path",
            with(
                "path",
                json!(format!("/v1/app/{}", "a".repeat(2049 - "/v1/app/".len()))),
            )
            .to_string(),
        ),
        ("GET digest", with("body_sha256", json!(digest)).to_string()),
        ("POST null digest", post(Value::Null).to_string()),
        ("POST missing digest", missing_post_digest.to_string()),
        (
            "uppercase digest",
            post(json!(digest.to_uppercase())).to_string(),
        ),
        ("short digest", post(json!(&digest[1..])).to_string()),
        ("non-hex digest", post(json!("g".repeat(64))).to_string()),
    ];
    for (case, body) in cases {
        let response =
            authorization_response(device.clone(), Some(TOKEN), Some("user_paul"), body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{case}");
        let body = response_body(response).await;
        assert_eq!(body["error_kind"], "invalid_direct_authorization", "{case}");
        assert_eq!(body["retryable"], false, "{case}");
    }
}

#[tokio::test]
async fn direct_authorization_requires_service_authorization_and_a_verified_user() {
    let root = TempDir::new().unwrap();
    let device = test_app(&root);
    state_for(device.clone(), "user_paul").await;
    let valid = get_request("/v1/app/state").to_string();

    for (token, user) in [
        (None, Some("user_paul")),
        (Some("wrong-token"), Some("user_paul")),
    ] {
        let response = authorization_response(device.clone(), token, user, valid.clone()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    // Service authorization is checked before the request is interpreted.
    let response = authorization_response(device.clone(), None, None, "not json").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = authorization_response(device.clone(), Some(TOKEN), None, valid.clone()).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = authorization_response(device, Some(TOKEN), Some(" "), valid).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn direct_authorization_never_mints_a_chat_identity() {
    let root = TempDir::new().unwrap();
    let device = test_app(&root);
    let user_root = |user: &str| {
        root.path()
            .join("users")
            .join(hex::encode(sha2::Sha256::digest(user.as_bytes())))
    };

    let response = authorize(device.clone(), "user_new", get_request("/v1/app/state")).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = response_body(response).await;
    assert_eq!(body["error_kind"], "chat_identity_missing");
    assert_eq!(body["retryable"], false);
    assert!(
        !user_root("user_new").exists(),
        "signing must not create any per-user state"
    );

    // A half-present pair fails closed instead of signing or replacing the key.
    state_for(device.clone(), "user_paul").await;
    let identity_path = user_root("user_paul").join("finite-home/identity/identity.json");
    let identity_bytes = fs::read(&identity_path).unwrap();
    fs::remove_file(user_root("user_paul").join("chat/client.sqlite3")).unwrap();
    let response = authorize(test_app(&root), "user_paul", get_request("/v1/app/state")).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(fs::read(&identity_path).unwrap(), identity_bytes);
}

fn assert_nonce_tag(tag: &serde_json::Value) {
    let tag = tag.as_array().expect("nonce tag is an array");
    assert_eq!(tag[0], "nonce");
    let value = tag[1].as_str().unwrap();
    assert_eq!(value.len(), 32);
    assert!(value.bytes().all(|byte| byte.is_ascii_hexdigit()));
}
