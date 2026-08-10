//! Resolver for Core's signed service directory
//! (`finite_service_directory.v1`).
//!
//! One signed document advertises service base URLs, client version
//! expectations, and release channel heads. This crate owns the typed
//! document, its fetch-and-verify path, and the on-disk cache agents keep at
//! `<agent home>/service-directory.json`. Canonicalization and signature
//! encoding are `finite-release`'s — the same single implementation Core
//! signs with.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use ed25519_dalek::{SigningKey, VerifyingKey};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub const SERVICE_DIRECTORY_SCHEMA: &str = "finite_service_directory.v1";
/// Channel-head kind keys, mirroring Core's bundle artifact kinds.
pub const SKILLS_BUNDLE_KIND: &str = "skills_bundle";
pub const PAYLOAD_BUNDLE_KIND: &str = "payload_bundle";

/// Anti-replay bound: a directory whose `generated_at` is older than this is
/// refused by [`fetch_and_verify`] and [`read_cache`]. Core regenerates the
/// document per request, so a genuine document is always minutes old at most;
/// only a replayed capture can be a day stale. 24h leaves generous room for
/// caches and clock skew while bounding how far back a replay can drag a
/// fleet.
pub const DIRECTORY_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// How far into the future a `generated_at` may sit before it is rejected. A
/// validly-signed but future-dated document would otherwise pass the max-age
/// check (now − future is negative) and then become the monotonic replay
/// floor, freezing all updates until wall-clock time catches up. 5 minutes
/// absorbs ordinary clock skew while bounding that freeze.
pub const DIRECTORY_MAX_FUTURE_SKEW: Duration = Duration::from_secs(5 * 60);

const DEFAULT_FETCH_CAP_BYTES: usize = 64 * 1024;
const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub enum DirectoryError {
    #[error("I/O failure: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON failure: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid service directory: {0}")]
    InvalidDocument(String),
    #[error("service directory fetch failed: {0}")]
    Fetch(String),
    #[error("service directory verification failed: {0}")]
    Verification(#[from] finite_release::ReleaseError),
    #[error(
        "service directory is expired: generated_at {generated_at} is older than the \
         {max_age_secs}s max age"
    )]
    Expired {
        generated_at: String,
        max_age_secs: u64,
    },
    #[error(
        "service directory replay refused: generated_at {generated_at} is older than the \
         last-seen floor {floor}"
    )]
    Replayed { generated_at: String, floor: String },
    #[error(
        "service directory is future-dated: generated_at {generated_at} is more than \
         {max_skew_secs}s ahead of now"
    )]
    Future {
        generated_at: String,
        max_skew_secs: u64,
    },
}

/// One advertised service: where to reach it and, optionally, what client
/// versions it expects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceEntryV1 {
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_version: Option<ClientVersionV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientVersionV1 {
    pub min: String,
    pub current: String,
}

/// One release channel head: the bundle artifact agents on that channel
/// converge on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelBundleV1 {
    pub artifact_id: String,
    pub version_label: String,
    pub tarball_url: String,
    /// Derived by convention: `tarball_url + ".manifest.json"`.
    pub manifest_url: String,
    pub tarball_sha256: String,
}

/// The signed directory document. `channels` maps channel name
/// (`stable`/`canary`) to artifact kind (`skills_bundle`/`payload_bundle`)
/// to that channel's head bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceDirectoryV1 {
    pub schema: String,
    pub generated_at: String,
    pub services: BTreeMap<String, ServiceEntryV1>,
    pub channels: BTreeMap<String, BTreeMap<String, ChannelBundleV1>>,
    /// Short id of the signing key ([`finite_release::verifying_key_id`]);
    /// verification refuses a document naming a different key than the one
    /// provided.
    pub key_id: String,
    /// `ed25519:<base64>` over the canonical document minus this field
    /// (see [`finite_release::document_signing_bytes`]).
    pub signature: String,
}

