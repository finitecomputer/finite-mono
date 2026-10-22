use super::*;
use crate::hosted_hermes_session::NativeSessions;
use crate::store::hosted_hermes::HostedLogin;
use axum::http::{HeaderMap, HeaderValue};
use axum::{
    Json, Router,
    routing::{get, post},
};

// Public certificate of a discarded throwaway key; it only exercises parsing.
const TEST_ROOT: &str = "-----BEGIN CERTIFICATE-----
MIIBvzCCAWWgAwIBAgIUelO84Zk/KNWMdFMuErka3EvT+64wCgYIKoZIzj0EAwIw
NDEyMDAGA1UEAwwpZmluaXRlLWNvcmUtdGVzdC1kZXZlbG9wbWVudC1pbmdyZXNz
LXJvb3QwIBcNMjYxMDEzMTQ1NDAzWhgPMjEyNjA5MTkxNDU0MDNaMDQxMjAwBgNV
BAMMKWZpbml0ZS1jb3JlLXRlc3QtZGV2ZWxvcG1lbnQtaW5ncmVzcy1yb290MFkw
EwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEDM87VzGQ4KY8il8O0lGcKW5CMrR/AvKL
ckG11ZEstqDlMZm0vmAmhCYtF2CVZV9kgP4OBYy6iablQhdN0caEyKNTMFEwHQYD
VR0OBBYEFO156XZkPG1I1ELtDB9Lfu9ijeHcMB8GA1UdIwQYMBaAFO156XZkPG1I
1ELtDB9Lfu9ijeHcMA8GA1UdEwEB/wQFMAMBAf8wCgYIKoZIzj0EAwIDSAAwRQIg
YMfqXSYc4HOBLvB8BgNOcOET9OU1HVP5v0onHOl3nX8CIQCA7RDXgtRx9bVMmQUI
xczIjuYUJveLuDOA1+YKQIrZlA==
-----END CERTIFICATE-----
";

fn config(path: &std::path::Path, address: &str) -> String {
    serde_json::json!({"rootCertificateFile": path, "loopbackAddress": address}).to_string()
}

#[test]
fn configuration_only_narrows_transport_to_loopback_and_an_absolute_ca() {
    let root = std::path::Path::new("/var/lib/devfinity/root.crt");
    for address in ["127.0.0.1", "::1"] {
        assert!(DevelopmentIngress::from_json(&config(root, address)).is_ok());
    }
    for address in ["10.0.0.1", "0.0.0.0", "203.0.113.7", "::"] {
        assert!(DevelopmentIngress::from_json(&config(root, address)).is_err());
    }
    assert!(DevelopmentIngress::from_json(&config("root.crt".as_ref(), "127.0.0.1")).is_err());
    for invalid in [
        "{}",
        r#"{"rootCertificateFile":"/root.crt"}"#,
        r#"{"rootCertificateFile":"/root.crt","loopbackAddress":"127.0.0.1","extra":1}"#,
    ] {
        assert!(DevelopmentIngress::from_json(invalid).is_err());
    }
}

#[tokio::test]
async fn grant_resolves_the_configured_origin_host_to_loopback_and_fails_closed_without_ca() {
    async fn login() -> (HeaderMap, Json<serde_json::Value>) {
        let mut headers = HeaderMap::new();
        headers.append(
            "set-cookie",
            HeaderValue::from_static("hermes_session_at=synthetic-access; HttpOnly"),
        );
        (headers, Json(serde_json::json!({"ok": true})))
    }
    async fn identity() -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "provider": "basic", "user_id": "native-user",
            "expires_at": time::OffsetDateTime::now_utc().unix_timestamp() + 60,
        }))
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = Router::new()
        .route("/auth/password-login", post(login))
        .route("/api/auth/me", get(identity));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let binding = HostedLogin {
        runtime_id: "runtime".into(),
        creation_request_id: "creation".into(),
        generation: 1,
        // A reserved name that public DNS never resolves.
        base_url: format!("http://agents.devfinity.test:{port}/"),
        username: "native-user".into(),
        password: "synthetic-password".into(),
    };
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root.crt");
    let ingress = DevelopmentIngress::from_json(&config(&root, "127.0.0.1")).unwrap();
    let sessions = NativeSessions::with_development_ingress(ingress);
    // The CA may appear after Core starts; until then no request is attempted.
    assert_eq!(
        sessions.grant(&binding).await.err(),
        Some("development ingress CA unavailable")
    );
    for invalid in [
        "not a certificate".to_string(),
        TEST_ROOT.replace("MIIBvzCC", "MIIBvzCD"),
    ] {
        std::fs::write(&root, invalid).unwrap();
        assert_eq!(
            sessions.grant(&binding).await.err(),
            Some("development ingress CA invalid")
        );
    }
    std::fs::write(&root, TEST_ROOT).unwrap();
    let grant = sessions.grant(&binding).await.unwrap();
    assert_eq!(grant.base_url, binding.base_url);
    assert_eq!(grant.access_token, "synthetic-access");
    server.abort();
}
