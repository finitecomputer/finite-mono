//! The stage's pure parts. Uploads, resumes, stale attempts and refusals
//! run against a fake finite.chat in `tests/finite_chat.rs`.

use super::*;

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
        left_more: Some(5),
    };
    fit_manifest(&mut manifest, vec![3; MANIFEST_LEFT_MAX]);
    let body = serde_json::to_vec(&manifest).unwrap();
    assert!(body.len() <= MANIFEST_MAX_BYTES);
    let cut = MANIFEST_LEFT_MAX - manifest.left.len();
    assert!(cut > 0);
    assert_eq!(manifest.left_more, Some(5 + 3 * cut as u64));
}

#[test]
fn nothing_left_behind_has_no_left_fields() {
    let manifest = Manifest {
        format: MANIFEST_FORMAT.to_string(),
        old: "proj_old1".to_string(),
        phase: Phase::P2,
        attempt: 2,
        layout: LAYOUT.to_string(),
        zip_bytes: 1,
        zip_sha256: "00".to_string(),
        bytes: 1,
        files: 1,
        left: Vec::new(),
        left_more: None,
    };
    assert_eq!(
        serde_json::to_value(&manifest).unwrap(),
        serde_json::json!({
            "format": "finite-stage/1", "old": "proj_old1", "phase": "p2", "attempt": 2,
            "layout": "finite-computer", "zip_bytes": 1, "zip_sha256": "00",
            "bytes": 1, "files": 1,
        })
    );
}

#[test]
fn targets_are_validated() {
    assert!(StageTarget::new(Phase::P1, "proj_1", 1).is_ok());
    let long = "p".repeat(65);
    for (old, attempt) in [
        (long.as_str(), 1),
        ("", 1),
        ("proj/1", 1),
        ("proj.1", 1),
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
        StageTarget::new(Phase::P2, "proj_1", 3).unwrap().stage_id(),
        "proj_1.p2.3"
    );
}

/// A stage whose central directory renames an entry the tail no longer
/// holds: only the comparison with what streamed past can see it.
#[test]
fn a_directory_that_renames_an_entry_beyond_the_tail_is_refused() {
    use finite_mover_host::zip64::ZipStream;

    let big = vec![7u8; 64 * 1024];
    let summary = br#"{"left":[],"left_more":{"files":0,"bytes":0}}"#;
    let mut stream = ZipStream::new(Vec::new());
    stream
        .entry(
            "workspace/big.bin",
            0o644,
            0,
            big.len() as u64,
            &mut &big[..],
        )
        .unwrap();
    stream
        .entry(
            SUMMARY_ENTRY,
            0o644,
            0,
            summary.len() as u64,
            &mut &summary[..],
        )
        .unwrap();
    let good = stream.finish().unwrap();
    let directory = good.len()
        - good
            .windows(4)
            .rev()
            .position(|window| window == 0x0201_4b50u32.to_le_bytes())
            .unwrap();
    // The last directory record is the summary's; the one before it names
    // the big entry.
    let name = good[..directory]
        .windows(17)
        .rposition(|window| window == b"workspace/big.bin")
        .unwrap();
    let mut renamed = good.clone();
    renamed[name..name + 17].copy_from_slice(b"workspace/big.env");
    let sent = |bytes: &[u8]| {
        let mut scanner = Scanner::default();
        scanner.feed(bytes).unwrap();
        let mut tail = Tail::new(4 * 1024);
        tail.push(bytes);
        Sent {
            scanned: scanner.finish().unwrap().0,
            tail,
            sha256: String::new(),
        }
    };
    assert!(check(&sent(&good)).is_ok());
    assert!(matches!(
        check(&sent(&renamed)),
        Err(StageError::DirectoryMismatch)
    ));
}

/// A finite.chat that takes each part and completes a different size.
struct Lies {
    part_bytes: u64,
}

impl StageSink for Lies {
    fn parts(&self, _: &str) -> Result<Vec<crate::sink::Part>, SinkError> {
        Ok(Vec::new())
    }

    fn put_part(
        &self,
        _: &str,
        n: u32,
        body: &[u8],
        sha256: &str,
    ) -> Result<crate::sink::Part, SinkError> {
        Ok(crate::sink::Part {
            n,
            bytes: self.part_bytes.min(body.len() as u64),
            sha256: sha256.to_string(),
            etag: String::new(),
        })
    }

    fn complete(&self, _: &str, manifest: &Manifest) -> Result<Completed, SinkError> {
        Ok(Completed {
            manifest_sha256: String::new(),
            zip_bytes: manifest.zip_bytes + 1,
        })
    }
}

#[test]
fn an_answer_that_does_not_match_what_was_sent_is_an_error() {
    use finite_mover_host::zip64::ZipStream;

    let summary = br#"{"left":[],"left_more":{"files":0,"bytes":0}}"#;
    let mut stream = ZipStream::new(Vec::new());
    stream
        .entry(
            SUMMARY_ENTRY,
            0o644,
            0,
            summary.len() as u64,
            &mut &summary[..],
        )
        .unwrap();
    let bytes = stream.finish().unwrap();
    let locks = tempfile::tempdir().unwrap();
    let target = StageTarget::new(Phase::P1, "proj_1", 1).unwrap();
    let error = upload_stage(
        &Lies { part_bytes: 1 },
        &target,
        &bytes[..],
        PART_BYTES_MIN,
        locks.path(),
    )
    .unwrap_err();
    assert!(
        matches!(error, StageError::Sink(SinkError::Unreadable { .. })),
        "{error}"
    );
    let error = upload_stage(
        &Lies {
            part_bytes: u64::MAX,
        },
        &target,
        &bytes[..],
        PART_BYTES_MIN,
        locks.path(),
    )
    .unwrap_err();
    assert!(
        matches!(error, StageError::CompletedMismatch { .. }),
        "{error}"
    );
}
