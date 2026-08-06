//! Release-material packing, signing, and verification for Finite bundles.
//!
//! This crate is the one Rust home for the `finite_skills_bundle.v1` manifest
//! schema and its tree-digest discipline. The digest reproduces, byte for
//! byte, the sha256 computed by `_validate()` in
//! `finitechat/containers/agent/finite.py`: the same traversal order (a global
//! lexicographic sort of relative POSIX path strings), the same per-file
//! framing, and the same rejection of symlinks and non-regular files.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub const SKILLS_BUNDLE_SCHEMA: &str = "finite_skills_bundle.v1";
const TREE_DIGEST_DOMAIN: &[u8] = b"finite-skills-tree-v1\0";
const SIGNATURE_PREFIX: &str = "ed25519:";
const READ_CHUNK_BYTES: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum ReleaseError {
    #[error("I/O failure: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON failure: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid skills tree: {0}")]
    InvalidTree(String),
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("invalid key material: {0}")]
    InvalidKey(String),
    #[error("bundle verification failed: {0}")]
    VerificationFailed(String),
    #[error("URL refused by policy: {0}")]
    UrlPolicy(String),
}

/// Dev-harness-only escape hatch admitting plain-http bundle/directory URLs.
pub const ALLOW_INSECURE_BUNDLE_URL_ENV: &str = "FINITE_ALLOW_INSECURE_BUNDLE_URL";

/// The one https-or-dev-escape URL policy for fetched release material
/// (bundles and service directories). `allow_insecure` is the caller's
/// [`ALLOW_INSECURE_BUNDLE_URL_ENV`] setting.
pub fn check_bundle_url_policy(url: &str, allow_insecure: bool) -> Result<(), ReleaseError> {
    if url.starts_with("https://") || (allow_insecure && url.starts_with("http://")) {
        return Ok(());
    }
    Err(ReleaseError::UrlPolicy(format!(
        "bundle URLs must use https ({ALLOW_INSECURE_BUNDLE_URL_ENV}=1 admits http in the dev harness only): {url:?}"
    )))
}

/// Signed release manifest for one managed skills bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillsBundleManifestV1 {
    pub schema: String,
    pub artifact_id: String,
    pub version_label: String,
    pub source_git_sha: Option<String>,
    pub tree_digest: String,
    pub tarball_sha256: String,
    pub created_at: String,
    /// `ed25519:<base64>` over [`canonical_signing_bytes`].
    pub signature: String,
}

impl SkillsBundleManifestV1 {
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ReleaseError> {
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate_structure()?;
        Ok(manifest)
    }

    /// Structural checks that do not need key material or bundle bytes.
    pub fn validate_structure(&self) -> Result<(), ReleaseError> {
        if self.schema != SKILLS_BUNDLE_SCHEMA {
            return Err(ReleaseError::InvalidManifest(format!(
                "unsupported schema {:?}",
                self.schema
            )));
        }
        validate_artifact_id(&self.artifact_id)?;
        if self.version_label.is_empty() {
            return Err(ReleaseError::InvalidManifest(
                "version_label must not be empty".to_owned(),
            ));
        }
        for (field, value) in [
            ("tree_digest", &self.tree_digest),
            ("tarball_sha256", &self.tarball_sha256),
        ] {
            if !is_lower_hex_256(value) {
                return Err(ReleaseError::InvalidManifest(format!(
                    "{field} must be 64 lowercase hex characters"
                )));
            }
        }
        if !self.signature.starts_with(SIGNATURE_PREFIX) {
            return Err(ReleaseError::InvalidManifest(format!(
                "signature must start with {SIGNATURE_PREFIX:?}"
            )));
        }
        Ok(())
    }
}

/// A packed and signed bundle on disk.
#[derive(Debug)]
pub struct PackOutput {
    pub manifest: SkillsBundleManifestV1,
    pub tarball_path: PathBuf,
    pub manifest_path: PathBuf,
}

/// Inputs for [`pack_skills_bundle`].
#[derive(Debug)]
pub struct PackRequest<'a> {
    pub source: &'a Path,
    pub artifact_id: &'a str,
    pub version_label: &'a str,
    pub source_git_sha: Option<&'a str>,
    /// RFC 3339 creation instant; the current UTC time when `None`.
    pub created_at: Option<&'a str>,
    pub out_dir: &'a Path,
}