impl ServiceDirectoryV1 {
    /// Parse and signature-verify a directory document. Verification runs
    /// over the raw JSON before typed deserialization, so a document this
    /// version does not fully understand can never verify by accident.
    pub fn from_verified_json_bytes(
        bytes: &[u8],
        public_key: &VerifyingKey,
    ) -> Result<Self, DirectoryError> {
        let document: Value = serde_json::from_slice(bytes)?;
        let schema = document.get("schema").and_then(Value::as_str);
        if schema != Some(SERVICE_DIRECTORY_SCHEMA) {
            return Err(DirectoryError::InvalidDocument(format!(
                "expected schema {SERVICE_DIRECTORY_SCHEMA:?}, found {schema:?}"
            )));
        }
        check_document_key_id(&document, public_key)?;
        finite_release::verify_document_signature(&document, public_key)?;
        let directory: Self = serde_json::from_value(document)
            .map_err(|error| DirectoryError::InvalidDocument(error.to_string()))?;
        Ok(directory)
    }

    /// Sign this document with the release key, filling `key_id` from the key
    /// and replacing any existing signature.
    pub fn sign(&mut self, signing_key: &SigningKey) -> Result<(), DirectoryError> {
        self.key_id = finite_release::verifying_key_id(&signing_key.verifying_key());
        self.signature = String::new();
        let document = serde_json::to_value(&self)?;
        self.signature = finite_release::sign_document(&document, signing_key)?;
        Ok(())
    }

    /// Verify this document's signature (and its `key_id`) against the
    /// release public key.
    pub fn verify_signature(&self, public_key: &VerifyingKey) -> Result<(), DirectoryError> {
        let document = serde_json::to_value(self)?;
        check_document_key_id(&document, public_key)?;
        finite_release::verify_document_signature(&document, public_key)?;
        Ok(())
    }

    /// This document's `generated_at` as a parsed instant.
    pub fn generated_at_time(&self) -> Result<OffsetDateTime, DirectoryError> {
        OffsetDateTime::parse(&self.generated_at, &Rfc3339).map_err(|error| {
            DirectoryError::InvalidDocument(format!(
                "generated_at {:?} is not RFC 3339: {error}",
                self.generated_at
            ))
        })
    }

    /// Anti-replay max-age check: refuse a document generated more than
    /// `max_age` before now (see [`DIRECTORY_MAX_AGE`]), and refuse one dated
    /// more than [`DIRECTORY_MAX_FUTURE_SKEW`] in the future — a future
    /// timestamp trivially passes the max-age test and would otherwise poison
    /// the monotonic replay floor.
    pub fn check_max_age(&self, max_age: Duration) -> Result<(), DirectoryError> {
        let generated_at = self.generated_at_time()?;
        let now = OffsetDateTime::now_utc();
        if now - generated_at > max_age {
            return Err(DirectoryError::Expired {
                generated_at: self.generated_at.clone(),
                max_age_secs: max_age.as_secs(),
            });
        }
        if (generated_at - now).whole_seconds() > DIRECTORY_MAX_FUTURE_SKEW.as_secs() as i64 {
            return Err(DirectoryError::Future {
                generated_at: self.generated_at.clone(),
                max_skew_secs: DIRECTORY_MAX_FUTURE_SKEW.as_secs(),
            });
        }
        Ok(())
    }

    /// Persisted-floor check: refuse a document strictly older than the
    /// caller's durable last-seen `generated_at`. An equal timestamp is
    /// accepted — Core idempotently re-serving the same document is not a
    /// replay.
    pub fn check_not_older_than(&self, floor: &str) -> Result<(), DirectoryError> {
        let generated_at = self.generated_at_time()?;
        let Ok(floor_time) = OffsetDateTime::parse(floor, &Rfc3339) else {
            // An unparseable persisted floor never wedges convergence.
            return Ok(());
        };
        if generated_at < floor_time {
            return Err(DirectoryError::Replayed {
                generated_at: self.generated_at.clone(),
                floor: floor.to_owned(),
            });
        }
        Ok(())
    }

    pub fn service(&self, name: &str) -> Option<&ServiceEntryV1> {
        self.services.get(name)
    }

    pub fn service_base_url(&self, name: &str) -> Option<&str> {
        self.services.get(name).map(|entry| entry.base_url.as_str())
    }

    /// The head bundle for `channel` (`stable`/`canary`) and `kind`
    /// (`skills_bundle`/`payload_bundle`), if the channel has one.
    pub fn channel_bundle(&self, channel: &str, kind: &str) -> Option<&ChannelBundleV1> {
        self.channels.get(channel)?.get(kind)
    }
}

