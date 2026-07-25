use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;

const TOKEN: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
const ACCOUNT_SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000004";

#[test]
fn process_binds_ephemeral_loopback_and_emits_only_a_nonsecret_ready_record() {
    let root = TempDir::new().unwrap();
    let mut child = spawn_daemon(&root, "127.0.0.1:0");
    let line = read_ready_line(&mut child);
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["event"], "ready");
    let url = ready["url"].as_str().unwrap();
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    assert!(!url.ends_with(":0"));
    assert!(!line.contains(TOKEN));
    assert!(!line.contains(ACCOUNT_SECRET));

    let unauthorized = reqwest::blocking::get(format!("{url}/v1/healthz")).unwrap();
    assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);
    let authorized = reqwest::blocking::Client::new()
        .get(format!("{url}/v1/healthz"))
        .bearer_auth(TOKEN)
        .send()
        .unwrap();
    assert!(authorized.status().is_success());

    child.kill().unwrap();
    let _ = child.wait();
}

#[test]
fn process_rejects_nonloopback_before_opening_the_store_or_echoing_secrets() {
    let root = TempDir::new().unwrap();
    let child = spawn_daemon(&root, "0.0.0.0:0");
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains("\"event\":\"ready\""));
    assert!(stderr.contains("daemon bind address must be loopback"));
    assert!(!stderr.contains(TOKEN));
    assert!(!stderr.contains(ACCOUNT_SECRET));
    assert!(!root.path().join("store/client.sqlite3").exists());
}

fn spawn_daemon(root: &TempDir, bind: &str) -> Child {
    let mut child = Command::new(env!("CARGO_BIN_EXE_finitechatd"))
        .args([
            "--bind",
            bind,
            "--data-dir",
            root.path().join("store").to_str().unwrap(),
            "--server-url",
            "http://127.0.0.1:9",
            "--device-id",
            "electron-process-test",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let startup = serde_json::json!({
        "auth_token": TOKEN,
        "account_secret": ACCOUNT_SECRET,
    });
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{startup}").unwrap();
    drop(stdin);
    child
}

fn read_ready_line(child: &mut Child) -> String {
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("daemon did not emit a ready record")
        .expect("failed to read daemon ready record")
}