/// The manifest a bundle was verified against.
#[derive(Debug)]
pub struct VerifiedSkillsBundle {
    pub manifest: SkillsBundleManifestV1,
}

pub fn validate_artifact_id(artifact_id: &str) -> Result<(), ReleaseError> {
    let valid = !artifact_id.is_empty()
        && artifact_id.len() <= 128
        && artifact_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && artifact_id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(ReleaseError::InvalidManifest(format!(
            "artifact_id {artifact_id:?} must be a plain [A-Za-z0-9._-] name"
        )))
    }
}

pub fn is_lower_hex_256(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn sha256_hex_of_file(path: &Path) -> Result<String, ReleaseError> {
    let mut digest = Sha256::new();
    let mut file = File::open(path)?;
    let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

/// Canonical JSON bytes signed by the release key: the manifest object with
/// the `signature` field removed, keys sorted, compact separators, UTF-8.
pub fn canonical_signing_bytes(manifest: &SkillsBundleManifestV1) -> Result<Vec<u8>, ReleaseError> {
    let mut fields: BTreeMap<&'static str, Value> = BTreeMap::new();
    fields.insert("schema", Value::from(manifest.schema.as_str()));
    fields.insert("artifact_id", Value::from(manifest.artifact_id.as_str()));
    fields.insert(
        "version_label",
        Value::from(manifest.version_label.as_str()),
    );
    fields.insert(
        "source_git_sha",
        match &manifest.source_git_sha {
            Some(sha) => Value::from(sha.as_str()),
            None => Value::Null,
        },
    );
    fields.insert("tree_digest", Value::from(manifest.tree_digest.as_str()));
    fields.insert(
        "tarball_sha256",
        Value::from(manifest.tarball_sha256.as_str()),
    );
    fields.insert("created_at", Value::from(manifest.created_at.as_str()));
    Ok(serde_json::to_vec(&fields)?)
}

pub fn generate_signing_key() -> Result<SigningKey, ReleaseError> {
    let mut seed = [0_u8; 32];
    getrandom::fill(&mut seed)
        .map_err(|error| ReleaseError::InvalidKey(format!("cannot gather entropy: {error}")))?;
    Ok(SigningKey::from_bytes(&seed))
}

pub fn parse_signing_key_hex(value: &str) -> Result<SigningKey, ReleaseError> {
    let bytes: [u8; 32] = hex::decode(value.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| {
            ReleaseError::InvalidKey("signing key must be 32 bytes of hex".to_owned())
        })?;
    Ok(SigningKey::from_bytes(&bytes))
}

pub fn parse_verifying_key_hex(value: &str) -> Result<VerifyingKey, ReleaseError> {
    let bytes: [u8; 32] = hex::decode(value.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| ReleaseError::InvalidKey("public key must be 32 bytes of hex".to_owned()))?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|error| ReleaseError::InvalidKey(format!("invalid ed25519 public key: {error}")))
}

/// `ed25519:<base64>` signature over `message` — the one signature encoding
/// for all Finite release material.
pub fn signature_string(message: &[u8], signing_key: &SigningKey) -> String {
    let signature = signing_key.sign(message);
    format!(
        "{SIGNATURE_PREFIX}{}",
        BASE64_STANDARD.encode(signature.to_bytes())
    )
}

/// Verify an `ed25519:<base64>` signature string over `message`.
pub fn verify_signature_string(
    message: &[u8],
    signature: &str,
    public_key: &VerifyingKey,
) -> Result<(), ReleaseError> {
    let encoded = signature.strip_prefix(SIGNATURE_PREFIX).ok_or_else(|| {
        ReleaseError::InvalidManifest(format!("signature must start with {SIGNATURE_PREFIX:?}"))
    })?;
    let bytes: [u8; 64] = BASE64_STANDARD
        .decode(encoded)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| {
            ReleaseError::InvalidManifest("signature must be 64 bytes of base64".to_owned())
        })?;
    public_key
        .verify_strict(message, &Signature::from_bytes(&bytes))
        .map_err(|_| {
            ReleaseError::VerificationFailed(
                "signature does not verify against the release public key".to_owned(),
            )
        })
}

