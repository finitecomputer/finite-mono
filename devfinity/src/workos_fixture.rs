//! A deliberately small local WorkOS boundary used by devfinity. It exposes
//! only the two reads Core uses: JWKS and a verified User Management lookup.

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rsa::{
    RsaPrivateKey, RsaPublicKey,
    pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding},
    traits::PublicKeyParts,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

pub const CLIENT_ID: &str = "client_devfinity";
pub const OPERATOR_ORG_ID: &str = "org_devfinity_operator";
pub const CUSTOMER_SUBJECT: &str = "user_devfinity";
pub const CUSTOMER_EMAIL: &str = "devfinity@finite.computer";
pub const OPERATOR_SUBJECT: &str = "user_devfinity_operator";
pub const OPERATOR_EMAIL: &str = "operator@finite.computer";
const KEY_ID: &str = "devfinity-local-rsa-1";

#[derive(Clone)]
pub struct FixturePaths {
    pub root: PathBuf,
    pub private_key: PathBuf,
    pub api_key: PathBuf,
    pub customer_token: PathBuf,
    pub operator_token: PathBuf,
}

impl FixturePaths {
    pub fn new(root: PathBuf) -> Self {
        Self {
            private_key: root.join("workos-fixture-private.pem"),
            api_key: root.join("workos-fixture-api-key"),
            customer_token: root.join("dashboard-customer.jwt"),
            operator_token: root.join("operator.jwt"),
            root,
        }
    }
}

pub fn prepare(paths: &FixturePaths, issuer: &str) -> Result<()> {
    fs::create_dir_all(&paths.root)?;
    if !paths.private_key.exists() {
        let mut rng = rand::rngs::OsRng;
        let key = RsaPrivateKey::new(&mut rng, 2048)?;
        let pem = key.to_pkcs8_pem(LineEnding::LF)?;
        write_private(&paths.private_key, pem.as_bytes())?;
    }
    if !paths.api_key.exists() {
        write_private(&paths.api_key, random_token()?.as_bytes())?;
    }
    let private_pem = fs::read(&paths.private_key)?;
    let encoding = EncodingKey::from_rsa_pem(&private_pem)?;
    write_private(
        &paths.customer_token,
        mint(&encoding, issuer, CUSTOMER_SUBJECT, None)?.as_bytes(),
    )?;
    write_private(
        &paths.operator_token,
        mint(&encoding, issuer, OPERATOR_SUBJECT, Some(OPERATOR_ORG_ID))?.as_bytes(),
    )?;
    Ok(())
}

pub fn prepare_if_missing(paths: &FixturePaths, issuer: &str) -> Result<()> {
    let prepared = [
        &paths.private_key,
        &paths.api_key,
        &paths.customer_token,
        &paths.operator_token,
    ]
    .into_iter()
    .all(|path| path.exists());
    if prepared {
        return Ok(());
    }
    prepare(paths, issuer)
}

pub async fn serve(addr: SocketAddr, paths: FixturePaths) -> Result<()> {
    let app = router(&paths)?;
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;
    Ok(())
}

fn router(paths: &FixturePaths) -> Result<Router> {
    let api_key = fs::read_to_string(&paths.api_key).context("read WorkOS fixture API key")?;
    let private_pem = fs::read(&paths.private_key).context("read WorkOS fixture signing key")?;
    let private = RsaPrivateKey::from_pkcs8_pem(std::str::from_utf8(&private_pem)?)?;
    let public = RsaPublicKey::from(&private);
    let users = BTreeMap::from([
        (
            CUSTOMER_SUBJECT.to_string(),
            User {
                id: CUSTOMER_SUBJECT.into(),
                email: CUSTOMER_EMAIL.into(),
                email_verified: true,
            },
        ),
        (
            OPERATOR_SUBJECT.to_string(),
            User {
                id: OPERATOR_SUBJECT.into(),
                email: OPERATOR_EMAIL.into(),
                email_verified: true,
            },
        ),
    ]);
    let state = Arc::new(AppState {
        api_key: api_key.trim().to_string(),
        jwks: Jwks {
            keys: vec![jwk(&public)],
        },
        users,
    });
    Ok(Router::new()
        .route("/sso/jwks/{client_id}", get(jwks))
        .route("/user_management/users/{subject}", get(user))
        .with_state(state))
}

#[derive(Clone)]
struct AppState {
    api_key: String,
    jwks: Jwks,
    users: BTreeMap<String, User>,
}
#[derive(Serialize, Clone)]
struct Jwks {
    keys: Vec<Jwk>,
}
#[derive(Serialize, Clone)]
struct Jwk {
    kid: &'static str,
    kty: &'static str,
    alg: &'static str,
    #[serde(rename = "use")]
    key_use: &'static str,
    n: String,
    e: String,
}
#[derive(Serialize, Clone)]
struct User {
    id: String,
    email: String,
    email_verified: bool,
}
#[derive(Serialize)]
struct Claims<'a> {
    sub: &'a str,
    client_id: &'static str,
    iss: &'a str,
    exp: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    org_id: Option<&'a str>,
}

