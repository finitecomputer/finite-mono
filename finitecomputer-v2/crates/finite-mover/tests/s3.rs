//! The uploader against a real S3 implementation, fed by the host reader's
//! own `stream`, as the mover runs them. Run by name with a local S3:
//!
//! ```text
//! FINITE_MOVER_TEST_S3_ENDPOINT=http://127.0.0.1:58333 \
//! FINITE_MOVER_TEST_S3_ACCESS_KEY=... FINITE_MOVER_TEST_S3_SECRET_KEY=... \
//! cargo test -p finite-mover --test s3 -- --ignored
//! ```

use std::cell::Cell;
use std::io::{self, Read};
use std::path::Path;

use finite_mover::bucket::{Bucket, BucketError, Part, Upload};
use finite_mover::s3::{S3Bucket, S3Config};
use finite_mover::stage::{PART_BYTES_MIN, Phase, StageError, StageTarget, upload_stage};
use finite_mover_host::command::{parse, run};
use finite_mover_host::stage::SUMMARY_ENTRY;
use finite_mover_host::zip64::ZipStream;
use zeroize::Zeroizing;

const MIB: usize = 1024 * 1024;

fn s3() -> S3Bucket {
    let env =
        |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} names the local S3"));
    let bucket = S3Bucket::new(S3Config {
        endpoint: env("FINITE_MOVER_TEST_S3_ENDPOINT").parse().unwrap(),
        bucket: "fragment-migrations-stage-test".to_string(),
        region: "auto".to_string(),
        access_key_id: env("FINITE_MOVER_TEST_S3_ACCESS_KEY"),
        secret_access_key: Zeroizing::new(env("FINITE_MOVER_TEST_S3_SECRET_KEY")),
    })
    .unwrap();
    match bucket.create_bucket() {
        Ok(()) | Err(BucketError::Refused { status: 409, .. }) => {}
        Err(error) => panic!("{error}"),
    }
    bucket
}

/// A unique old-agent id per run, so runs never meet.
fn old(name: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("proj_{name}_{now}")
}

/// Counts the parts it uploads; everything else passes through.
struct Counted<'a> {
    inner: &'a S3Bucket,
    parts: Cell<u32>,
}

impl Bucket for Counted<'_> {
    fn list_keys(&self, prefix: &str) -> Result<Vec<String>, BucketError> {
        self.inner.list_keys(prefix)
    }
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, BucketError> {
        self.inner.get(key)
    }
    fn put(&self, key: &str, body: &[u8], content_type: &str) -> Result<(), BucketError> {
        self.inner.put(key, body, content_type)
    }
    fn uploads(&self, prefix: &str) -> Result<Vec<Upload>, BucketError> {
        self.inner.uploads(prefix)
    }
    fn create_upload(&self, key: &str) -> Result<String, BucketError> {
        self.inner.create_upload(key)
    }
    fn parts(&self, key: &str, upload_id: &str) -> Result<Vec<Part>, BucketError> {
        self.inner.parts(key, upload_id)
    }
    fn upload_part(
        &self,
        key: &str,
        upload_id: &str,
        number: u32,
        body: &[u8],
    ) -> Result<String, BucketError> {
        self.parts.set(self.parts.get() + 1);
        self.inner.upload_part(key, upload_id, number, body)
    }
    fn complete(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[(u32, String)],
    ) -> Result<(), BucketError> {
        self.inner.complete(key, upload_id, parts)
    }
    fn abort(&self, key: &str, upload_id: &str) -> Result<(), BucketError> {
        self.inner.abort(key, upload_id)
    }
}

/// A stopped agent's tree, with secrets the stage must not hold.
fn tree(root: &Path) {
    let write = |path: &str, body: &[u8]| {
        let path = root.join("kata/rt_s3").join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    };
    write("agent/hermes-home/SOUL.md", b"kind and brief\n");
    write("agent/hermes-home/memories/MEMORY.md", b"likes tea\n");
    write(
        "agent/hermes-home/.env",
        b"OPENROUTER_API_KEY='sk-or-v1-0123456789abcdef0123'\n",
    );
    write("agent/identity/identity.json", b"{\"secret_hex\":\"01\"}");
    write("workspace/notes.md", b"notes\n");
    let photo: Vec<u8> = (0..12 * MIB).map(|index| (index % 253) as u8).collect();
    write("workspace/photos/big.jpg", &photo);
}

/// `finite-mover-host stream rt_s3`, as the ssh forced command would print.
fn host_stream(work_root: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    run(work_root, parse(["stream", "rt_s3"]).unwrap(), &mut out).unwrap();
    out
}

struct Breaks<'a> {
    bytes: &'a [u8],
    limit: usize,
}

impl Read for Breaks<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.limit == 0 {
            return Err(io::Error::new(io::ErrorKind::ConnectionReset, "ssh closed"));
        }
        let length = buffer.len().min(self.limit).min(self.bytes.len());
        buffer[..length].copy_from_slice(&self.bytes[..length]);
        self.bytes = &self.bytes[length..];
        self.limit -= length;
        Ok(length)
    }
}

