use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, Read};

use finite_mover_host::zip64::ZipStream;

use serde_json::json;

use super::*;
use crate::bucket::Part;

const MIB: usize = 1024 * 1024;

/// An in-memory stage bucket that records the calls that write.
#[derive(Default)]
struct Memory {
    state: RefCell<State>,
}

#[derive(Default)]
struct State {
    objects: BTreeMap<String, Vec<u8>>,
    uploads: BTreeMap<String, (String, BTreeMap<u32, Vec<u8>>)>,
    next_id: u32,
    writes: Vec<String>,
}

fn md5_hex(bytes: &[u8]) -> String {
    hex::encode(Md5::digest(bytes))
}

impl Bucket for Memory {
    fn list_keys(&self, prefix: &str) -> Result<Vec<String>, BucketError> {
        let state = self.state.borrow();
        Ok(state
            .objects
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect())
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, BucketError> {
        Ok(self.state.borrow().objects.get(key).cloned())
    }

    fn put(&self, key: &str, body: &[u8], _content_type: &str) -> Result<(), BucketError> {
        let mut state = self.state.borrow_mut();
        state.writes.push(format!("put {key}"));
        state.objects.insert(key.to_string(), body.to_vec());
        Ok(())
    }

    fn uploads(&self, prefix: &str) -> Result<Vec<Upload>, BucketError> {
        let state = self.state.borrow();
        Ok(state
            .uploads
            .iter()
            .filter(|(_, (key, _))| key.starts_with(prefix))
            .map(|(id, (key, _))| Upload {
                key: key.clone(),
                id: id.clone(),
            })
            .collect())
    }

    fn create_upload(&self, key: &str) -> Result<String, BucketError> {
        let mut state = self.state.borrow_mut();
        state.next_id += 1;
        let id = format!("upload-{}", state.next_id);
        state.writes.push(format!("create {key}"));
        state
            .uploads
            .insert(id.clone(), (key.to_string(), BTreeMap::new()));
        Ok(id)
    }

    fn parts(&self, _key: &str, upload_id: &str) -> Result<Vec<Part>, BucketError> {
        let state = self.state.borrow();
        Ok(state.uploads[upload_id]
            .1
            .iter()
            .map(|(number, body)| Part {
                number: *number,
                etag: md5_hex(body),
                size: body.len() as u64,
            })
            .collect())
    }

    fn upload_part(
        &self,
        _key: &str,
        upload_id: &str,
        number: u32,
        body: &[u8],
    ) -> Result<String, BucketError> {
        let mut state = self.state.borrow_mut();
        state.writes.push(format!("part {number}"));
        state
            .uploads
            .get_mut(upload_id)
            .expect("an upload in progress")
            .1
            .insert(number, body.to_vec());
        Ok(md5_hex(body))
    }

    fn complete(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[(u32, String)],
    ) -> Result<(), BucketError> {
        let mut state = self.state.borrow_mut();
        let (_, stored) = state
            .uploads
            .remove(upload_id)
            .expect("an upload in progress");
        let mut object = Vec::new();
        for (number, etag) in parts {
            let body = &stored[number];
            assert_eq!(&md5_hex(body), etag, "part {number}'s ETag");
            object.extend_from_slice(body);
        }
        state.writes.push(format!("complete {key}"));
        state.objects.insert(key.to_string(), object);
        Ok(())
    }

    fn abort(&self, key: &str, upload_id: &str) -> Result<(), BucketError> {
        let mut state = self.state.borrow_mut();
        state.writes.push(format!("abort {key}"));
        state.uploads.remove(upload_id);
        Ok(())
    }
}

impl Memory {
    fn writes(&self) -> Vec<String> {
        std::mem::take(&mut self.state.borrow_mut().writes)
    }
}

/// A stage as the host writes it: entries, then the summary.
fn stage(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut stream = ZipStream::new(Vec::new());
    for (name, body) in files {
        stream.entry(name, 0o644, 0, &mut &body[..]).unwrap();
    }
    let summary = br#"{"format":"finite-mover-host/summary/1","id":"runtime_1",
        "left":[{"path":"agent/identity","class":"secret","why":"nostr-identity","files":1,"bytes":300},
                {"path":"workspace/app/node_modules","class":"cache","why":"cache-dir","files":40,"bytes":9000}],
        "left_more":{"files":7,"bytes":70}}"#;
    stream
        .entry(SUMMARY_ENTRY, 0o644, 0, &mut &summary[..])
        .unwrap();
    stream.finish().unwrap()
}