/// Canonical JSON: compact separators, object keys recursively byte-sorted.
/// This reproduces `serde_json::to_vec` over `BTreeMap`-backed objects (the
/// manifest canonicalization above) without depending on `serde_json`'s map
/// ordering feature flags.
pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, ReleaseError> {
    let mut out = Vec::new();
    write_canonical_json(value, &mut out)?;
    Ok(out)
}

fn write_canonical_json(value: &Value, out: &mut Vec<u8>) -> Result<(), ReleaseError> {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            out.push(b'{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(&serde_json::to_vec(key)?);
                out.push(b':');
                write_canonical_json(&map[key.as_str()], out)?;
            }
            out.push(b'}');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_canonical_json(item, out)?;
            }
            out.push(b']');
        }
        other => out.extend_from_slice(&serde_json::to_vec(other)?),
    }
    Ok(())
}

/// The canonical bytes a signed JSON document's `signature` field covers: the
/// top-level object with `signature` removed, in [`canonical_json_bytes`]
/// form. This is the one canonicalization shared by every signed Finite JSON
/// document (e.g. the Core service directory).
pub fn document_signing_bytes(document: &Value) -> Result<Vec<u8>, ReleaseError> {
    let Value::Object(map) = document else {
        return Err(ReleaseError::InvalidManifest(
            "signed documents must be JSON objects".to_owned(),
        ));
    };
    let mut unsigned = map.clone();
    unsigned.remove("signature");
    canonical_json_bytes(&Value::Object(unsigned))
}

/// Sign a JSON document's canonical bytes (any existing `signature` field is
/// ignored). Returns the `ed25519:<base64>` value to store in `signature`.
pub fn sign_document(document: &Value, signing_key: &SigningKey) -> Result<String, ReleaseError> {
    Ok(signature_string(
        &document_signing_bytes(document)?,
        signing_key,
    ))
}

/// Verify a JSON document's `signature` field against its canonical bytes.
pub fn verify_document_signature(
    document: &Value,
    public_key: &VerifyingKey,
) -> Result<(), ReleaseError> {
    let signature = document
        .get("signature")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ReleaseError::InvalidManifest("document has no signature field".to_owned())
        })?;
    verify_signature_string(&document_signing_bytes(document)?, signature, public_key)
}

/// Sign the manifest's canonical bytes; the current `signature` field value
/// is ignored.
pub fn signature_for_manifest(
    manifest: &SkillsBundleManifestV1,
    signing_key: &SigningKey,
) -> Result<String, ReleaseError> {
    Ok(signature_string(
        &canonical_signing_bytes(manifest)?,
        signing_key,
    ))
}

pub fn verify_manifest_signature(
    manifest: &SkillsBundleManifestV1,
    public_key: &VerifyingKey,
) -> Result<(), ReleaseError> {
    verify_signature_string(
        &canonical_signing_bytes(manifest)?,
        &manifest.signature,
        public_key,
    )
}

/// Every regular file under `root` as `(relative POSIX path, absolute path)`,
/// in the exact order `finite.py` hashes them: a global lexicographic sort of
/// the relative path strings. Symlinks and non-regular files anywhere in the
/// tree are rejected, exactly as `finite.py` rejects them.
fn regular_tree_files(root: &Path) -> Result<Vec<(String, PathBuf)>, ReleaseError> {
    let metadata = fs::symlink_metadata(root).map_err(|_| unavailable_tree(root))?;
    if !metadata.is_dir() {
        return Err(unavailable_tree(root));
    }

    let mut files = Vec::new();
    collect_regular_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    Ok(files)
}

fn unavailable_tree(root: &Path) -> ReleaseError {
    ReleaseError::InvalidTree(format!(
        "bundled Finite Skills directory is unavailable: {}",
        root.display()
    ))
}

