//! `finite_payload_bundle.v1`: packing, signing, and verification for runtime
//! payload generations (ADR 0006).
//!
//! The payload tree digest generalizes the skills tree digest with one
//! deliberate difference: payload trees MAY contain symlinks, because Python
//! virtualenvs need them. Only *relative* symlinks whose lexically resolved
//! target stays inside the tree are admitted; absolute or escaping symlinks
//! are rejected at digest, pack, and unpack time. The digest domain prefix
//! (`finite-payload-tree-v1\0`) and a per-entry kind byte keep payload
//! digests distinct from skills digests and keep a symlink from ever
//! colliding with a regular file holding the target string as content.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::{
    READ_CHUNK_BYTES, ReleaseError, SIGNATURE_PREFIX, is_lower_hex_256, sha256_hex,
    sha256_hex_of_file, signature_string, validate_artifact_id, verify_signature_string,
};

pub const PAYLOAD_BUNDLE_SCHEMA: &str = "finite_payload_bundle.v1";
const PAYLOAD_TREE_DIGEST_DOMAIN: &[u8] = b"finite-payload-tree-v1\0";
/// Per-entry kind bytes hashed after the relative path, so a symlink can
/// never collide with a regular file whose bytes equal the link target.
const PAYLOAD_ENTRY_KIND_FILE: u8 = b'F';
const PAYLOAD_ENTRY_KIND_SYMLINK: u8 = b'L';

/// A strict `MAJOR.MINOR.PATCH` version (no pre-release or build metadata),
/// used for `min_shell_version` gating. Ordering is numeric per component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ShellSemver {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl ShellSemver {
    pub fn parse(value: &str) -> Result<Self, ReleaseError> {
        let invalid = || {
            ReleaseError::InvalidManifest(format!(
                "min_shell_version must be a plain MAJOR.MINOR.PATCH semver string: {value:?}"
            ))
        };
        let mut parts = value.trim().split('.');
        let component = |part: Option<&str>| -> Result<u64, ReleaseError> {
            let part = part.ok_or_else(invalid)?;
            if part.is_empty()
                || part.len() > 10
                || !part.chars().all(|c| c.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
            {
                return Err(invalid());
            }
            part.parse().map_err(|_| invalid())
        };
        let parsed = Self {
            major: component(parts.next())?,
            minor: component(parts.next())?,
            patch: component(parts.next())?,
        };
        if parts.next().is_some() {
            return Err(invalid());
        }
        Ok(parsed)
    }
}

impl std::fmt::Display for ShellSemver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Signed release manifest for one runtime payload bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadBundleManifestV1 {
    pub schema: String,
    pub artifact_id: String,
    pub version_label: String,
    pub source_git_sha: Option<String>,
    /// Lowest `finite-shell` version (semver) that may stage this payload.
    pub min_shell_version: String,
    pub tree_digest: String,
    pub tarball_sha256: String,
    pub created_at: String,
    /// `ed25519:<base64>` over [`payload_canonical_signing_bytes`].
    pub signature: String,
}

impl PayloadBundleManifestV1 {
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ReleaseError> {
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate_structure()?;
        Ok(manifest)
    }

