#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use finitesites_blob::BlobStore;
use finitesites_proto::{hex, limits::MAX_FILE_BYTES};
use finitesites_store::Store;
use finitesitesd::backup::Receipt;
use sha2::{Digest, Sha256};

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_finitesitesd"))
}

fn capture(root: &Path, count: u32) -> (PathBuf, Receipt, Vec<PathBuf>) {
    let data = root.join("source");
    fs::create_dir(&data).unwrap();
    let mut store = Store::open(&data.join("registry.db")).unwrap();
    let blobs = BlobStore::open(&data.join("blobs")).unwrap();
    let mut expected = vec![PathBuf::from("registry.db"), PathBuf::from("cookie-secret")];
    for index in 0..count {
        let bytes = format!("restore publication payload {index}").into_bytes();
        let hash = hex::encode(&Sha256::digest(&bytes));
        blobs.put(&hash, &bytes, MAX_FILE_BYTES).unwrap();
        store.record_blob(&hash, bytes.len() as u64, 100).unwrap();
        expected.push(PathBuf::from(format!(
            "blobs/{}/{}/{hash}",
            &hash[..2],
            &hash[2..4]
        )));
    }
    fs::write(data.join("cookie-secret"), hex::encode(&[9; 32])).unwrap();
    let repository = root.join("backup");
    let output = command()
        .args(["backup", "capture", "--data"])
        .arg(&data)
        .arg("--repository")
        .arg(&repository)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt = serde_json::from_slice(&output.stdout).unwrap();
    (repository, receipt, expected)
}

fn restore(repository: &Path, receipt: &Receipt, target: &Path) -> Command {
    let mut cmd = command();
    cmd.args(["backup", "restore", "--repository"])
        .arg(repository)
        .args(["--point", &receipt.id, "--target"])
        .arg(target);
    cmd
}

