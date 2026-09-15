//! Operator transport for verified local Recovery Points, not a backup scheduler.
//! All objects are immutable; this module never deletes or configures lifecycle.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use aws_sdk_s3::{
    Client,
    error::ProvideErrorMetadata,
    primitives::ByteStream,
    types::{BucketVersioningStatus, ExpirationStatus},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::Digest;

use super::{
    BackupError, Manifest, ManifestBuffer, digest, hex, limits, persist, private_dir, read_bounded,
    validate_entry, validate_manifest,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub id: String,
    pub version_id: String,
    pub local_point: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteManifest {
    format: u32,
    local_point: String,
    /// Exact versions, including the local manifest as an immutable dependency.
    objects: BTreeMap<String, String>,
}

/// The caller supplies credentials/configuration. Merely loading the daemon
/// never resolves credentials or contacts an AWS endpoint.
pub struct Repository {
    client: Client,
    bucket: String,
    prefix: String,
}

impl Repository {
    pub fn new(client: Client, bucket: String, prefix: String) -> Result<Self, BackupError> {
        if !(3..=63).contains(&bucket.len())
            || !bucket.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
            || prefix.is_empty()
            || prefix.split('/').any(|part| part.is_empty() || part == "." || part == "..")
            || !prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_/".contains(&b))
            // S3 object keys are at most 1024 bytes, including our suffix.
            || prefix.len() + "/objects/".len() + 64 > 1024
        {
            return Err(BackupError::Invalid("invalid S3 bucket or prefix"));
        }
        Ok(Self {
            client,
            bucket,
            prefix,
        })
    }

    /// Verify a local point through the full restore boundary before uploading.
    pub async fn capture(&self, repository: &Path, id: &str) -> Result<Receipt, BackupError> {
        let verification = tempfile::tempdir_in(repository)?;
        super::restore(repository, id, &verification.path().join("verified"))?;
        let bytes = read_bounded(
            &repository.join("points").join(id),
            limits::MAX_BACKUP_MANIFEST_BYTES,
        )?;
        if digest(&bytes) != id {
            return Err(BackupError::Invalid("manifest checksum mismatch"));
        }
        let manifest: Manifest = serde_json::from_slice(&bytes)?;
        validate_manifest(&manifest)?;
        self.require_safe_bucket().await?;
        let mut objects = BTreeMap::new();
        // The local manifest validator bounds the number of entries.
        for entry in &manifest.entries {
            validate_entry(entry)?;
            if objects.contains_key(&entry.sha256) {
                continue;
            }
            let content =
                read_bounded(&repository.join("objects").join(&entry.sha256), entry.size)?;
            if content.len() as u64 != entry.size || digest(&content) != entry.sha256 {
                return Err(BackupError::Invalid("object checksum or size mismatch"));
            }
            let version = self.put_verified("objects", &entry.sha256, content).await?;
            objects.insert(entry.sha256.clone(), version);
        }
        let version = self.put_verified("objects", id, bytes).await?;
        objects.insert(id.to_string(), version);
        let mut encoded = ManifestBuffer(Vec::new());
        serde_json::to_writer(
            &mut encoded,
            &RemoteManifest {
                format: 1,
                local_point: id.to_string(),
                objects,
            },
        )?;
        let remote_id = digest(&encoded.0);
        self.require_safe_bucket().await?;
        // Only this final object makes the remotely verified set selectable.
        let version_id = self.put_verified("points", &remote_id, encoded.0).await?;
        Ok(Receipt {
            id: remote_id,
            version_id,
            local_point: id.to_string(),
        })
    }

    /// Download exact versions into private scratch space, then use the local
    /// validator and atomic no-replace installer for the final destination.
    pub async fn restore(
        &self,
        id: &str,
        version_id: &str,
        target: &Path,
    ) -> Result<(), BackupError> {
        if !hex::is_hex32(id) || target.try_exists()? {
            return Err(BackupError::Invalid(
                "invalid point or destination already exists",
            ));
        }
        validate_version(version_id)?;
        let parent = target
            .parent()
            .ok_or(BackupError::Invalid("destination has no parent"))?;
        let scratch = tempfile::tempdir_in(parent)?;
        private_dir(&scratch.path().join("objects"))?;
        private_dir(&scratch.path().join("points"))?;
        let (bytes, _) = self
            .get_required(
                "points",
                id,
                Some(version_id),
                limits::MAX_BACKUP_MANIFEST_BYTES,
            )
            .await?;
        let remote: RemoteManifest = serde_json::from_slice(&bytes)?;
        if remote.format != 1
            || !hex::is_hex32(&remote.local_point)
            || remote.objects.len() > limits::MAX_BACKUP_OBJECTS as usize + 1
        {
            return Err(BackupError::Invalid("invalid remote manifest"));
        }
        for (hash, version) in &remote.objects {
            if !hex::is_hex32(hash) {
                return Err(BackupError::Invalid("invalid remote object hash"));
            }
            validate_version(version)?;
        }
        let version = remote
            .objects
            .get(&remote.local_point)
            .ok_or(BackupError::Invalid("missing local manifest version"))?;
        let (bytes, _) = self
            .get_required(
                "objects",
                &remote.local_point,
                Some(version),
                limits::MAX_BACKUP_MANIFEST_BYTES,
            )
            .await?;
        let manifest: Manifest = serde_json::from_slice(&bytes)?;
        validate_manifest(&manifest)?;
        let mut required = BTreeSet::from([remote.local_point.clone()]);
        for entry in &manifest.entries {
            validate_entry(entry)?;
            required.insert(entry.sha256.clone());
        }
        if required != remote.objects.keys().cloned().collect() {
            return Err(BackupError::Invalid(
                "remote dependency inventory differs from local manifest",
            ));
        }
        persist(
            &scratch.path().join("points").join(&remote.local_point),
            &bytes,
        )?;
        for entry in &manifest.entries {
            let path = scratch.path().join("objects").join(&entry.sha256);
            if path.try_exists()? {
                continue;
            }
            let (bytes, _) = self
                .get_required(
                    "objects",
                    &entry.sha256,
                    Some(&remote.objects[&entry.sha256]),
                    entry.size,
                )
                .await?;
            if bytes.len() as u64 != entry.size {
                return Err(BackupError::Invalid("remote object size mismatch"));
            }
            persist(&path, &bytes)?;
        }
        super::restore(scratch.path(), &remote.local_point, target)
    }

    async fn require_safe_bucket(&self) -> Result<(), BackupError> {
        let result = bounded(async {
            self.client
                .get_bucket_versioning()
                .bucket(&self.bucket)
                .send()
                .await
                .map_err(|_| BackupError::Remote("cannot verify bucket versioning"))
        })
        .await?;
        if result.status() != Some(&BucketVersioningStatus::Enabled) {
            return Err(BackupError::Remote("bucket versioning must be Enabled"));
        }
        bounded(async {
            match self
                .client
                .get_bucket_lifecycle_configuration()
                .bucket(&self.bucket)
                .send()
                .await
            {
                Ok(config) => {
                    // Require a bucket without active expiration, conservatively
                    // including unrelated prefixes. Do not implement a lifecycle
                    // filter evaluator or mistake object age for reachability.
                    if config.rules().iter().any(|rule| {
                        rule.status() != &ExpirationStatus::Disabled
                            && (rule.expiration().is_some()
                                || rule.noncurrent_version_expiration().is_some())
                    }) {
                        return Err(BackupError::Remote(
                            "bucket lifecycle expiration can delete retained dependencies",
                        ));
                    }
                    Ok(())
                }
                Err(error)
                    if error.as_service_error().and_then(|e| e.code())
                        == Some("NoSuchLifecycleConfiguration") =>
                {
                    Ok(())
                }
                Err(_) => Err(BackupError::Remote("cannot verify bucket lifecycle")),
            }
        })
        .await?;
        Ok(())
    }

    async fn put_verified(
        &self,
        kind: &str,
        hash: &str,
        bytes: Vec<u8>,
    ) -> Result<String, BackupError> {
        let size = bytes.len() as u64;
        if let Some((existing, version)) = self.get(kind, hash, None, size).await? {
            if existing.len() as u64 != size {
                return Err(BackupError::Invalid("remote object size mismatch"));
            }
            return Ok(version);
        }
        let checksum =
            base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(&bytes));
        let result = bounded(async {
            let result = self
                .client
                .put_object()
                .bucket(&self.bucket)
                .key(self.key(kind, hash))
                .if_none_match("*")
                .checksum_sha256(checksum)
                .body(ByteStream::from(bytes))
                .send()
                .await;
            match result {
                Ok(output) => {
                    validate_encryption(
                        output.server_side_encryption().map(|value| value.as_str()),
                    )?;
                    let version = output
                        .version_id()
                        .ok_or(BackupError::Remote("PUT returned no version ID"))?;
                    validate_version(version)?;
                    Ok(Some(version.to_string()))
                }
                Err(error)
                    if error
                        .raw_response()
                        .is_some_and(|r| r.status().as_u16() == 412) =>
                {
                    Ok(None)
                }
                Err(_) => Err(BackupError::Remote(
                    "conditional upload failed; retry capture",
                )),
            }
        })
        .await?;
        let (verified, version) = self
            .get_required(kind, hash, result.as_deref(), size)
            .await?;
        if verified.len() as u64 != size {
            return Err(BackupError::Invalid("remote object size mismatch"));
        }
        Ok(version)
    }

    async fn get_required(
        &self,
        kind: &str,
        hash: &str,
        version: Option<&str>,
        limit: u64,
    ) -> Result<(Vec<u8>, String), BackupError> {
        self.get(kind, hash, version, limit)
            .await?
            .ok_or(BackupError::Remote("required object is missing"))
    }

    async fn get(
        &self,
        kind: &str,
        hash: &str,
        version: Option<&str>,
        limit: u64,
    ) -> Result<Option<(Vec<u8>, String)>, BackupError> {
        bounded(async {
            let result = self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(self.key(kind, hash))
                .set_version_id(version.map(str::to_string))
                .send()
                .await;
            let mut output = match result {
                Ok(output) => output,
                Err(error) if error.as_service_error().is_some_and(|e| e.is_no_such_key()) => {
                    return Ok(None);
                }
                Err(_) => return Err(BackupError::Remote("object download failed")),
            };
            let actual = output
                .version_id()
                .ok_or(BackupError::Remote("GET returned no version ID"))?
                .to_string();
            validate_encryption(output.server_side_encryption().map(|value| value.as_str()))?;
            validate_version(&actual)?;
            if version.is_some_and(|expected| expected != actual) {
                return Err(BackupError::Remote("GET returned a different version"));
            }
            if !output
                .content_length()
                .is_some_and(|n| n >= 0 && n as u64 <= limit)
            {
                return Err(BackupError::Invalid("remote object exceeds size limit"));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = output
                .body
                .try_next()
                .await
                .map_err(|_| BackupError::Remote("object body interrupted"))?
            {
                if bytes.len() as u64 + chunk.len() as u64 > limit {
                    return Err(BackupError::Invalid("remote object exceeds size limit"));
                }
                bytes.extend_from_slice(&chunk);
            }
            if digest(&bytes) != hash {
                return Err(BackupError::Invalid("remote object checksum mismatch"));
            }
            Ok(Some((bytes, actual)))
        })
        .await
    }

    fn key(&self, kind: &str, hash: &str) -> String {
        format!("{}/{kind}/{hash}", self.prefix)
    }
}

fn validate_version(version: &str) -> Result<(), BackupError> {
    // S3 version IDs are opaque, UTF-8 strings up to 1024 bytes. The special
    // null version can be overwritten when bucket versioning is suspended.
    if version.is_empty()
        || version == "null"
        || version.len() > 1024
        || version.chars().any(char::is_control)
    {
        return Err(BackupError::Remote("a non-null S3 version ID is required"));
    }
    Ok(())
}

fn validate_encryption(encryption: Option<&str>) -> Result<(), BackupError> {
    // Honor the bucket's default (including KMS) instead of overriding it with
    // SSE-S3. This checks server-side storage only, not client-side encryption.
    if !matches!(encryption, Some("AES256" | "aws:kms" | "aws:kms:dsse")) {
        return Err(BackupError::Remote(
            "S3 server-side encryption is missing or unsupported",
        ));
    }
    Ok(())
}

async fn bounded<T>(
    operation: impl std::future::Future<Output = Result<T, BackupError>>,
) -> Result<T, BackupError> {
    tokio::time::timeout(
        Duration::from_secs(limits::MAX_BACKUP_CAPTURE_SECONDS),
        operation,
    )
    .await
    .map_err(|_| BackupError::Remote("S3 operation timed out"))?
}

pub(crate) fn command(action: &str, args: &[String]) -> Result<(), String> {
    let required: &[&str] = match action {
        "capture-s3" => &["repository", "point", "bucket", "prefix", "region"],
        "restore-s3" => &[
            "point",
            "version-id",
            "target",
            "bucket",
            "prefix",
            "region",
        ],
        _ => return Err("unknown S3 recovery action".into()),
    };
    let (flags, positionals) = crate::parse_flags(args)?;
    let names: BTreeSet<_> = flags.iter().map(|(name, _)| name.as_str()).collect();
    if !positionals.is_empty()
        || names.len() != flags.len()
        || required.iter().any(|name| !names.contains(name))
        || names
            .iter()
            .any(|name| !required.contains(name) && *name != "endpoint-url")
    {
        return Err("missing, unexpected, or duplicate S3 backup arguments".into());
    }
    let value = |name| crate::flag_value(&flags, name).expect("required flags were checked");
    if !hex::is_hex32(value("point")) {
        return Err("invalid recovery point ID".into());
    }
    if action == "restore-s3" {
        validate_version(value("version-id")).map_err(|e| e.to_string())?;
        if Path::new(value("target"))
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            return Err("destination already exists".into());
        }
    }
    let endpoint = crate::flag_value(&flags, "endpoint-url");
    if let Some(endpoint) = endpoint {
        let uri: hyper::Uri = endpoint.parse().map_err(|_| "invalid S3 endpoint")?;
        let loopback = matches!(uri.host(), Some("127.0.0.1" | "localhost" | "[::1]"));
        if !(uri.scheme_str() == Some("https") || (uri.scheme_str() == Some("http") && loopback))
            || uri.authority().is_none_or(|a| a.as_str().contains('@'))
            || uri.query().is_some()
            || !matches!(uri.path(), "" | "/")
        {
            return Err("S3 endpoint requires HTTPS (HTTP allowed only on loopback)".into());
        }
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    if action == "capture-s3" {
        eprintln!(
            "S3 capture is not production-qualified: registry, private content and cookie keys are uploaded without client-side encryption."
        );
    }
    runtime.block_on(async {
        // Only an explicit S3 command enters the AWS credential provider chain.
        let http = aws_smithy_http_client::Builder::new()
            .tls_provider(aws_smithy_http_client::tls::Provider::Rustls(
                aws_smithy_http_client::tls::rustls_provider::CryptoMode::Ring,
            ))
            .build_https();
        let config = aws_config::defaults(aws_config::BehaviorVersion::v2026_01_12())
            .http_client(http)
            .region(aws_sdk_s3::config::Region::new(value("region").to_string()))
            .retry_config(aws_sdk_s3::config::retry::RetryConfig::standard().with_max_attempts(3))
            .timeout_config(
                aws_sdk_s3::config::timeout::TimeoutConfig::builder()
                    .operation_timeout(Duration::from_secs(limits::MAX_BACKUP_CAPTURE_SECONDS))
                    .build(),
            )
            .load()
            .await;
        let mut builder = aws_sdk_s3::config::Builder::from(&config);
        // Only the validated explicit flag may override the AWS S3 endpoint.
        // Inherited endpoint environment/profile settings must not bypass TLS.
        builder.set_endpoint_url(endpoint.map(str::to_string));
        if let Some(endpoint) = endpoint {
            builder = builder.endpoint_url(endpoint).force_path_style(true);
        }
        let remote = Repository::new(
            Client::from_conf(builder.build()),
            value("bucket").to_string(),
            value("prefix").to_string(),
        )
        .map_err(|e| e.to_string())?;
        if action == "capture-s3" {
            let receipt = remote
                .capture(Path::new(value("repository")), value("point"))
                .await
                .map_err(|e| e.to_string())?;
            println!(
                "{}",
                serde_json::to_string(&receipt).map_err(|e| e.to_string())?
            );
        } else {
            remote
                .restore(
                    value("point"),
                    value("version-id"),
                    Path::new(value("target")),
                )
                .await
                .map_err(|e| e.to_string())?;
            println!("{}", serde_json::json!({"restored": true}));
        }
        Ok(())
    })
}