    /// Structural checks that do not need key material or bundle bytes.
    pub fn validate_structure(&self) -> Result<(), ReleaseError> {
        if self.schema != PAYLOAD_BUNDLE_SCHEMA {
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
        ShellSemver::parse(&self.min_shell_version)?;
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

    pub fn min_shell_version(&self) -> Result<ShellSemver, ReleaseError> {
        ShellSemver::parse(&self.min_shell_version)
    }
}

/// Canonical JSON bytes signed by the release key: the manifest object with
/// the `signature` field removed, keys sorted, compact separators, UTF-8.
pub fn payload_canonical_signing_bytes(
    manifest: &PayloadBundleManifestV1,
) -> Result<Vec<u8>, ReleaseError> {
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
    fields.insert(
        "min_shell_version",
        Value::from(manifest.min_shell_version.as_str()),
    );
    fields.insert("tree_digest", Value::from(manifest.tree_digest.as_str()));
    fields.insert(
        "tarball_sha256",
        Value::from(manifest.tarball_sha256.as_str()),
    );
    fields.insert("created_at", Value::from(manifest.created_at.as_str()));
    Ok(serde_json::to_vec(&fields)?)
}

pub fn signature_for_payload_manifest(
    manifest: &PayloadBundleManifestV1,
    signing_key: &SigningKey,
) -> Result<String, ReleaseError> {
    Ok(signature_string(
        &payload_canonical_signing_bytes(manifest)?,
        signing_key,
    ))
}

pub fn verify_payload_manifest_signature(
    manifest: &PayloadBundleManifestV1,
    public_key: &VerifyingKey,
) -> Result<(), ReleaseError> {
    verify_signature_string(
        &payload_canonical_signing_bytes(manifest)?,
        &manifest.signature,
        public_key,
    )
}

/// Inputs for [`pack_payload_bundle`].
#[derive(Debug)]
pub struct PayloadPackRequest<'a> {
    pub source: &'a Path,
    pub artifact_id: &'a str,
    pub version_label: &'a str,
    pub min_shell_version: &'a str,
    pub source_git_sha: Option<&'a str>,
    /// RFC 3339 creation instant; the current UTC time when `None`.
    pub created_at: Option<&'a str>,
    pub out_dir: &'a Path,
}

/// A packed and signed payload bundle on disk.
#[derive(Debug)]
pub struct PayloadPackOutput {
    pub manifest: PayloadBundleManifestV1,
    pub tarball_path: PathBuf,
    pub manifest_path: PathBuf,
}

/// The manifest a payload bundle was verified against.
#[derive(Debug)]
pub struct VerifiedPayloadBundle {
    pub manifest: PayloadBundleManifestV1,
}

#[derive(Debug)]
enum PayloadEntry {
    File(PathBuf),
    /// The symlink target exactly as stored on disk (relative, in-tree).
    Symlink(String),
}

/// Validate a symlink target lexically: it must be relative, UTF-8, and its
/// resolution from `link_relative` (the link's tree-relative POSIX path) must
/// stay inside the tree. Symlink chains stay in-tree by induction: every hop
/// is itself an in-tree relative link.
fn validate_symlink_target(link_relative: &str, target: &Path) -> Result<String, ReleaseError> {
    let target_str = target.to_str().ok_or_else(|| {
        ReleaseError::InvalidTree(format!(
            "payload symlink target is not valid UTF-8: {link_relative}"
        ))
    })?;
    if target.is_absolute() {
        return Err(ReleaseError::InvalidTree(format!(
            "payload symlink {link_relative} has an absolute target {target_str:?}"
        )));
    }
    // The link's directory, as normalized components.
    let mut resolved: Vec<&str> = link_relative.split('/').collect();
    resolved.pop();
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if resolved.pop().is_none() {
                    return Err(ReleaseError::InvalidTree(format!(
                        "payload symlink {link_relative} escapes the tree via {target_str:?}"
                    )));
                }
            }
            Component::Normal(part) => {
                resolved.push(part.to_str().ok_or_else(|| {
                    ReleaseError::InvalidTree(format!(
                        "payload symlink target is not valid UTF-8: {link_relative}"
                    ))
                })?);
            }
            _ => {
                return Err(ReleaseError::InvalidTree(format!(
                    "payload symlink {link_relative} has an unsupported target {target_str:?}"
                )));
            }
        }
    }
    Ok(target_str.to_owned())
}

/// Every regular file and (relative, in-tree) symlink under `root` as
/// `(relative POSIX path, entry)`, in a global lexicographic sort of the
/// relative path strings — the same traversal order as the skills digest.
fn payload_tree_entries(root: &Path) -> Result<Vec<(String, PayloadEntry)>, ReleaseError> {
    let metadata = fs::symlink_metadata(root).map_err(|_| unavailable_payload_tree(root))?;
    if !metadata.is_dir() {
        return Err(unavailable_payload_tree(root));
    }
    let mut entries = Vec::new();
    collect_payload_entries(root, root, &mut entries)?;
    entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    Ok(entries)
}