/// About 12 MiB: three 5 MiB parts.
fn big_stage() -> Vec<u8> {
    let photo: Vec<u8> = (0..12 * MIB).map(|index| (index % 251) as u8).collect();
    stage(&[
        ("agent/hermes-home/SOUL.md", b"me".to_vec()),
        ("workspace/photos/cat.jpg", photo),
    ])
}

fn target(attempt: u32) -> StageTarget {
    StageTarget::new(Phase::P1, "proj_old1", attempt).unwrap()
}

/// A stream that fails after `limit` bytes, as a dropped ssh would.
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
fn a_stage_is_uploaded_whole_and_its_manifest_comes_last() {
    let bucket = Memory::default();
    let bytes = big_stage();
    let manifest = upload_stage(&bucket, &target(1), &bytes[..], PART_BYTES_MIN).unwrap();
    assert_eq!(
        bucket.writes(),
        [
            "create p1/proj_old1/1/stage.zip",
            "part 1",
            "part 2",
            "part 3",
            "complete p1/proj_old1/1/stage.zip",
            "put p1/proj_old1/1/manifest.json",
        ]
    );
    let state = bucket.state.borrow();
    assert_eq!(state.objects["p1/proj_old1/1/stage.zip"], bytes);
    let stored: Manifest =
        serde_json::from_slice(&state.objects["p1/proj_old1/1/manifest.json"]).unwrap();
    assert_eq!(stored, manifest);
    // The host's left lines arrive by class; the rest are counted.
    assert_eq!(
        serde_json::to_value(&manifest).unwrap(),
        json!({
            "format": "finite-stage/1", "old": "proj_old1", "phase": "p1", "attempt": 1,
            "layout": "finite-computer", "zip_bytes": bytes.len(),
            "zip_sha256": hex::encode(Sha256::digest(&bytes)),
            "bytes": 2 + 12 * MIB, "files": 2,
            "left": [
                {"path": "agent/identity", "bytes": 300, "why": "secret"},
                {"path": "workspace/app/node_modules", "bytes": 9000, "why": "cache"},
            ],
            "left_more": 7,
        })
    );
}

#[test]
fn an_interrupted_upload_resumes_without_resending_its_parts() {
    let bucket = Memory::default();
    let bytes = big_stage();
    let broken = Breaks {
        bytes: &bytes,
        limit: 2 * PART_BYTES_MIN + 100,
    };
    let error = upload_stage(&bucket, &target(1), broken, PART_BYTES_MIN).unwrap_err();
    assert!(matches!(error, StageError::Read(_)), "{error}");
    assert_eq!(
        bucket.writes(),
        ["create p1/proj_old1/1/stage.zip", "part 1", "part 2"]
    );
    assert!(
        bucket.state.borrow().objects.is_empty(),
        "nothing readable yet"
    );

    // The mover runs `stream` again and the upload picks up where it was.
    upload_stage(&bucket, &target(1), &bytes[..], PART_BYTES_MIN).unwrap();
    assert_eq!(
        bucket.writes(),
        [
            "part 3",
            "complete p1/proj_old1/1/stage.zip",
            "put p1/proj_old1/1/manifest.json",
        ]
    );
    assert_eq!(
        bucket.state.borrow().objects["p1/proj_old1/1/stage.zip"],
        bytes
    );
}

