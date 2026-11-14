//! The HTTP backend against a local fake of WorkOS user management and the
//! hosted chat device, and the provisioner's fail-closed choices.

use super::*;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode as HttpStatus};
use axum::routing::get;
use axum::{Json, Router};
use std::collections::HashMap;
use std::sync::Mutex;

const ORG: &str = "org_operators";

#[derive(Default)]
struct Fake {
    users: Mutex<Vec<serde_json::Value>>,
    created_bodies: Mutex<Vec<serde_json::Value>>,
    memberships: Mutex<Vec<serde_json::Value>>,
    hosted_calls: Mutex<Vec<String>>,
}

type Shared = Arc<Fake>;

fn bearer(headers: &HeaderMap, expected: &str) -> Result<(), HttpStatus> {
    (headers.get("authorization").and_then(|v| v.to_str().ok())
        == Some(format!("Bearer {expected}").as_str()))
    .then_some(())
    .ok_or(HttpStatus::UNAUTHORIZED)
}

async fn list_users(
    State(fake): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HttpStatus> {
    bearer(&headers, "workos-key")?;
    let email = query.get("email").cloned().unwrap_or_default();
    let data: Vec<_> = fake
        .users
        .lock()
        .unwrap()
        .iter()
        .filter(|user| user["email"] == email.as_str())
        .cloned()
        .collect();
    Ok(Json(
        serde_json::json!({ "object": "list", "data": data, "list_metadata": {} }),
    ))
}

async fn create_user(
    State(fake): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, HttpStatus> {
    bearer(&headers, "workos-key")?;
    fake.created_bodies.lock().unwrap().push(body.clone());
    let mut users = fake.users.lock().unwrap();
    if users.iter().any(|user| user["email"] == body["email"]) {
        return Err(HttpStatus::UNPROCESSABLE_ENTITY);
    }
    let user = serde_json::json!({
        "object": "user",
        "id": format!("user_{}", users.len() + 1),
        "email": body["email"],
        "email_verified": body["email_verified"],
    });
    users.push(user.clone());
    Ok(Json(user))
}

async fn get_user(
    State(fake): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, HttpStatus> {
    bearer(&headers, "workos-key")?;
    fake.users
        .lock()
        .unwrap()
        .iter()
        .find(|user| user["id"] == id.as_str())
        .cloned()
        .map(Json)
        .ok_or(HttpStatus::NOT_FOUND)
}

async fn memberships(
    State(fake): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, HttpStatus> {
    bearer(&headers, "workos-key")?;
    assert_eq!(query.get("organization_id").map(String::as_str), Some(ORG));
    assert_eq!(query.get("statuses").map(String::as_str), Some("active"));
    let user = query.get("user_id").cloned().unwrap_or_default();
    let data: Vec<_> = fake
        .memberships
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m["user_id"] == user.as_str())
        .cloned()
        .collect();
    Ok(Json(serde_json::json!({ "data": data })))
}

async fn hosted_state(
    State(fake): State<Shared>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, HttpStatus> {
    bearer(&headers, "hosted-token")?;
    let user = headers
        .get("x-finite-workos-user-id")
        .and_then(|v| v.to_str().ok())
        .ok_or(HttpStatus::BAD_REQUEST)?
        .to_string();
    fake.hosted_calls.lock().unwrap().push(user.clone());
    if user == "user_broken" {
        return Ok(Json(serde_json::json!({ "identity": null })));
    }
    Ok(Json(serde_json::json!({
        "identity": { "account_id": test_support::test_chat_account_id(&user).to_uppercase() }
    })))
}

async fn fake_backend() -> (OwnerIdentityProvisioner, Shared) {
    let fake = Shared::default();
    let app = Router::new()
        .route("/user_management/users", get(list_users).post(create_user))
        .route("/user_management/users/{id}", get(get_user))
        .route(
            "/user_management/organization_memberships",
            get(memberships),
        )
        .route("/v1/app/state", get(hosted_state))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let backend = HttpOwnerIdentityBackend::new(
        &base,
        "workos-key",
        ORG,
        &format!("{base}/"),
        "hosted-token",
    )
    .unwrap();
    (OwnerIdentityProvisioner::new(Arc::new(backend)), fake)
}

#[tokio::test]
async fn the_http_backend_finds_or_creates_one_unverified_user_and_reads_its_chat_account() {
    let (provisioner, fake) = fake_backend().await;
    assert_eq!(
        provisioner.find_user("new@example.com").await.unwrap(),
        None
    );
    let id = provisioner.ensure_user("new@example.com").await.unwrap();
    assert_eq!(id, "user_1");
    assert_eq!(
        fake.created_bodies.lock().unwrap().as_slice(),
        &[serde_json::json!({ "email": "new@example.com", "email_verified": false })]
    );
    // Found, not created, the second time.
    assert_eq!(
        provisioner.ensure_user("new@example.com").await.unwrap(),
        "user_1"
    );
    assert_eq!(fake.created_bodies.lock().unwrap().len(), 1);

    let chat = provisioner.chat_account("user_1").await.unwrap();
    assert_eq!(chat, test_support::test_chat_account_id("user_1"));
    assert_eq!(
        fake.hosted_calls.lock().unwrap().as_slice(),
        &["user_1".to_string()]
    );
    let error = provisioner.chat_account("user_broken").await.unwrap_err();
    assert!(
        matches!(error, CoreError::OwnerIdentityUnavailable(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn the_provisioner_refuses_two_workos_users_for_one_email() {
    let (provisioner, fake) = fake_backend().await;
    for id in ["user_a", "user_b"] {
        fake.users.lock().unwrap().push(serde_json::json!({
            "id": id, "email": "twice@example.com", "email_verified": true
        }));
    }
    let error = provisioner
        .ensure_user("twice@example.com")
        .await
        .unwrap_err();
    assert!(
        matches!(error, CoreError::OperatorAgentCreationRefused(_)),
        "{error:?}"
    );
    assert!(fake.created_bodies.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_cli_admin_must_be_a_verified_operator_org_member_at_workos() {
    let (provisioner, fake) = fake_backend().await;
    fake.users.lock().unwrap().extend([
        serde_json::json!({ "id": "user_admin", "email": "admin@finite.vip", "email_verified": true }),
        serde_json::json!({ "id": "user_member", "email": "member@finite.vip", "email_verified": true }),
        serde_json::json!({ "id": "user_unverified", "email": "unverified@finite.vip", "email_verified": false }),
    ]);
    fake.memberships.lock().unwrap().extend([
        serde_json::json!({ "user_id": "user_admin", "organization_id": ORG, "status": "active" }),
        serde_json::json!({ "user_id": "user_unverified", "organization_id": ORG, "status": "active" }),
        serde_json::json!({ "user_id": "user_member", "organization_id": "org_other", "status": "active" }),
    ]);
    let admin = |email: &str, id: &str| OperatorIdentity {
        email: email.into(),
        workos_user_id: id.into(),
    };
    provisioner
        .verify_operator(&admin(" Admin@Finite.VIP ", "user_admin"))
        .await
        .unwrap();
    for (email, id) in [
        ("someone@finite.vip", "user_admin"),
        ("member@finite.vip", "user_member"),
        ("unverified@finite.vip", "user_unverified"),
        ("admin@finite.vip", "user_missing"),
    ] {
        let error = provisioner
            .verify_operator(&admin(email, id))
            .await
            .unwrap_err();
        assert!(
            matches!(error, CoreError::OperatorIdentityMismatch),
            "{email}: {error:?}"
        );
    }
}

#[test]
fn the_backend_needs_both_hosted_settings_or_neither() {
    assert!(
        HttpOwnerIdentityBackend::new("https://api.workos.com", "k", ORG, "ftp://x", "t").is_err()
    );
    assert!(
        HttpOwnerIdentityBackend::new(
            "https://api.workos.com",
            "k",
            ORG,
            "http://127.0.0.1:1",
            " "
        )
        .is_err()
    );
    assert!(
        HttpOwnerIdentityBackend::new("https://api.workos.com", "", ORG, "http://127.0.0.1:1", "t")
            .is_err()
    );
}