fn unavailable_payload_tree(root: &Path) -> ReleaseError {
    ReleaseError::InvalidTree(format!("payload tree is unavailable: {}", root.display()))
}

fn relative_posix_path(root: &Path, path: &Path) -> Result<String, ReleaseError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        ReleaseError::InvalidTree(format!("path escapes the payload tree: {}", path.display()))
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
            ReleaseError::InvalidTree(format!("file name is not valid UTF-8: {}", path.display()))
        })?;
        if !posix.is_empty() {
            posix.push('/');
        }
        posix.push_str(part);
    }
    Ok(posix)
}

fn collect_payload_entries(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<(String, PayloadEntry)>,
) -> Result<(), ReleaseError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            let relative = relative_posix_path(root, &path)?;
            let target = fs::read_link(&path)?;
            let target = validate_symlink_target(&relative, &target)?;
            entries.push((relative, PayloadEntry::Symlink(target)));
            continue;
        }
        if file_type.is_dir() {
            collect_payload_entries(root, &path, entries)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(ReleaseError::InvalidTree(format!(
                "payload tree contains a non-file entry: {}",
                path.display()
            )));
        }
        entries.push((relative_posix_path(root, &path)?, PayloadEntry::File(path)));
    }
    Ok(())
}

/// The payload tree digest: the skills digest algorithm generalized with a
/// distinct domain prefix, a per-entry kind byte, and symlink entries hashed
/// over their target string.
pub fn compute_payload_tree_digest(root: &Path) -> Result<String, ReleaseError> {
    let entries = payload_tree_entries(root)?;
    let mut digest = Sha256::new();
    digest.update(PAYLOAD_TREE_DIGEST_DOMAIN);
    let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
    for (relative, entry) in &entries {
        let relative_bytes = relative.as_bytes();
        digest.update((relative_bytes.len() as u64).to_be_bytes());
        digest.update(relative_bytes);
        match entry {
            PayloadEntry::File(path) => {
                digest.update([PAYLOAD_ENTRY_KIND_FILE]);
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
            PayloadEntry::Symlink(target) => {
                digest.update([PAYLOAD_ENTRY_KIND_SYMLINK]);
                let target_bytes = target.as_bytes();
                digest.update((target_bytes.len() as u64).to_be_bytes());
                digest.update(Sha256::digest(target_bytes));
            }
        }
    }
    Ok(hex::encode(digest.finalize()))
}

/// Pack `source` into `<artifact-id>.tar.gz` plus a signed
/// `<artifact-id>.tar.gz.manifest.json` under `out_dir` (the manifest URL for
/// a hosted bundle is derived by appending `.manifest.json` to its tarball
/// URL — the same convention as skills bundles).
///
/// The tarball is deterministic: entries sorted by relative POSIX path,
/// zeroed mtimes/uids/gids, normalized 0644/0755 file modes (0o777 for
/// symlinks), root-relative paths.
pub fn pack_payload_bundle(
    request: &PayloadPackRequest<'_>,
    signing_key: &SigningKey,
) -> Result<PayloadPackOutput, ReleaseError> {
    validate_artifact_id(request.artifact_id)?;
    if request.version_label.is_empty() {
        return Err(ReleaseError::InvalidManifest(
            "version_label must not be empty".to_owned(),
        ));
    }
    ShellSemver::parse(request.min_shell_version)?;
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

    let entries = payload_tree_entries(request.source)?;
    if entries.is_empty() {
        return Err(ReleaseError::InvalidTree(format!(
            "payload tree contains no files: {}",
            request.source.display()
        )));
    }
    let tree_digest = compute_payload_tree_digest(request.source)?;

    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (relative, entry) in &entries {
        match entry {
            PayloadEntry::File(path) => {
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
            PayloadEntry::Symlink(target) => {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_mode(0o777);
                header.set_mtime(0);
                header.set_uid(0);
                header.set_gid(0);
                builder.append_link(&mut header, relative, target)?;
            }
        }
    }
    let tarball_bytes = builder.into_inner()?.finish()?;
    let tarball_sha256 = sha256_hex(&tarball_bytes);

    let mut manifest = PayloadBundleManifestV1 {
        schema: PAYLOAD_BUNDLE_SCHEMA.to_owned(),
        artifact_id: request.artifact_id.to_owned(),
        version_label: request.version_label.to_owned(),
        source_git_sha: request.source_git_sha.map(str::to_owned),
        min_shell_version: request.min_shell_version.to_owned(),
        tree_digest,
        tarball_sha256,
        created_at,
        signature: String::new(),
    };
    manifest.signature = signature_for_payload_manifest(&manifest, signing_key)?;
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

    Ok(PayloadPackOutput {
        manifest,
        tarball_path,
        manifest_path,
    })
}

/// Unpack a payload tarball into `destination`, admitting regular files,
/// directories, and relative in-tree symlinks with plain relative paths.
/// Symlinks are recreated exactly; their targets are re-validated against the
/// same lexical in-tree rule enforced at pack time.
pub fn unpack_payload_tarball(tarball: &Path, destination: &Path) -> Result<(), ReleaseError> {
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
            tar::EntryType::Symlink => {
                let relative = path.to_str().ok_or_else(|| {
                    ReleaseError::VerificationFailed(format!(
                        "tarball entry name is not valid UTF-8: {}",
                        path.display()
                    ))
                })?;
                let link_target = entry
                    .link_name()?
                    .ok_or_else(|| {
                        ReleaseError::VerificationFailed(format!(
                            "tarball symlink entry {relative} has no target"
                        ))
                    })?
                    .into_owned();
                let link_target = validate_symlink_target(relative, &link_target)
                    .map_err(|error| ReleaseError::VerificationFailed(error.to_string()))?;
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                std::os::unix::fs::symlink(&link_target, &target)?;
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

/// Verify a payload bundle end to end: tarball digest against the manifest
/// (and an optional caller expectation), manifest signature against the
/// release public key, then the recomputed payload tree digest of the bytes
/// unpacked into `unpack_dir`.
pub fn verify_payload_bundle(
    tarball: &Path,
    manifest_path: &Path,
    public_key: &VerifyingKey,
    expected_tarball_sha256: Option<&str>,
    unpack_dir: &Path,
) -> Result<VerifiedPayloadBundle, ReleaseError> {
    let manifest = PayloadBundleManifestV1::from_json_bytes(&fs::read(manifest_path)?)?;
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
    verify_payload_manifest_signature(&manifest, public_key)?;
    unpack_payload_tarball(tarball, unpack_dir)?;
    let tree_digest = compute_payload_tree_digest(unpack_dir)?;
    if tree_digest != manifest.tree_digest {
        return Err(ReleaseError::VerificationFailed(format!(
            "unpacked tree digest {tree_digest} does not match the manifest's {}",
            manifest.tree_digest
        )));
    }
    Ok(VerifiedPayloadBundle { manifest })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden digest of `tests/fixtures/payload-tree` (a `bin/` script plus a
    /// relative venv-style symlink). If a digest change is intentional,
    /// recompute and update this constant.
    const FIXTURE_PAYLOAD_TREE_DIGEST: &str =
        "2ab319f0ab2d7e01b34fc50b41864268a51d6ff8f65e66d2e42b71d7a424e93e";

    fn fixture_tree() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/payload-tree")
    }

    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7_u8; 32])
    }

    fn pack_fixture(out_dir: &Path) -> PayloadPackOutput {
        pack_payload_bundle(
            &PayloadPackRequest {
                source: &fixture_tree(),
                artifact_id: "payload-test",
                version_label: "2026.08.06-test",
                min_shell_version: "0.1.0",
                source_git_sha: Some("c24cd96f00000000000000000000000000000000"),
                created_at: Some("2026-08-06T00:00:00Z"),
                out_dir,
            },
            &test_signing_key(),
        )
        .unwrap()
    }

    #[test]
    fn fixture_payload_tree_digest_matches_the_golden_constant() {
        assert_eq!(
            compute_payload_tree_digest(&fixture_tree()).unwrap(),
            FIXTURE_PAYLOAD_TREE_DIGEST
        );
    }

    #[test]
    fn a_symlink_and_a_file_with_the_target_as_content_digest_differently() {
        let link_dir = tempfile::tempdir().unwrap();
        fs::write(link_dir.path().join("real"), "x").unwrap();
        std::os::unix::fs::symlink("real", link_dir.path().join("entry")).unwrap();

        let file_dir = tempfile::tempdir().unwrap();
        fs::write(file_dir.path().join("real"), "x").unwrap();
        fs::write(file_dir.path().join("entry"), "real").unwrap();

        assert_ne!(
            compute_payload_tree_digest(link_dir.path()).unwrap(),
            compute_payload_tree_digest(file_dir.path()).unwrap()
        );
    }

    #[test]
    fn escaping_and_absolute_symlinks_are_rejected() {
        for target in ["../../outside", "x/../../../outside", "/etc/passwd"] {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("sub")).unwrap();
            fs::write(dir.path().join("sub/file"), "x").unwrap();
            std::os::unix::fs::symlink(target, dir.path().join("sub/link")).unwrap();
            let error = compute_payload_tree_digest(dir.path()).unwrap_err();
            assert!(
                matches!(error, ReleaseError::InvalidTree(_)),
                "target {target:?} must be rejected"
            );
        }
    }

    #[test]
    fn an_in_tree_parent_dir_symlink_is_admitted() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        fs::write(dir.path().join("a/file"), "x").unwrap();
        std::os::unix::fs::symlink("../file", dir.path().join("a/b/link")).unwrap();
        compute_payload_tree_digest(dir.path()).unwrap();
    }

    #[test]
    fn min_shell_version_parses_strict_semver_only() {
        assert_eq!(
            ShellSemver::parse("1.2.3").unwrap(),
            ShellSemver {
                major: 1,
                minor: 2,
                patch: 3
            }
        );
        assert_eq!(ShellSemver::parse("0.1.0").unwrap().to_string(), "0.1.0");
        assert!(ShellSemver::parse("10.0.0").unwrap() > ShellSemver::parse("9.9.9").unwrap());
        assert!(ShellSemver::parse("0.2.0").unwrap() > ShellSemver::parse("0.1.9").unwrap());
        for invalid in [
            "1.2", "1.2.3.4", "1.2.x", "v1.2.3", "1.2.-3", "01.2.3", "", "1..3",
        ] {
            assert!(
                ShellSemver::parse(invalid).is_err(),
                "{invalid:?} must not parse"
            );
        }
    }

    #[test]
    fn packing_is_deterministic_and_roundtrips_with_symlinks_recreated() {
        let out_one = tempfile::tempdir().unwrap();
        let out_two = tempfile::tempdir().unwrap();
        let packed_one = pack_fixture(out_one.path());
        let packed_two = pack_fixture(out_two.path());
        assert_eq!(
            packed_one.manifest.tarball_sha256, packed_two.manifest.tarball_sha256,
            "packing the same tree twice must produce identical tarball bytes"
        );
        assert_eq!(packed_one.manifest.tree_digest, FIXTURE_PAYLOAD_TREE_DIGEST);
        assert_eq!(
            packed_one.manifest_path.file_name().unwrap().to_str(),
            Some("payload-test.tar.gz.manifest.json")
        );

        let unpack = tempfile::tempdir().unwrap();
        let tree = unpack.path().join("tree");
        let verified = verify_payload_bundle(
            &packed_one.tarball_path,
            &packed_one.manifest_path,
            &test_signing_key().verifying_key(),
            Some(&packed_one.manifest.tarball_sha256),
            &tree,
        )
        .unwrap();
        assert_eq!(verified.manifest, packed_one.manifest);
        let link = tree.join("hermes-venv/bin/python");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(&link).unwrap(),
            PathBuf::from("python3.11"),
            "the relative symlink must be recreated exactly"
        );
        let script = fs::symlink_metadata(tree.join("bin/finite-agentd")).unwrap();
        assert_eq!(
            script.permissions().mode() & 0o777,
            0o755,
            "executable bits must survive the roundtrip"
        );
    }

    #[test]
    fn a_tampered_payload_tarball_or_wrong_key_fails_verification() {
        let out = tempfile::tempdir().unwrap();
        let packed = pack_fixture(out.path());
        let mut bytes = fs::read(&packed.tarball_path).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x01;
        fs::write(&packed.tarball_path, &bytes).unwrap();
        let unpack = tempfile::tempdir().unwrap();
        let error = verify_payload_bundle(
            &packed.tarball_path,
            &packed.manifest_path,
            &test_signing_key().verifying_key(),
            None,
            &unpack.path().join("tree"),
        )
        .unwrap_err();
        assert!(matches!(error, ReleaseError::VerificationFailed(_)));

        bytes[middle] ^= 0x01;
        fs::write(&packed.tarball_path, &bytes).unwrap();
        let wrong_key = SigningKey::from_bytes(&[8_u8; 32]).verifying_key();
        let error = verify_payload_bundle(
            &packed.tarball_path,
            &packed.manifest_path,
            &wrong_key,
            None,
            &unpack.path().join("tree-two"),
        )
        .unwrap_err();
        assert!(matches!(error, ReleaseError::VerificationFailed(_)));
    }

    #[test]
    fn unpacking_rejects_a_forged_escaping_symlink_entry() {
        let dir = tempfile::tempdir().unwrap();
        let tarball = dir.path().join("escape.tar.gz");
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_mtime(0);
        builder
            .append_link(&mut header, "link", "../outside")
            .unwrap();
        fs::write(&tarball, builder.into_inner().unwrap().finish().unwrap()).unwrap();

        let error = unpack_payload_tarball(&tarball, &dir.path().join("tree")).unwrap_err();
        assert!(matches!(error, ReleaseError::VerificationFailed(_)));
        assert!(error.to_string().contains("escapes the tree"));
    }

    #[test]
    fn payload_canonical_signing_bytes_include_min_shell_version() {
        let manifest = PayloadBundleManifestV1 {
            schema: PAYLOAD_BUNDLE_SCHEMA.to_owned(),
            artifact_id: "payload-test".to_owned(),
            version_label: "v1".to_owned(),
            source_git_sha: None,
            min_shell_version: "0.1.0".to_owned(),
            tree_digest: "1".repeat(64),
            tarball_sha256: "2".repeat(64),
            created_at: "2026-08-06T00:00:00Z".to_owned(),
            signature: "ed25519:ignored".to_owned(),
        };
        let bytes = payload_canonical_signing_bytes(&manifest).unwrap();
        let expected = format!(
            "{{\"artifact_id\":\"payload-test\",\"created_at\":\"2026-08-06T00:00:00Z\",\
             \"min_shell_version\":\"0.1.0\",\"schema\":\"finite_payload_bundle.v1\",\
             \"source_git_sha\":null,\"tarball_sha256\":\"{}\",\"tree_digest\":\"{}\",\
             \"version_label\":\"v1\"}}",
            "2".repeat(64),
            "1".repeat(64)
        );
        assert_eq!(String::from_utf8(bytes).unwrap(), expected);

        let mut forged = manifest.clone();
        forged.min_shell_version = "9.0.0".to_owned();
        let signed = signature_for_payload_manifest(&manifest, &test_signing_key()).unwrap();
        forged.signature = signed.clone();
        assert!(
            verify_payload_manifest_signature(&forged, &test_signing_key().verifying_key())
                .is_err(),
            "min_shell_version must be covered by the signature"
        );
    }
}