fn wait_bounded(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("operator process timed out");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn operator_restore_publishes_the_complete_tree_at_once() {
    let root = tempfile::tempdir().unwrap();
    let (repository, receipt, expected) = capture(root.path(), 512);
    let target = root.path().join("restored");
    let mut child = restore(&repository, &receipt, &target)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut partial = false;
    let mut timed_out = false;
    loop {
        if target.exists() {
            partial = expected.iter().any(|path| !target.join(path).is_file());
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            child.kill().unwrap();
            break;
        }
        std::thread::yield_now();
    }
    let status = wait_bounded(&mut child);
    assert!(!timed_out, "restore timed out");
    assert!(status.success());
    assert!(
        !partial,
        "destination was visible before all restored files existed"
    );
    assert!(expected.iter().all(|path| target.join(path).is_file()));
    assert!(!target.join("git-bundles").exists());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn operator_restore_preserves_existing_files_directories_and_dangling_symlinks() {
    use std::os::unix::fs::{MetadataExt, symlink};

    let root = tempfile::tempdir().unwrap();
    let (repository, receipt, _) = capture(root.path(), 1);
    let empty = root.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let populated = root.path().join("populated");
    fs::create_dir(&populated).unwrap();
    fs::write(populated.join("keep"), b"existing content").unwrap();
    let file = root.path().join("file");
    fs::write(&file, b"existing file").unwrap();
    let dangling = root.path().join("dangling");
    let missing = root.path().join("missing");
    symlink(&missing, &dangling).unwrap();
    for target in [&empty, &populated, &file, &dangling] {
        let before = fs::symlink_metadata(target).unwrap();
        let output = restore(&repository, &receipt, target).output().unwrap();
        assert!(!output.status.success());
        assert_eq!(fs::symlink_metadata(target).unwrap().ino(), before.ino());
    }
    assert_eq!(fs::read_dir(&empty).unwrap().count(), 0);
    assert_eq!(
        fs::read(populated.join("keep")).unwrap(),
        b"existing content"
    );
    assert_eq!(fs::read(&file).unwrap(), b"existing file");
    assert_eq!(fs::read_link(&dangling).unwrap(), missing);
    assert!(!missing.exists());
}

#[test]
fn concurrent_operator_restores_have_exactly_one_complete_winner() {
    let root = tempfile::tempdir().unwrap();
    let (repository, receipt, expected) = capture(root.path(), 32);
    let target = root.path().join("restored");
    let mut first = restore(&repository, &receipt, &target)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut second = restore(&repository, &receipt, &target)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let first_status = wait_bounded(&mut first);
    let second_status = wait_bounded(&mut second);
    assert_ne!(
        first_status.success(),
        second_status.success(),
        "exactly one restore must succeed"
    );
    for path in expected {
        assert!(target.join(&path).is_file());
        if path != Path::new("registry.db") {
            assert_eq!(
                fs::read(target.join(&path)).unwrap(),
                fs::read(root.path().join("source").join(&path)).unwrap(),
                "restored file differs: {}",
                path.display()
            );
        }
    }
    let replay = restore(&repository, &receipt, &target).output().unwrap();
    assert!(!replay.status.success());
}

#[test]
fn interrupted_operator_restore_leaves_no_target_and_retry_completes() {
    use std::os::unix::process::ExitStatusExt;

    let root = tempfile::tempdir().unwrap();
    let (repository, receipt, expected) = capture(root.path(), 128);
    let restore_parent = root.path().join("destinations");
    fs::create_dir(&restore_parent).unwrap();
    let target = restore_parent.join("restored");
    let mut child = restore(&repository, &receipt, &target)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let observed_staging = loop {
        if child.try_wait().unwrap().is_some() || Instant::now() >= deadline {
            break false;
        }
        let staged = fs::read_dir(&restore_parent).unwrap().any(|entry| {
            let path = entry.unwrap().path();
            path != target && path.join("registry.db").is_file()
        });
        if staged {
            break true;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    // Child::kill sends SIGKILL on Unix; always reap before any assertion.
    let killed = child.kill();
    let status = wait_bounded(&mut child);
    assert!(
        observed_staging,
        "did not observe staged restore files before deadline"
    );
    assert!(killed.is_ok());
    assert_eq!(status.signal(), Some(9));
    assert!(
        !target.exists(),
        "interrupted staging published a destination"
    );

    let mut retry = restore(&repository, &receipt, &target)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    assert!(wait_bounded(&mut retry).success());
    assert!(expected.iter().all(|path| target.join(path).is_file()));
    assert_eq!(
        fs::read(target.join("cookie-secret")).unwrap(),
        hex::encode(&[9; 32]).as_bytes()
    );
    // TempDir removes the interrupted staging tree after both children exit.
}

#[test]
fn capture_rejects_git_refs_that_exceed_restore_depth() {
    use finitesites_proto::limits::MAX_BACKUP_RESTORE_TREE_DEPTH;

    let root = tempfile::tempdir().unwrap();
    let (repository, _, _) = capture(root.path(), 0);
    let data = root.path().join("source");
    let mut store = Store::open(&data.join("registry.db")).unwrap();
    let project = store
        .init_project(&"1".repeat(64), "deep-refs", &[], 100)
        .unwrap()
        .project;
    store
        .create_git_credential(
            "deep-credential",
            &project.id,
            &project.owner_principal_id,
            &"b".repeat(64),
            None,
            100,
        )
        .unwrap();
    let repo = data
        .join("git/projects")
        .join(format!("{}.git", project.id));
    fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(&repo)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Restore test")
            .env("GIT_AUTHOR_EMAIL", "restore@example.com")
            .env("GIT_COMMITTER_NAME", "Restore test")
            .env("GIT_COMMITTER_EMAIL", "restore@example.com")
            .stdin(Stdio::null())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    git(&["init", "--bare", "--initial-branch=main"]);
    let tree = git(&["mktree"]);
    let commit = git(&["commit-tree", &tree, "-m", "deep ref"]);
    let reference = format!(
        "refs/heads/{}leaf",
        "d/".repeat(MAX_BACKUP_RESTORE_TREE_DEPTH as usize)
    );
    git(&["update-ref", &reference, &commit]);
    let (event, _) = store
        .record_git_ref_event(
            &project.id,
            &reference,
            &"0".repeat(40),
            &commit,
            &project.owner_principal_id,
            None,
            "deep-credential",
            100,
        )
        .unwrap();
    store.mark_git_ref_event_ignored(event.id, 100).unwrap();
    let output = command()
        .args(["backup", "capture", "--data"])
        .arg(&data)
        .arg("--repository")
        .arg(&repository)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Git ref exceeds restore depth"));
    assert_eq!(fs::read_dir(repository.join("points")).unwrap().count(), 1);
}