async fn jwks(
    Path(client_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if client_id == CLIENT_ID {
        (StatusCode::OK, Json(state.jwks.clone())).into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}
async fn user(
    Path(subject): Path<String>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let permitted = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|v| v == state.api_key);
    if !permitted {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.users.get(&subject) {
        Some(user) => (StatusCode::OK, Json(user)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
fn jwk(key: &RsaPublicKey) -> Jwk {
    Jwk {
        kid: KEY_ID,
        kty: "RSA",
        alg: "RS256",
        key_use: "sig",
        n: URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()),
        e: URL_SAFE_NO_PAD.encode(key.e().to_bytes_be()),
    }
}
fn mint(key: &EncodingKey, issuer: &str, subject: &str, org_id: Option<&str>) -> Result<String> {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KEY_ID.into());
    let exp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize + 60 * 60;
    Ok(encode(
        &header,
        &Claims {
            sub: subject,
            client_id: CLIENT_ID,
            iss: issuer,
            exp,
            org_id,
        },
        key,
    )?)
}
fn random_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("local credential generation failed: {error:?}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn write_private(path: &FsPath, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let temporary = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn fixture_paths() -> FixturePaths {
        let unique = random_token().expect("generate unique fixture directory");
        FixturePaths::new(std::env::temp_dir().join(format!("devfinity-workos-{unique}")))
    }

    #[cfg(unix)]
    #[test]
    fn generated_credentials_and_tokens_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let paths = fixture_paths();
        prepare(&paths, "http://fixture.invalid").expect("prepare fixture");

        for path in [
            &paths.private_key,
            &paths.api_key,
            &paths.customer_token,
            &paths.operator_token,
        ] {
            let mode = fs::metadata(path)
                .expect("fixture file metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{} must be owner-only", path.display());
        }

        fs::remove_dir_all(paths.root).expect("remove fixture directory");
    }

    #[test]
    fn fixture_process_start_preserves_prepared_tokens() {
        let paths = fixture_paths();
        prepare(&paths, "http://fixture.invalid").expect("prepare fixture");
        fs::write(&paths.customer_token, "prepared-customer-token")
            .expect("replace customer token sentinel");
        fs::write(&paths.operator_token, "prepared-operator-token")
            .expect("replace operator token sentinel");

        prepare_if_missing(&paths, "http://fixture.invalid")
            .expect("prepare fixture only when missing");

        assert_eq!(
            fs::read_to_string(&paths.customer_token).expect("read customer token"),
            "prepared-customer-token"
        );
        assert_eq!(
            fs::read_to_string(&paths.operator_token).expect("read operator token"),
            "prepared-operator-token"
        );

        fs::remove_dir_all(paths.root).expect("remove fixture directory");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fixture_exposes_only_expected_jwks_and_user_reads() {
        let paths = fixture_paths();
        prepare(&paths, "http://fixture.invalid").expect("prepare fixture");
        let api_key = fs::read_to_string(&paths.api_key).expect("read fixture API key");
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind fixture listener");
        let addr = listener.local_addr().expect("fixture listener address");
        let app = router(&paths).expect("build fixture router");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve fixture");
        });

        let result = tokio::task::spawn_blocking(move || {
            let base = format!("http://{addr}");

            let jwks: Value = ureq::get(&format!("{base}/sso/jwks/{CLIENT_ID}"))
                .call()?
                .into_json()?;
            let key = jwks["keys"].as_array().and_then(|keys| keys.first());
            assert_eq!(key.and_then(|key| key["kid"].as_str()), Some(KEY_ID));
            assert_eq!(key.and_then(|key| key["alg"].as_str()), Some("RS256"));
            assert!(matches!(
                ureq::get(&format!("{base}/sso/jwks/wrong-client")).call(),
                Err(ureq::Error::Status(404, _))
            ));

            let customer_url = format!("{base}/user_management/users/{CUSTOMER_SUBJECT}");
            assert!(matches!(
                ureq::get(&customer_url).call(),
                Err(ureq::Error::Status(401, _))
            ));
            assert!(matches!(
                ureq::get(&customer_url)
                    .set("authorization", "Bearer wrong")
                    .call(),
                Err(ureq::Error::Status(401, _))
            ));
            assert!(matches!(
                ureq::get(&format!("{base}/user_management/users/unknown"))
                    .set("authorization", &format!("Bearer {}", api_key.trim()))
                    .call(),
                Err(ureq::Error::Status(404, _))
            ));

            let user: Value = ureq::get(&customer_url)
                .set("authorization", &format!("Bearer {}", api_key.trim()))
                .call()?
                .into_json()?;
            assert_eq!(user["id"], CUSTOMER_SUBJECT);
            assert_eq!(user["email"], CUSTOMER_EMAIL);
            assert_eq!(user["email_verified"], true);

            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        })
        .await
        .expect("join fixture requests");

        server.abort();
        result.expect("fixture requests");
        fs::remove_dir_all(paths.root).expect("remove fixture directory");
    }
}