fn collect_regular_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), ReleaseError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(ReleaseError::InvalidTree(format!(
                "bundled Finite Skills must not contain symlinks: {}",
                path.display()
            )));
        }
        if file_type.is_dir() {
            collect_regular_files(root, &path, files)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(ReleaseError::InvalidTree(format!(
                "bundled Finite Skills contains a non-file entry: {}",
                path.display()
            )));
        }
        let relative = path.strip_prefix(root).map_err(|_| {
            ReleaseError::InvalidTree(format!("path escapes the skills tree: {}", path.display()))
        })?;
        let mut posix = String::new();
        for component in relative.components() {
            let Component::Normal(part) = component else {
                return Err(ReleaseError::InvalidTree(format!(
                    "unsupported path component in {}",
                    relative.display()
                )));
            };
            let part = part.to_str().ok_or_else(|| {
                ReleaseError::InvalidTree(format!(
                    "file name is not valid UTF-8: {}",
                    path.display()
                ))
            })?;
            if !posix.is_empty() {
                posix.push('/');
            }
            posix.push_str(part);
        }
        files.push((posix, path));
    }
    Ok(())
}

/// The `finite.py` `_validate()` tree digest over every regular file under
/// `root`.
pub fn compute_tree_digest(root: &Path) -> Result<String, ReleaseError> {
    let files = regular_tree_files(root)?;
    let mut digest = Sha256::new();
    digest.update(TREE_DIGEST_DOMAIN);
    let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
    for (relative, path) in &files {
        let relative_bytes = relative.as_bytes();
        digest.update((relative_bytes.len() as u64).to_be_bytes());
        digest.update(relative_bytes);
        let mut content_digest = Sha256::new();
        let mut content_size = 0_u64;
        let mut file = File::open(path)?;
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            content_size += read as u64;
            content_digest.update(&buffer[..read]);
        }
        digest.update(content_size.to_be_bytes());
        digest.update(content_digest.finalize());
    }
    Ok(hex::encode(digest.finalize()))
}

/// Pack `source` into `<artifact-id>.tar.gz` plus a signed
/// `<artifact-id>.tar.gz.manifest.json` under `out_dir` (the manifest URL for
/// a hosted bundle is derived by appending `.manifest.json` to its tarball
/// URL).
///
/// The tarball is deterministic: entries sorted by relative POSIX path,
/// zeroed mtimes/uids/gids, normalized 0644/0755 modes, root-relative paths.
pub fn pack_skills_bundle(
    request: &PackRequest<'_>,
    signing_key: &SigningKey,
) -> Result<PackOutput, ReleaseError> {
    validate_artifact_id(request.artifact_id)?;
    if request.version_label.is_empty() {
        return Err(ReleaseError::InvalidManifest(
            "version_label must not be empty".to_owned(),
        ));
    }
    let created_at = match request.created_at {
        Some(value) => {
            OffsetDateTime::parse(value, &Rfc3339).map_err(|error| {
                ReleaseError::InvalidManifest(format!("created_at is not RFC 3339: {error}"))
            })?;
            value.to_owned()
        }
        None => OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|error| {
                ReleaseError::InvalidManifest(format!("cannot format the current time: {error}"))
            })?,
    };

    let files = regular_tree_files(request.source)?;
    if files.is_empty() {
        return Err(ReleaseError::InvalidTree(format!(
            "bundled Finite Skills contains no files: {}",
            request.source.display()
        )));
    }
    let tree_digest = compute_tree_digest(request.source)?;

    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (relative, path) in &files {
        let metadata = fs::symlink_metadata(path)?;
        let mode = if metadata.permissions().mode() & 0o100 != 0 {
            0o755
        } else {
            0o644
        };
        let mut header = tar::Header::new_gnu();
        header.set_size(metadata.len());
        header.set_mode(mode);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        builder.append_data(&mut header, relative, File::open(path)?)?;
    }
    let tarball_bytes = builder.into_inner()?.finish()?;
    let tarball_sha256 = sha256_hex(&tarball_bytes);

    let mut manifest = SkillsBundleManifestV1 {
        schema: SKILLS_BUNDLE_SCHEMA.to_owned(),
        artifact_id: request.artifact_id.to_owned(),
        version_label: request.version_label.to_owned(),
        source_git_sha: request.source_git_sha.map(str::to_owned),
        tree_digest,
        tarball_sha256,
        created_at,
        signature: String::new(),
    };
    manifest.signature = signature_for_manifest(&manifest, signing_key)?;
    manifest.validate_structure()?;

    fs::create_dir_all(request.out_dir)?;
    let tarball_path = request
        .out_dir
        .join(format!("{}.tar.gz", request.artifact_id));
    let manifest_path = request
        .out_dir
        .join(format!("{}.tar.gz.manifest.json", request.artifact_id));
    fs::write(&tarball_path, &tarball_bytes)?;
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    manifest_bytes.push(b'\n');
    fs::write(&manifest_path, manifest_bytes)?;

    Ok(PackOutput {
        manifest,
        tarball_path,
        manifest_path,
    })
}

