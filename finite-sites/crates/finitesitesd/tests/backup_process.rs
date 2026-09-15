#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use finitesites_store::Store;

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .stdin(std::process::Stdio::null())
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn capture_with_git(wrapper: &str) -> (tempfile::TempDir, Output) {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "--initial-branch=main"]);
    let data = root.path().join("data");
    fs::create_dir(&data).unwrap();
    let mut store = Store::open(&data.join("registry.db")).unwrap();
    let project = store
        .init_project(&"1".repeat(64), "process-test", &[], 100)
        .unwrap()
        .project;
    let repo = data
        .join("git/projects")
        .join(format!("{}.git", project.id));
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--bare", "--initial-branch=main"]);
    let tree = git(&repo, &["mktree"]);
    let commit = git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit-tree",
            &tree,
            "-m",
            "source",
        ],
    );
    git(&repo, &["update-ref", "refs/heads/main", &commit]);
    store
        .create_git_credential(
            "process-credential",
            &project.id,
            &project.owner_principal_id,
            &"b".repeat(64),
            None,
            100,
        )
        .unwrap();
    let (event, _) = store
        .record_git_ref_event(
            &project.id,
            "refs/heads/main",
            &"0".repeat(40),
            &commit,
            &project.owner_principal_id,
            None,
            "process-credential",
            100,
        )
        .unwrap();
    store.mark_git_ref_event_ignored(event.id, 100).unwrap();
    fs::write(data.join("cookie-secret"), "9".repeat(64)).unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let real_git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real_git = String::from_utf8(real_git.stdout).unwrap();
    let executable = bin.join("git");
    let template = root.path().join("template");
    fs::create_dir(&template).unwrap();
    fs::write(
        template.join("unexpected-template"),
        "must not enter recovery staging",
    )
    .unwrap();
    fs::write(
        &executable,
        format!("#!/bin/sh\n{wrapper}\nexec '{}' \"$@\"\n", real_git.trim()),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_finitesitesd"))
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("BACKUP_PROCESS_TEST_ROOT", root.path())
        .env("BACKUP_PROCESS_TEST_REAL_GIT", real_git.trim())
        .env("GIT_TEMPLATE_DIR", &template)
        .args(["backup", "capture", "--data"])
        .arg(&data)
        .arg("--repository")
        .arg(root.path().join("backup"))
        .output()
        .unwrap();
    (root, output)
}

#[test]
fn excessive_git_stderr_rejects_capture_without_publishing_a_point() {
    let (root, output) = capture_with_git("dd if=/dev/zero bs=1048576 count=2 1>&2 2>/dev/null");
    assert!(
        !output.status.success(),
        "capture must reject excessive Git stderr"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("stderr exceeds limit"));
    assert_eq!(
        fs::read_dir(root.path().join("backup/points"))
            .unwrap()
            .count(),
        0
    );
}

fn assert_descendant_stopped(root: &Path) {
    let pid = fs::read_to_string(root.join("descendant")).unwrap();
    // Orphan reaping belongs to init; a zombie is already terminated and cannot
    // keep writing, consume CPU, or hold our pipes open.
    for _ in 0..100 {
        let status = Command::new("ps")
            .args(["-o", "stat=", "-p", pid.trim()])
            .output()
            .unwrap();
        let status = String::from_utf8_lossy(&status.stdout);
        if status.trim().is_empty() || status.trim().starts_with('Z') {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("Git descendant {} survived capture", pid.trim());
}

#[test]
fn excessive_git_stdout_stops_the_process_group_before_publishing() {
    let (root, output) = capture_with_git(
        "sleep 90 &\necho $! > \"$BACKUP_PROCESS_TEST_ROOT/descendant\"\nexec dd if=/dev/zero bs=65536 2>/dev/null",
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("stdout exceeds limit"));
    assert_eq!(
        fs::read_dir(root.path().join("backup/points"))
            .unwrap()
            .count(),
        0
    );
    assert_descendant_stopped(root.path());
}

#[test]
fn stalled_git_times_out_and_stops_descendants() {
    let started = Instant::now();
    let (root, output) =
        capture_with_git("sleep 90 &\necho $! > \"$BACKUP_PROCESS_TEST_ROOT/descendant\"\nwait");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(80));
    assert_eq!(
        fs::read_dir(root.path().join("backup/points"))
            .unwrap()
            .count(),
        0
    );
    assert_descendant_stopped(root.path());
}

#[test]
fn bundle_stream_is_stopped_at_the_object_limit() {
    let (root, output) = capture_with_git(
        "case \" $* \" in\n*\" bundle create \"*) exec dd if=/dev/zero bs=65536 2>/dev/null ;;\nesac",
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("stdout exceeds limit"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_dir(root.path().join("backup/points"))
            .unwrap()
            .count(),
        0
    );
    // The failed bundle and scratch repositories are removed on unwind.
    assert_eq!(fs::read_dir(root.path().join("backup")).unwrap().count(), 2);
}

#[test]
fn git_observation_does_not_discover_an_ancestor_repository() {
    let (root, output) =
        capture_with_git("case \" $* \" in\n*\" symbolic-ref HEAD \"*)\n  rm -f HEAD\n  ;;\nesac");
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(root.path().join(".git/HEAD")).unwrap(),
        "ref: refs/heads/main\n"
    );
    assert_eq!(
        fs::read_dir(root.path().join("backup/points"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn bounded_git_processes_capture_and_restore_real_history() {
    let (root, output) = capture_with_git("");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: finitesitesd::backup::Receipt = serde_json::from_slice(&output.stdout).unwrap();
    let target = root.path().join("restored");
    let restore = Command::new(env!("CARGO_BIN_EXE_finitesitesd"))
        .args(["backup", "restore", "--repository"])
        .arg(root.path().join("backup"))
        .arg("--point")
        .arg(receipt.id)
        .arg("--target")
        .arg(&target)
        .output()
        .unwrap();
    assert!(
        restore.status.success(),
        "{}",
        String::from_utf8_lossy(&restore.stderr)
    );
    let repo = fs::read_dir(target.join("git/projects"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(git(&repo, &["show", "-s", "--format=%s", "main"]), "source");
}

#[test]
fn capture_ignores_inherited_git_templates() {
    let (root, output) = capture_with_git(
        "case \" $* \" in\n*\" init \"*)\n  \"$BACKUP_PROCESS_TEST_REAL_GIT\" \"$@\" || exit $?\n  test ! -e unexpected-template\n  exit $?\n  ;;\nesac",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("template/unexpected-template")).unwrap(),
        "must not enter recovery staging"
    );
}