#[test]
fn a_part_that_differs_on_resume_is_uploaded_again() {
    let bucket = Memory::default();
    let id = bucket.create_upload("p1/proj_old1/1/stage.zip").unwrap();
    bucket
        .upload_part("p1/proj_old1/1/stage.zip", &id, 1, &vec![9; PART_BYTES_MIN])
        .unwrap();
    bucket
        .upload_part("p1/proj_old1/1/stage.zip", &id, 7, &vec![9; PART_BYTES_MIN])
        .unwrap();
    bucket.writes();
    let bytes = big_stage();
    upload_stage(&bucket, &target(1), &bytes[..], PART_BYTES_MIN).unwrap();
    assert_eq!(
        bucket.writes(),
        [
            "part 1",
            "part 2",
            "part 3",
            "complete p1/proj_old1/1/stage.zip",
            "put p1/proj_old1/1/manifest.json",
        ]
    );
    // Part 7, from an older and longer stream, is not part of the object.
    assert_eq!(
        bucket.state.borrow().objects["p1/proj_old1/1/stage.zip"],
        bytes
    );
}

#[test]
fn a_secret_path_refuses_the_whole_stage() {
    for secret in [
        "workspace/app/.env",
        "agent/identity/identity.json",
        "agent/hermes-home/config.yaml",
    ] {
        let bucket = Memory::default();
        let photo = vec![1; 6 * MIB];
        let bytes = stage(&[
            ("workspace/photos/cat.jpg", photo),
            (secret, b"TOKEN=1".to_vec()),
        ]);
        let error = upload_stage(&bucket, &target(1), &bytes[..], PART_BYTES_MIN).unwrap_err();
        assert!(
            matches!(&error, StageError::Refused { path } if path == secret),
            "{error}"
        );
        assert_eq!(
            bucket.writes(),
            [
                "create p1/proj_old1/1/stage.zip",
                "part 1",
                "part 2",
                "abort p1/proj_old1/1/stage.zip",
            ]
        );
        let state = bucket.state.borrow();
        assert!(state.uploads.is_empty(), "the parts are dropped");
        assert!(state.objects.is_empty(), "no stage, no manifest");
    }
}

#[test]
fn unsafe_names_and_unreadable_stages_are_refused() {
    let bucket = Memory::default();
    let error = upload_stage(
        &bucket,
        &target(1),
        &stage(&[("workspace/../x", vec![1])])[..],
        PART_BYTES_MIN,
    )
    .unwrap_err();
    assert!(matches!(error, StageError::UnsafeName { .. }), "{error}");
    assert!(bucket.state.borrow().uploads.is_empty());

    let mut no_summary = ZipStream::new(Vec::new());
    no_summary
        .entry("workspace/a.md", 0o644, 0, &mut &b"a"[..])
        .unwrap();
    let no_summary = no_summary.finish().unwrap();
    let error = upload_stage(&bucket, &target(1), &no_summary[..], PART_BYTES_MIN).unwrap_err();
    assert!(matches!(error, StageError::NoSummary), "{error}");

    let truncated = &big_stage()[..PART_BYTES_MIN + 10];
    let error = upload_stage(&bucket, &target(1), truncated, PART_BYTES_MIN).unwrap_err();
    assert!(matches!(error, StageError::Unreadable(_)), "{error}");
    let error = upload_stage(&bucket, &target(1), &b""[..], PART_BYTES_MIN).unwrap_err();
    assert!(matches!(error, StageError::Empty), "{error}");
    assert!(
        !bucket
            .state
            .borrow()
            .objects
            .contains_key("p1/proj_old1/1/manifest.json"),
        "nothing unchecked is ever staged"
    );
}