/// Raw-document `key_id` check, shared by verify-before-parse and typed
/// verification: a document naming a different key than the verification key
/// is a diagnosable rotation failure, not an opaque bad signature.
fn check_document_key_id(
    document: &Value,
    public_key: &VerifyingKey,
) -> Result<(), DirectoryError> {
    let document_key_id = document
        .get("key_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            DirectoryError::InvalidDocument("document has no key_id field".to_owned())
        })?;
    let verification_key_id = finite_release::verifying_key_id(public_key);
    if document_key_id != verification_key_id {
        return Err(DirectoryError::Verification(
            finite_release::ReleaseError::KeyIdMismatch {
                document_key_id: document_key_id.to_owned(),
                verification_key_id,
            },
        ));
    }
    Ok(())
}

/// Bounds for [`fetch_and_verify`].
#[derive(Debug, Clone)]
pub struct FetchLimits {
    /// Response size cap; the directory is a small document.
    pub max_bytes: usize,
    pub timeout: Duration,
    /// Dev-harness-only escape hatch admitting plain-http directory URLs
    /// (`FINITE_ALLOW_INSECURE_BUNDLE_URL=1`).
    pub allow_insecure_url: bool,
    /// Anti-replay max age for the fetched document's `generated_at`
    /// ([`DIRECTORY_MAX_AGE`] by default); `None` disables the check.
    pub max_age: Option<Duration>,
}

impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_FETCH_CAP_BYTES,
            timeout: DEFAULT_FETCH_TIMEOUT,
            allow_insecure_url: false,
            max_age: Some(DIRECTORY_MAX_AGE),
        }
    }
}