/// Unpack a skills tarball into `destination`, admitting only regular files
/// and directories with plain relative paths.
pub fn unpack_tarball(tarball: &Path, destination: &Path) -> Result<(), ReleaseError> {
    fs::create_dir_all(destination)?;
    let decoder = flate2::read::GzDecoder::new(File::open(tarball)?);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_type = entry.header().entry_type();
        let path = entry.path()?.into_owned();
        if path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(ReleaseError::VerificationFailed(format!(
                "tarball entry has an unsafe path: {}",
                path.display()
            )));
        }
        let target = destination.join(&path);
        match entry_type {
            tar::EntryType::Directory => fs::create_dir_all(&target)?,
            tar::EntryType::Regular => {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mode = entry.header().mode().unwrap_or(0o644) & 0o777;
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(mode)
                    .open(&target)?;
                std::io::copy(&mut entry, &mut file)?;
            }
            other => {
                return Err(ReleaseError::VerificationFailed(format!(
                    "tarball entry {} has unsupported type {other:?}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

/// Verify a bundle end to end: tarball digest against the manifest (and an
/// optional caller expectation), manifest signature against the release
/// public key, then the recomputed tree digest of the bytes unpacked into
/// `unpack_dir`.
pub fn verify_skills_bundle(
    tarball: &Path,
    manifest_path: &Path,
    public_key: &VerifyingKey,
    expected_tarball_sha256: Option<&str>,
    unpack_dir: &Path,
) -> Result<VerifiedSkillsBundle, ReleaseError> {
    let manifest = SkillsBundleManifestV1::from_json_bytes(&fs::read(manifest_path)?)?;
    let actual_tarball_sha256 = sha256_hex_of_file(tarball)?;
    if actual_tarball_sha256 != manifest.tarball_sha256 {
        return Err(ReleaseError::VerificationFailed(format!(
            "tarball sha256 {actual_tarball_sha256} does not match the manifest's {}",
            manifest.tarball_sha256
        )));
    }
    if let Some(expected) = expected_tarball_sha256 {
        let expected = expected.trim().to_ascii_lowercase();
        if actual_tarball_sha256 != expected {
            return Err(ReleaseError::VerificationFailed(format!(
                "tarball sha256 {actual_tarball_sha256} does not match the expected {expected}"
            )));
        }
    }
    verify_manifest_signature(&manifest, public_key)?;
    unpack_tarball(tarball, unpack_dir)?;
    let tree_digest = compute_tree_digest(unpack_dir)?;
    if tree_digest != manifest.tree_digest {
        return Err(ReleaseError::VerificationFailed(format!(
            "unpacked tree digest {tree_digest} does not match the manifest's {}",
            manifest.tree_digest
        )));
    }
    Ok(VerifiedSkillsBundle { manifest })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden digest of `tests/fixtures/skills-tree`, shared with the Python
    /// parity test `finitechat/tests/container/test_finite_skills_digest.py`.
    /// If a digest change is intentional, update both constants together.
    const FIXTURE_TREE_DIGEST: &str =
        "bc07f6443ed59aff6738c3a132ddfc5c70eb05ab231599faf725779214d06413";

    fn fixture_tree() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skills-tree")
    }

    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7_u8; 32])
    }

    fn pack_fixture(out_dir: &Path) -> PackOutput {
        pack_skills_bundle(
            &PackRequest {
                source: &fixture_tree(),
                artifact_id: "skills-test",
                version_label: "2026.08.06-test",
                source_git_sha: Some("eee4713f00000000000000000000000000000000"),
                created_at: Some("2026-08-06T00:00:00Z"),
                out_dir,
            },
            &test_signing_key(),
        )
        .unwrap()
    }

    #[test]
    fn fixture_tree_digest_matches_the_shared_golden_constant() {
        assert_eq!(
            compute_tree_digest(&fixture_tree()).unwrap(),
            FIXTURE_TREE_DIGEST
        );
    }

    #[test]
    fn tree_digest_orders_paths_as_full_strings_like_finite_py() {
        // "note-extra/SKILL.md" must hash before "note.md" ('-' < '.'), which
        // a per-directory component sort would get wrong.
        let files = regular_tree_files(&fixture_tree()).unwrap();
        let relative: Vec<&str> = files.iter().map(|(rel, _)| rel.as_str()).collect();
        assert_eq!(
            relative,
            [
                "productivity/note-extra/SKILL.md",
                "productivity/note.md",
                "software-development/finite-sites-publishing-finite/SKILL.md",
                "software-development/finitebrain/SKILL.md",
                "software-development/finitebrain/references/guide.md",
            ]
        );
    }

    #[test]
    fn tree_digest_rejects_symlinks_like_finite_py() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("skill")).unwrap();
        fs::write(dir.path().join("skill/SKILL.md"), "---\n").unwrap();
        std::os::unix::fs::symlink("SKILL.md", dir.path().join("skill/link.md")).unwrap();
        let error = compute_tree_digest(dir.path()).unwrap_err();
        assert!(matches!(error, ReleaseError::InvalidTree(_)));
    }

    #[test]
    fn packing_is_deterministic_and_roundtrips_through_verification() {
        let out_one = tempfile::tempdir().unwrap();
        let out_two = tempfile::tempdir().unwrap();
        let packed_one = pack_fixture(out_one.path());
        let packed_two = pack_fixture(out_two.path());
        assert_eq!(
            packed_one.manifest.tarball_sha256, packed_two.manifest.tarball_sha256,
            "packing the same tree twice must produce identical tarball bytes"
        );
        assert_eq!(packed_one.manifest.tree_digest, FIXTURE_TREE_DIGEST);

        let unpack = tempfile::tempdir().unwrap();
        let verified = verify_skills_bundle(
            &packed_one.tarball_path,
            &packed_one.manifest_path,
            &test_signing_key().verifying_key(),
            Some(&packed_one.manifest.tarball_sha256),
            &unpack.path().join("tree"),
        )
        .unwrap();
        assert_eq!(verified.manifest, packed_one.manifest);
        assert_eq!(
            compute_tree_digest(&unpack.path().join("tree")).unwrap(),
            FIXTURE_TREE_DIGEST
        );
    }

    #[test]
    fn a_flipped_tarball_byte_fails_verification() {
        let out = tempfile::tempdir().unwrap();
        let packed = pack_fixture(out.path());
        let mut bytes = fs::read(&packed.tarball_path).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x01;
        fs::write(&packed.tarball_path, bytes).unwrap();

        let unpack = tempfile::tempdir().unwrap();
        let error = verify_skills_bundle(
            &packed.tarball_path,
            &packed.manifest_path,
            &test_signing_key().verifying_key(),
            Some(&packed.manifest.tarball_sha256),
            &unpack.path().join("tree"),
        )
        .unwrap_err();
        assert!(matches!(error, ReleaseError::VerificationFailed(_)));
    }

    #[test]
    fn a_mutated_manifest_field_fails_the_signature() {
        let out = tempfile::tempdir().unwrap();
        let packed = pack_fixture(out.path());
        let mut manifest = packed.manifest.clone();
        manifest.version_label = "2026.08.07-forged".to_owned();
        let error =
            verify_manifest_signature(&manifest, &test_signing_key().verifying_key()).unwrap_err();
        assert!(matches!(error, ReleaseError::VerificationFailed(_)));
        verify_manifest_signature(&packed.manifest, &test_signing_key().verifying_key()).unwrap();
    }

    #[test]
    fn canonical_signing_bytes_sort_keys_and_drop_the_signature() {
        let manifest = SkillsBundleManifestV1 {
            schema: SKILLS_BUNDLE_SCHEMA.to_owned(),
            artifact_id: "skills-test".to_owned(),
            version_label: "v1".to_owned(),
            source_git_sha: None,
            tree_digest: "1".repeat(64),
            tarball_sha256: "2".repeat(64),
            created_at: "2026-08-06T00:00:00Z".to_owned(),
            signature: "ed25519:ignored".to_owned(),
        };
        let bytes = canonical_signing_bytes(&manifest).unwrap();
        let expected = format!(
            "{{\"artifact_id\":\"skills-test\",\"created_at\":\"2026-08-06T00:00:00Z\",\
             \"schema\":\"finite_skills_bundle.v1\",\"source_git_sha\":null,\
             \"tarball_sha256\":\"{}\",\"tree_digest\":\"{}\",\"version_label\":\"v1\"}}",
            "2".repeat(64),
            "1".repeat(64)
        );
        assert_eq!(String::from_utf8(bytes).unwrap(), expected);
    }

    #[test]
    fn packed_bundle_manifest_uses_the_appended_filename_convention() {
        let out = tempfile::tempdir().unwrap();
        let packed = pack_fixture(out.path());
        assert_eq!(
            packed.manifest_path.file_name().unwrap().to_str().unwrap(),
            "skills-test.tar.gz.manifest.json",
            "the manifest URL is derived by appending .manifest.json to the tarball URL"
        );
        assert_eq!(
            packed.tarball_path.file_name().unwrap().to_str().unwrap(),
            "skills-test.tar.gz"
        );
    }

    #[test]
    fn canonical_document_signing_sorts_keys_recursively_and_drops_the_signature() {
        let document = serde_json::json!({
            "schema": "finite_service_directory.v1",
            "services": { "sites": { "base_url": "https://sites.test" } },
            "channels": { "canary": { "skills_bundle": { "artifact_id": "a-1" } } },
            "generated_at": "2026-08-06T00:00:00Z",
            "signature": "ed25519:ignored",
        });
        let bytes = document_signing_bytes(&document).unwrap();
        assert_eq!(
            String::from_utf8(bytes.clone()).unwrap(),
            "{\"channels\":{\"canary\":{\"skills_bundle\":{\"artifact_id\":\"a-1\"}}},\
             \"generated_at\":\"2026-08-06T00:00:00Z\",\"schema\":\"finite_service_directory.v1\",\
             \"services\":{\"sites\":{\"base_url\":\"https://sites.test\"}}}"
        );

        let signing_key = test_signing_key();
        let mut signed = document.clone();
        signed["signature"] = Value::from(sign_document(&document, &signing_key).unwrap());
        verify_document_signature(&signed, &signing_key.verifying_key()).unwrap();

        let mut tampered = signed.clone();
        tampered["services"]["sites"]["base_url"] = Value::from("https://forged.test");
        assert!(matches!(
            verify_document_signature(&tampered, &signing_key.verifying_key()).unwrap_err(),
            ReleaseError::VerificationFailed(_)
        ));
        let unsigned = serde_json::json!({ "schema": "finite_service_directory.v1" });
        assert!(matches!(
            verify_document_signature(&unsigned, &signing_key.verifying_key()).unwrap_err(),
            ReleaseError::InvalidManifest(_)
        ));
    }

    #[test]
    fn bundle_url_policy_admits_https_and_gates_http_behind_the_dev_escape() {
        check_bundle_url_policy("https://releases.finite.computer/x.tar.gz", false).unwrap();
        check_bundle_url_policy("http://127.0.0.1:18791/x.tar.gz", true).unwrap();
        for refused in [
            ("http://127.0.0.1:18791/x.tar.gz", false),
            ("file:///etc/passwd", true),
            ("ftp://release.invalid/x", true),
        ] {
            let error = check_bundle_url_policy(refused.0, refused.1).unwrap_err();
            assert!(matches!(error, ReleaseError::UrlPolicy(_)));
            assert!(error.to_string().contains("https"));
        }
    }

    #[test]
    fn unpacking_rejects_path_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let tarball = dir.path().join("escape.tar.gz");
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let payload = b"outside";
        let mut header = tar::Header::new_gnu();
        header.set_size(payload.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        // The tar writer itself refuses `..`, so forge the name bytes the way
        // a malicious bundle would.
        let name = b"../escaped.txt";
        header.as_mut_bytes()[..name.len()].copy_from_slice(name);
        header.set_cksum();
        builder.append(&header, payload.as_slice()).unwrap();
        fs::write(&tarball, builder.into_inner().unwrap().finish().unwrap()).unwrap();

        let error = unpack_tarball(&tarball, &dir.path().join("tree")).unwrap_err();
        assert!(matches!(error, ReleaseError::VerificationFailed(_)));
    }
}