#[test]
#[ignore = "needs a local S3 (FINITE_MOVER_TEST_S3_*)"]
fn the_host_stream_is_staged_resumed_and_read_back() {
    let bucket = s3();
    let dir = tempfile::Builder::new()
        .prefix("mover-s3-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    tree(dir.path());
    let stream = host_stream(dir.path());
    let target = StageTarget::new(Phase::P1, &old("resume"), 1).unwrap();

    // ssh drops after two parts; the parts stay in the bucket's upload.
    let counted = Counted {
        inner: &bucket,
        parts: Cell::new(0),
    };
    let broken = Breaks {
        bytes: &stream,
        limit: 2 * PART_BYTES_MIN + 1,
    };
    let error = upload_stage(&counted, &target, broken, PART_BYTES_MIN, dir.path()).unwrap_err();
    assert!(matches!(error, StageError::Read(_)), "{error}");
    assert_eq!(counted.parts.get(), 2);
    let pending = bucket.uploads(&target.zip_key()).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        bucket
            .parts(&target.zip_key(), &pending[0].id)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        bucket.get(&target.zip_key()).unwrap(),
        None,
        "nothing readable mid-upload"
    );

    // The next run streams again and sends only what is missing.
    counted.parts.set(0);
    let manifest =
        upload_stage(&counted, &target, &stream[..], PART_BYTES_MIN, dir.path()).unwrap();
    assert_eq!(counted.parts.get(), 1, "only the last part is sent again");
    let staged = bucket.get(&target.zip_key()).unwrap().unwrap();
    assert_eq!(staged, stream);
    let stored = bucket.get(&target.manifest_key()).unwrap().unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&stored).unwrap();
    assert_eq!(stored, serde_json::to_value(&manifest).unwrap());
    assert_eq!((manifest.files, manifest.attempt), (4, 1));
    // The host's left lines arrive by class: the identity and the .env.
    let left: Vec<(&str, &str)> = manifest
        .left
        .iter()
        .map(|line| (line.path.as_str(), line.why.as_str()))
        .collect();
    assert!(left.contains(&("agent/identity", "secret")), "{left:?}");
    assert!(
        left.contains(&("agent/hermes-home/.env", "secret")),
        "{left:?}"
    );

    let mut zip = zip::ZipArchive::new(io::Cursor::new(staged)).unwrap();
    let mut names: Vec<String> = zip.file_names().map(str::to_string).collect();
    names.sort();
    assert_eq!(
        names,
        [
            SUMMARY_ENTRY,
            "agent/hermes-home/SOUL.md",
            "agent/hermes-home/memories/MEMORY.md",
            "workspace/notes.md",
            "workspace/photos/big.jpg",
        ]
    );
    let mut photo = Vec::new();
    zip.by_name("workspace/photos/big.jpg")
        .unwrap()
        .read_to_end(&mut photo)
        .unwrap();
    assert_eq!(photo.len(), 12 * MIB);
    assert!(bucket.uploads(&target.zip_key()).unwrap().is_empty());
}

#[test]
#[ignore = "needs a local S3 (FINITE_MOVER_TEST_S3_*)"]
fn a_secret_path_is_refused_and_leaves_nothing() {
    let bucket = s3();
    let dir = tempfile::Builder::new()
        .prefix("mover-s3-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let target = StageTarget::new(Phase::P1, &old("refused"), 1).unwrap();
    let mut zip = ZipStream::new(Vec::new());
    zip.entry(
        "workspace/photos/a.jpg",
        0o644,
        0,
        &mut &vec![7u8; 6 * MIB][..],
    )
    .unwrap();
    zip.entry("workspace/.env", 0o600, 0, &mut &b"TOKEN=1"[..])
        .unwrap();
    zip.entry(SUMMARY_ENTRY, 0o644, 0, &mut &b"{}"[..]).unwrap();
    let stream = zip.finish().unwrap();
    let error =
        upload_stage(&bucket, &target, &stream[..], PART_BYTES_MIN, dir.path()).unwrap_err();
    assert!(
        matches!(&error, StageError::Refused { path } if path == "workspace/.env"),
        "{error}"
    );
    assert!(
        bucket.uploads(&target.zip_key()).unwrap().is_empty(),
        "the upload is aborted"
    );
    assert_eq!(bucket.get(&target.zip_key()).unwrap(), None);
    assert_eq!(bucket.get(&target.manifest_key()).unwrap(), None);
}

#[test]
#[ignore = "needs a local S3 (FINITE_MOVER_TEST_S3_*)"]
fn a_stale_attempt_writes_nothing() {
    let bucket = s3();
    let dir = tempfile::Builder::new()
        .prefix("mover-s3-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    tree(dir.path());
    let stream = host_stream(dir.path());
    let old = old("stale");
    let second = StageTarget::new(Phase::P1, &old, 2).unwrap();
    upload_stage(&bucket, &second, &stream[..], PART_BYTES_MIN, dir.path()).unwrap();
    let first = StageTarget::new(Phase::P1, &old, 1).unwrap();
    let error = upload_stage(&bucket, &first, &stream[..], PART_BYTES_MIN, dir.path()).unwrap_err();
    assert!(
        matches!(error, StageError::StaleAttempt { newer: 2 }),
        "{error}"
    );
    assert!(
        bucket
            .list_keys(&format!("p1/{old}/1/"))
            .unwrap()
            .is_empty()
    );
    assert!(bucket.uploads(&format!("p1/{old}/1/")).unwrap().is_empty());
}
