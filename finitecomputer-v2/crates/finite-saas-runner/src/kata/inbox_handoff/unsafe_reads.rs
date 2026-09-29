use super::*;
use std::os::unix::fs::symlink;
use std::sync::mpsc;

const PROTECTED: &str = r#"{"events":[{"lease":{"state":"refusal_v1"}}]}"#;
const HOST_SECRET: &str = "HOST-ONLY-SECRET root:x:0:0";

fn chat_home(state_root: &Path) -> PathBuf {
    let home = state_root.join("agent");
    std::fs::create_dir_all(&home).unwrap();
    home
}

fn assert_fixed_refusal(error: RunnerError, reason: &str) {
    let message = error.to_string();
    assert!(
        message.contains("chat inbox compatibility not established")
            && message.contains(reason)
            && message.contains("inbox not modified"),
        "{message}"
    );
    assert!(!message.contains("HOST-ONLY-SECRET"), "{message}");
    assert!(!message.contains("line "), "{message}");
}

#[test]
fn absent_inbox_is_compatible_at_every_level() {
    let temp = tempfile::tempdir().unwrap();
    let state_root = temp.path().join("state");
    assert!(inbox(&state_root).unwrap().is_none());
    std::fs::create_dir_all(&state_root).unwrap();
    assert!(inbox(&state_root).unwrap().is_none());
    chat_home(&state_root);
    assert!(inbox(&state_root).unwrap().is_none());
}

#[test]
fn regular_inbox_is_read_through_a_host_symlinked_state_root() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real-state");
    std::fs::write(chat_home(&real).join("hermes-inbox.json"), PROTECTED).unwrap();
    let state_root = temp.path().join("state");
    symlink(&real, &state_root).unwrap();

    let bytes = inbox(&state_root).unwrap().unwrap();
    assert_eq!(bytes, PROTECTED.as_bytes());
    assert!(inbox_compatibility::check(&bytes, None).is_err());
}

#[test]
fn leaf_symlink_is_refused_without_reading_its_target() {
    let temp = tempfile::tempdir().unwrap();
    let host_file = temp.path().join("host-secret");
    std::fs::write(&host_file, HOST_SECRET).unwrap();
    let state_root = temp.path().join("state");
    let home = chat_home(&state_root);
    symlink(&host_file, home.join("hermes-inbox.json")).unwrap();
    assert_fixed_refusal(inbox(&state_root).unwrap_err(), "not a regular file");

    // A dangling link is guest-planted state, not an absent inbox.
    std::fs::remove_file(home.join("hermes-inbox.json")).unwrap();
    symlink(temp.path().join("missing"), home.join("hermes-inbox.json")).unwrap();
    assert_fixed_refusal(inbox(&state_root).unwrap_err(), "not a regular file");
}

#[test]
fn parent_symlink_is_refused_without_traversing_it() {
    let temp = tempfile::tempdir().unwrap();
    let host_dir = temp.path().join("host-dir");
    std::fs::create_dir_all(&host_dir).unwrap();
    std::fs::write(host_dir.join("hermes-inbox.json"), HOST_SECRET).unwrap();
    let state_root = temp.path().join("state");
    std::fs::create_dir_all(&state_root).unwrap();
    symlink(&host_dir, state_root.join("agent")).unwrap();
    assert_fixed_refusal(
        inbox(&state_root).unwrap_err(),
        "chat home is not a directory",
    );

    std::fs::remove_file(state_root.join("agent")).unwrap();
    symlink(temp.path().join("missing"), state_root.join("agent")).unwrap();
    assert_fixed_refusal(
        inbox(&state_root).unwrap_err(),
        "chat home is not a directory",
    );
}

#[test]
fn fifo_and_directory_inboxes_are_refused_promptly() {
    let temp = tempfile::tempdir().unwrap();
    let state_root = temp.path().join("state");
    let fifo = chat_home(&state_root).join("hermes-inbox.json");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(status.success());

    // No writer ever opens a FIFO; a blocking open would never return.
    let prompt_error = |root: PathBuf| {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || sender.send(inbox(&root).map(|_| ())).unwrap());
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("inbox read blocked on a FIFO")
            .unwrap_err()
    };
    assert_fixed_refusal(prompt_error(state_root.clone()), "not a regular file");

    std::fs::remove_file(&fifo).unwrap();
    std::fs::create_dir(&fifo).unwrap();
    assert_fixed_refusal(inbox(&state_root).unwrap_err(), "not a regular file");

    std::fs::remove_dir_all(state_root.join("agent")).unwrap();
    let status = std::process::Command::new("mkfifo")
        .arg(state_root.join("agent"))
        .status()
        .unwrap();
    assert!(status.success());
    assert_fixed_refusal(prompt_error(state_root), "chat home is not a directory");
}

#[test]
fn regular_inbox_over_the_bound_is_refused_and_the_bound_itself_is_read() {
    let temp = tempfile::tempdir().unwrap();
    let state_root = temp.path().join("state");
    let path = chat_home(&state_root).join("hermes-inbox.json");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(MAX_INBOX_BYTES).unwrap();
    assert_eq!(
        inbox(&state_root).unwrap().unwrap().len() as u64,
        MAX_INBOX_BYTES
    );

    file.set_len(MAX_INBOX_BYTES + 1).unwrap();
    assert_fixed_refusal(inbox(&state_root).unwrap_err(), "64 MiB handoff bound");
}
