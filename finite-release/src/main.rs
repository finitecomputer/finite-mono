use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use finite_release::{
    PackRequest, ReleaseError, generate_signing_key, pack_skills_bundle, parse_signing_key_hex,
    parse_verifying_key_hex, verify_skills_bundle,
};

#[derive(Debug, Parser)]
#[command(name = "finite-release")]
#[command(about = "Pack, sign, and verify Finite release material")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Generate a release signing keypair (release.key seed + release.pub).
    Keygen {
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Pack a skills tree into a deterministic tarball plus signed manifest.
    PackSkills {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        artifact_id: String,
        #[arg(long)]
        version_label: String,
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long)]
        source_git_sha: Option<String>,
        #[arg(long)]
        signing_key: PathBuf,
        /// RFC 3339 creation instant; defaults to the current UTC time.
        #[arg(long)]
        now: Option<String>,
    },
    /// Verify a packed skills bundle end to end.
    VerifySkills {
        #[arg(long)]
        tarball: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
        /// 32-byte hex public key, or a path to a file containing it.
        #[arg(long)]
        public_key: String,
        #[arg(long)]
        expected_tarball_sha256: Option<String>,
    },
}

fn main() {
    if let Err(error) = run(Args::parse()) {
        eprintln!("finite-release: {error}");
        std::process::exit(1);
    }
}

fn run(args: Args) -> Result<(), ReleaseError> {
    match args.command {
        Command::Keygen { out_dir } => keygen(&out_dir),
        Command::PackSkills {
            source,
            artifact_id,
            version_label,
            out_dir,
            source_git_sha,
            signing_key,
            now,
        } => {
            let signing_key = parse_signing_key_hex(&fs::read_to_string(&signing_key)?)?;
            let packed = pack_skills_bundle(
                &PackRequest {
                    source: &source,
                    artifact_id: &artifact_id,
                    version_label: &version_label,
                    source_git_sha: source_git_sha.as_deref(),
                    created_at: now.as_deref(),
                    out_dir: &out_dir,
                },
                &signing_key,
            )?;
            println!(
                "{}",
                serde_json::json!({
                    "artifactId": packed.manifest.artifact_id,
                    "tarballSha256": packed.manifest.tarball_sha256,
                    "treeDigest": packed.manifest.tree_digest,
                    "paths": {
                        "tarball": packed.tarball_path,
                        "manifest": packed.manifest_path,
                    },
                })
            );
            Ok(())
        }
        Command::VerifySkills {
            tarball,
            manifest,
            public_key,
            expected_tarball_sha256,
        } => {
            let public_key = load_public_key(&public_key)?;
            let unpack_dir = tempfile::tempdir()?;
            let verified = verify_skills_bundle(
                &tarball,
                &manifest,
                &public_key,
                expected_tarball_sha256.as_deref(),
                &unpack_dir.path().join("tree"),
            )?;
            println!(
                "{}",
                serde_json::json!({
                    "verified": true,
                    "artifactId": verified.manifest.artifact_id,
                    "versionLabel": verified.manifest.version_label,
                    "tarballSha256": verified.manifest.tarball_sha256,
                    "treeDigest": verified.manifest.tree_digest,
                })
            );
            Ok(())
        }
    }
}

fn keygen(out_dir: &Path) -> Result<(), ReleaseError> {
    fs::create_dir_all(out_dir)?;
    let signing_key = generate_signing_key()?;
    let key_path = out_dir.join("release.key");
    let mut key_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&key_path)?;
    key_file.write_all(hex::encode(signing_key.to_bytes()).as_bytes())?;
    key_file.write_all(b"\n")?;
    let public_path = out_dir.join("release.pub");
    fs::write(
        &public_path,
        format!("{}\n", hex::encode(signing_key.verifying_key().to_bytes())),
    )?;
    println!(
        "{}",
        serde_json::json!({
            "signingKey": key_path,
            "publicKey": public_path,
        })
    );
    Ok(())
}

fn load_public_key(value: &str) -> Result<ed25519_dalek::VerifyingKey, ReleaseError> {
    let trimmed = value.trim();
    if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return parse_verifying_key_hex(trimmed);
    }
    parse_verifying_key_hex(&fs::read_to_string(trimmed)?)
}