/// Fetch the directory from `url`, enforcing the shared https-or-dev-escape
/// URL policy and the size cap, and verify its signature before returning it.
pub async fn fetch_and_verify(
    url: &str,
    public_key: &VerifyingKey,
    limits: &FetchLimits,
) -> Result<ServiceDirectoryV1, DirectoryError> {
    finite_release::check_bundle_url_policy(url, limits.allow_insecure_url)?;
    let client = reqwest::Client::builder()
        .timeout(limits.timeout)
        .connect_timeout(limits.timeout.min(Duration::from_secs(10)))
        .build()
        .map_err(|error| DirectoryError::Fetch(error.to_string()))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| DirectoryError::Fetch(error.to_string()))?;
    if !response.status().is_success() {
        return Err(DirectoryError::Fetch(format!(
            "directory fetch failed with {}",
            response.status()
        )));
    }
    if let Some(length) = response.content_length()
        && length > limits.max_bytes as u64
    {
        return Err(DirectoryError::Fetch(format!(
            "directory exceeds the {}-byte cap",
            limits.max_bytes
        )));
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| DirectoryError::Fetch(error.to_string()))?;
        if bytes.len() + chunk.len() > limits.max_bytes {
            return Err(DirectoryError::Fetch(format!(
                "directory exceeds the {}-byte cap",
                limits.max_bytes
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    let directory = ServiceDirectoryV1::from_verified_json_bytes(&bytes, public_key)?;
    if let Some(max_age) = limits.max_age {
        directory.check_max_age(max_age)?;
    }
    Ok(directory)
}

/// Atomically write the verified directory to `path` (tempfile + rename in
/// the destination directory).
pub fn write_cache(directory: &ServiceDirectoryV1, path: &Path) -> Result<(), DirectoryError> {
    let parent = path.parent().ok_or_else(|| {
        DirectoryError::Io(std::io::Error::other("cache path has no parent directory"))
    })?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let mut bytes = serde_json::to_vec_pretty(directory)?;
    bytes.push(b'\n');
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| DirectoryError::Io(error.error))?;
    Ok(())
}

/// Read a cached directory and re-verify its signature. The cache lives on
/// shared `/data`, so it is never trusted without re-proving the signature —
/// nor without the [`DIRECTORY_MAX_AGE`] anti-replay check.
pub fn read_cache(
    path: &Path,
    public_key: &VerifyingKey,
) -> Result<ServiceDirectoryV1, DirectoryError> {
    let directory =
        ServiceDirectoryV1::from_verified_json_bytes(&std::fs::read(path)?, public_key)?;
    directory.check_max_age(DIRECTORY_MAX_AGE)?;
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[11_u8; 32])
    }

    fn now_rfc3339() -> String {
        OffsetDateTime::now_utc().format(&Rfc3339).unwrap()
    }

    fn signed_directory() -> ServiceDirectoryV1 {
        signed_directory_generated_at(&now_rfc3339())
    }

    fn signed_directory_generated_at(generated_at: &str) -> ServiceDirectoryV1 {
        let mut directory = ServiceDirectoryV1 {
            schema: SERVICE_DIRECTORY_SCHEMA.to_owned(),
            generated_at: generated_at.to_owned(),
            services: BTreeMap::from([
                (
                    "sites".to_owned(),
                    ServiceEntryV1 {
                        base_url: "http://192.168.64.1:18789".to_owned(),
                        client_version: Some(ClientVersionV1 {
                            min: "0.0.0".to_owned(),
                            current: "0.0.0".to_owned(),
                        }),
                    },
                ),
                (
                    "core".to_owned(),
                    ServiceEntryV1 {
                        base_url: "http://192.168.64.1:4200".to_owned(),
                        client_version: None,
                    },
                ),
            ]),
            channels: BTreeMap::from([(
                "canary".to_owned(),
                BTreeMap::from([(
                    SKILLS_BUNDLE_KIND.to_owned(),
                    ChannelBundleV1 {
                        artifact_id: "devfinity-skills-abc".to_owned(),
                        version_label: "devfinity-abc".to_owned(),
                        tarball_url: "http://192.168.64.1:18791/devfinity-skills-abc.tar.gz"
                            .to_owned(),
                        manifest_url:
                            "http://192.168.64.1:18791/devfinity-skills-abc.tar.gz.manifest.json"
                                .to_owned(),
                        tarball_sha256: "1".repeat(64),
                    },
                )]),
            )]),
            key_id: String::new(),
            signature: String::new(),
        };
        directory.sign(&test_signing_key()).unwrap();
        directory
    }

    #[test]
    fn a_signed_directory_roundtrips_and_exposes_typed_lookups() {
        let directory = signed_directory();
        directory
            .verify_signature(&test_signing_key().verifying_key())
            .unwrap();

        let bytes = serde_json::to_vec(&directory).unwrap();
        let parsed = ServiceDirectoryV1::from_verified_json_bytes(
            &bytes,
            &test_signing_key().verifying_key(),
        )
        .unwrap();
        assert_eq!(parsed, directory);
        assert_eq!(
            parsed.service_base_url("sites"),
            Some("http://192.168.64.1:18789")
        );
        assert_eq!(
            parsed.service("sites").unwrap().client_version,
            Some(ClientVersionV1 {
                min: "0.0.0".to_owned(),
                current: "0.0.0".to_owned(),
            })
        );
        assert!(parsed.service_base_url("brain").is_none());
        let bundle = parsed.channel_bundle("canary", SKILLS_BUNDLE_KIND).unwrap();
        assert_eq!(bundle.artifact_id, "devfinity-skills-abc");
        assert_eq!(
            bundle.manifest_url,
            format!("{}.manifest.json", bundle.tarball_url)
        );
        assert!(
            parsed
                .channel_bundle("canary", PAYLOAD_BUNDLE_KIND)
                .is_none()
        );
        assert!(
            parsed
                .channel_bundle("stable", SKILLS_BUNDLE_KIND)
                .is_none()
        );
    }

    #[test]
    fn a_tampered_document_or_wrong_key_is_refused() {
        let directory = signed_directory();
        let mut document = serde_json::to_value(&directory).unwrap();
        document["services"]["sites"]["base_url"] = serde_json::json!("http://forged.invalid");
        let error = ServiceDirectoryV1::from_verified_json_bytes(
            &serde_json::to_vec(&document).unwrap(),
            &test_signing_key().verifying_key(),
        )
        .unwrap_err();
        assert!(matches!(error, DirectoryError::Verification(_)));

        let wrong_key = SigningKey::from_bytes(&[12_u8; 32]).verifying_key();
        let error = ServiceDirectoryV1::from_verified_json_bytes(
            &serde_json::to_vec(&directory).unwrap(),
            &wrong_key,
        )
        .unwrap_err();
        assert!(matches!(error, DirectoryError::Verification(_)));

        let mut wrong_schema = serde_json::to_value(&directory).unwrap();
        wrong_schema["schema"] = serde_json::json!("finite_service_directory.v2");
        let error = ServiceDirectoryV1::from_verified_json_bytes(
            &serde_json::to_vec(&wrong_schema).unwrap(),
            &test_signing_key().verifying_key(),
        )
        .unwrap_err();
        assert!(matches!(error, DirectoryError::InvalidDocument(_)));
    }

    #[test]
    fn an_unknown_field_cannot_ride_through_the_typed_document() {
        // A field the struct would drop must fail parsing loudly: silently
        // dropping it would make the re-serialized cache bytes diverge from
        // what the signature covers.
        let mut document = serde_json::to_value(signed_directory()).unwrap();
        document["extra"] = serde_json::json!("field");
        document["signature"] = serde_json::json!(
            finite_release::sign_document(&document, &test_signing_key()).unwrap()
        );
        let error = ServiceDirectoryV1::from_verified_json_bytes(
            &serde_json::to_vec(&document).unwrap(),
            &test_signing_key().verifying_key(),
        )
        .unwrap_err();
        assert!(matches!(error, DirectoryError::InvalidDocument(_)));
    }

    #[test]
    fn the_cache_roundtrips_and_is_reverified_on_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-home/service-directory.json");
        let directory = signed_directory();
        write_cache(&directory, &path).unwrap();
        let read = read_cache(&path, &test_signing_key().verifying_key()).unwrap();
        assert_eq!(read, directory);

        // A tampered cache on shared /data must be refused on read.
        let mut tampered: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        tampered["services"]["sites"]["base_url"] = serde_json::json!("http://forged.invalid");
        std::fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
        let error = read_cache(&path, &test_signing_key().verifying_key()).unwrap_err();
        assert!(matches!(error, DirectoryError::Verification(_)));
    }

    async fn serve_bytes(body: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0_u8; 8192];
                    let _ = stream.read(&mut buffer).await;
                    let header = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn fetch_and_verify_accepts_a_signed_directory_over_the_dev_escape() {
        let directory = signed_directory();
        let base_url = serve_bytes(serde_json::to_vec(&directory).unwrap()).await;
        let limits = FetchLimits {
            allow_insecure_url: true,
            ..FetchLimits::default()
        };
        let fetched = fetch_and_verify(
            &format!("{base_url}/api/core/v1/service-directory"),
            &test_signing_key().verifying_key(),
            &limits,
        )
        .await
        .unwrap();
        assert_eq!(fetched, directory);
    }

    #[tokio::test]
    async fn fetch_refuses_plain_http_without_the_dev_escape_and_oversized_documents() {
        let error = fetch_and_verify(
            "http://127.0.0.1:1/api/core/v1/service-directory",
            &test_signing_key().verifying_key(),
            &FetchLimits::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            DirectoryError::Verification(finite_release::ReleaseError::UrlPolicy(_))
        ));

        let directory = signed_directory();
        let base_url = serve_bytes(serde_json::to_vec(&directory).unwrap()).await;
        let limits = FetchLimits {
            max_bytes: 16,
            allow_insecure_url: true,
            ..FetchLimits::default()
        };
        let error = fetch_and_verify(
            &format!("{base_url}/api/core/v1/service-directory"),
            &test_signing_key().verifying_key(),
            &limits,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, DirectoryError::Fetch(_)));
    }

    #[test]
    fn a_document_naming_a_different_key_id_is_a_diagnosable_mismatch() {
        let directory = signed_directory();
        assert_eq!(
            directory.key_id,
            finite_release::verifying_key_id(&test_signing_key().verifying_key()),
            "signing must fill key_id from the signing key"
        );
        let wrong_key = SigningKey::from_bytes(&[12_u8; 32]).verifying_key();
        let error = directory.verify_signature(&wrong_key).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains(&directory.key_id)
                && message.contains(&finite_release::verifying_key_id(&wrong_key)),
            "the mismatch must name both key ids: {message}"
        );

        // A document with no key_id at all (a pre-key_id capture) is refused
        // before any signature work.
        let mut document = serde_json::to_value(&directory).unwrap();
        document.as_object_mut().unwrap().remove("key_id");
        let error = ServiceDirectoryV1::from_verified_json_bytes(
            &serde_json::to_vec(&document).unwrap(),
            &test_signing_key().verifying_key(),
        )
        .unwrap_err();
        assert!(matches!(error, DirectoryError::InvalidDocument(_)));
    }

    #[tokio::test]
    async fn a_stale_directory_is_refused_by_fetch_and_by_cache_read() {
        // Two days old: past the 24h max age even though the signature is
        // perfectly valid — the replay window is bounded.
        let stale_at = (OffsetDateTime::now_utc() - Duration::from_secs(48 * 60 * 60))
            .format(&Rfc3339)
            .unwrap();
        let stale = signed_directory_generated_at(&stale_at);
        let base_url = serve_bytes(serde_json::to_vec(&stale).unwrap()).await;
        let limits = FetchLimits {
            allow_insecure_url: true,
            ..FetchLimits::default()
        };
        let error = fetch_and_verify(
            &format!("{base_url}/api/core/v1/service-directory"),
            &test_signing_key().verifying_key(),
            &limits,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, DirectoryError::Expired { .. }));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("service-directory.json");
        write_cache(&stale, &path).unwrap();
        let error = read_cache(&path, &test_signing_key().verifying_key()).unwrap_err();
        assert!(matches!(error, DirectoryError::Expired { .. }));

        // Disabling the check (max_age: None) admits the same document.
        let no_age = FetchLimits {
            allow_insecure_url: true,
            max_age: None,
            ..FetchLimits::default()
        };
        fetch_and_verify(
            &format!("{base_url}/api/core/v1/service-directory"),
            &test_signing_key().verifying_key(),
            &no_age,
        )
        .await
        .unwrap();
    }

    #[test]
    fn the_floor_check_refuses_older_and_admits_equal_and_newer_documents() {
        let directory = signed_directory_generated_at("2026-08-08T12:00:00Z");
        // Strictly newer floor: refused with a distinct replay error.
        let error = directory
            .check_not_older_than("2026-08-08T12:00:01Z")
            .unwrap_err();
        assert!(matches!(error, DirectoryError::Replayed { .. }));
        // Equal: Core idempotently re-serving the same document is accepted.
        directory
            .check_not_older_than("2026-08-08T12:00:00Z")
            .unwrap();
        // Older floor: the document advances it.
        directory
            .check_not_older_than("2026-08-08T11:59:59Z")
            .unwrap();
        // An unparseable persisted floor never wedges convergence.
        directory.check_not_older_than("not a timestamp").unwrap();
    }

    #[test]
    fn a_future_dated_directory_is_refused_before_it_can_poison_the_floor() {
        // Well beyond the 5-minute skew tolerance: a validly-signed but
        // future-dated document must be rejected distinctly, not admitted as
        // "not stale" (now − future is negative) and then frozen in as the
        // monotonic replay floor.
        let future_at = (OffsetDateTime::now_utc() + Duration::from_secs(60 * 60))
            .format(&Rfc3339)
            .unwrap();
        let directory = signed_directory_generated_at(&future_at);
        let error = directory.check_max_age(DIRECTORY_MAX_AGE).unwrap_err();
        assert!(matches!(error, DirectoryError::Future { .. }));

        // Inside the tolerance (a minute ahead): ordinary clock skew is fine.
        let near = (OffsetDateTime::now_utc() + Duration::from_secs(60))
            .format(&Rfc3339)
            .unwrap();
        signed_directory_generated_at(&near)
            .check_max_age(DIRECTORY_MAX_AGE)
            .unwrap();
    }

    #[tokio::test]
    async fn a_tampered_fetched_document_is_refused() {
        let mut document = serde_json::to_value(signed_directory()).unwrap();
        document["generated_at"] = serde_json::json!("2027-01-01T00:00:00Z");
        let base_url = serve_bytes(serde_json::to_vec(&document).unwrap()).await;
        let limits = FetchLimits {
            allow_insecure_url: true,
            ..FetchLimits::default()
        };
        let error = fetch_and_verify(
            &format!("{base_url}/api/core/v1/service-directory"),
            &test_signing_key().verifying_key(),
            &limits,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, DirectoryError::Verification(_)));
    }
}