#[test]
fn a_stale_attempt_never_writes_and_a_newer_one_clears_it() {
    let bucket = Memory::default();
    let bytes = big_stage();
    // Attempt 1 stops halfway; attempt 2 begins and aborts it.
    let broken = Breaks {
        bytes: &bytes,
        limit: PART_BYTES_MIN + 1,
    };
    upload_stage(&bucket, &target(1), broken, PART_BYTES_MIN).unwrap_err();
    bucket.writes();
    upload_stage(&bucket, &target(2), &bytes[..], PART_BYTES_MIN).unwrap();
    let writes = bucket.writes();
    assert_eq!(writes[0], "abort p1/proj_old1/1/stage.zip");
    assert_eq!(writes.last().unwrap(), "put p1/proj_old1/2/manifest.json");

    // A late command for attempt 1 now writes nothing.
    let error = upload_stage(&bucket, &target(1), &bytes[..], PART_BYTES_MIN).unwrap_err();
    assert!(
        matches!(error, StageError::StaleAttempt { newer: 2 }),
        "{error}"
    );
    assert!(bucket.writes().is_empty());
}

#[test]
fn a_staged_attempt_is_answered_from_its_manifest() {
    let bucket = Memory::default();
    let bytes = big_stage();
    let first = upload_stage(&bucket, &target(1), &bytes[..], PART_BYTES_MIN).unwrap();
    bucket.writes();
    struct Unread;
    impl Read for Unread {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("a staged attempt reads no stream")
        }
    }
    assert_eq!(
        upload_stage(&bucket, &target(1), Unread, PART_BYTES_MIN).unwrap(),
        first
    );
    assert!(bucket.writes().is_empty());
}

#[test]
fn a_long_left_list_is_cut_to_fit_the_manifest() {
    let mut manifest = Manifest {
        format: MANIFEST_FORMAT.to_string(),
        old: "proj_old1".to_string(),
        phase: Phase::P1,
        attempt: 1,
        layout: LAYOUT.to_string(),
        zip_bytes: 1,
        zip_sha256: String::new(),
        bytes: 1,
        files: 1,
        left: (0..MANIFEST_LEFT_MAX)
            .map(|index| LeftBehind {
                path: format!("workspace/{}/{index}", "deep/".repeat(80)),
                bytes: 1,
                why: "runtime".to_string(),
            })
            .collect(),
        left_more: 5,
    };
    let body = manifest_body(&mut manifest, vec![3; MANIFEST_LEFT_MAX]);
    assert!(body.len() <= MANIFEST_MAX_BYTES);
    let cut = MANIFEST_LEFT_MAX - manifest.left.len();
    assert!(cut > 0);
    assert_eq!(manifest.left_more, 5 + 3 * cut as u64);
    assert_eq!(serde_json::from_slice::<Manifest>(&body).unwrap(), manifest);
}

#[test]
fn duplicate_names_are_refused() {
    let bucket = Memory::default();
    let bytes = stage(&[
        ("workspace/a.md", b"one".to_vec()),
        ("workspace/a.md", b"two".to_vec()),
    ]);
    let error = upload_stage(&bucket, &target(1), &bytes[..], PART_BYTES_MIN).unwrap_err();
    assert!(
        matches!(&error, StageError::UnsafeName { path } if path == "workspace/a.md"),
        "{error}"
    );
    assert!(bucket.state.borrow().uploads.is_empty());
}

#[test]
fn targets_are_validated() {
    assert!(StageTarget::new(Phase::P1, "proj_1", 1).is_ok());
    let long = "p".repeat(65);
    for (old, attempt) in [
        (long.as_str(), 1),
        ("", 1),
        ("proj/1", 1),
        ("../x", 1),
        ("proj 1", 1),
        ("proj_1", 0),
    ] {
        assert!(
            StageTarget::new(Phase::P1, old, attempt).is_err(),
            "{old} {attempt}"
        );
    }
    assert_eq!(
        StageTarget::new(Phase::P2, "proj_1", 3)
            .unwrap()
            .manifest_key(),
        "p2/proj_1/3/manifest.json"
    );
}
