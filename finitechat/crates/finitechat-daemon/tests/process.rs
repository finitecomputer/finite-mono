use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use finitechat_core::device_link::{DeviceLinkEncryptInput, encrypt_device_link_payload};
use finitechat_http::{
    GetLinkSessionRequest, HttpLinkSessionRecord, HttpLinkSessionState, UploadLinkPayloadRequest,
};
use finitechat_server::{HttpServerState, http_router};
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

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn link_process_keeps_the_account_secret_on_fd3_until_fd4_confirms_storage() {
    let root = TempDir::new().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, http_router(HttpServerState::default()))
            .await
            .unwrap();
    });

    let result_path = root.path().join("link-result.json");
    let confirmation_path = root.path().join("storage-confirmation.pipe");
    assert!(
        Command::new("mkfifo")
            .arg(&confirmation_path)
            .status()
            .unwrap()
            .success()
    );
    let mut child = Command::new("/bin/sh")
        .args([
            "-c",
            r#"exec 3>"$FINITECHAT_TEST_RESULT"
exec 4<"$FINITECHAT_TEST_CONFIRM"
exec "$FINITECHAT_TEST_BIN" link \
  --server-url "$FINITECHAT_TEST_SERVER_URL" \
  --dashboard-url https://finite.test \
  --device-id electron-process-link \
  --result-fd 3 \
  --confirm-fd 4 \
  --timeout-seconds 10"#,
        ])
        .env("FINITECHAT_TEST_BIN", env!("CARGO_BIN_EXE_finitechatd"))
        .env("FINITECHAT_TEST_SERVER_URL", &server_url)
        .env("FINITECHAT_TEST_RESULT", &result_path)
        .env("FINITECHAT_TEST_CONFIRM", &confirmation_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Opening the FIFO writer releases the child's fd4 open. Keep it open but
    // do not confirm storage until fd3 has actually received the secret.
    let mut confirmation = fs::OpenOptions::new()
        .write(true)
        .open(&confirmation_path)
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (line_sender, line_receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if line_sender.send(line).is_err() {
                break;
            }
        }
    });

    let ready_line = line_receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("link process did not emit public readiness")
        .expect("could not read link readiness");
    let ready: Value = serde_json::from_str(&ready_line).unwrap();
    assert_eq!(ready["event"], "link_ready");
    assert_eq!(ready["target_device_id"], "electron-process-link");
    assert!(
        ready["approval_url"]
            .as_str()
            .unwrap()
            .starts_with("https://finite.test/dashboard/device-link?")
    );
    assert!(!ready_line.contains(ACCOUNT_SECRET));
    assert!(ready.get("pairing_secret_key").is_none());

    let link_session_id = ready["link_session_id"].as_str().unwrap().to_owned();
    let client = reqwest::Client::new();
    let created: Option<HttpLinkSessionRecord> = client
        .post(format!("{server_url}/link-sessions/get"))
        .json(&GetLinkSessionRequest {
            link_session_id: link_session_id.clone(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let created = created.unwrap();
    assert_eq!(created.state, HttpLinkSessionState::Created);
    let issued_at_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let encrypted_payload = encrypt_device_link_payload(DeviceLinkEncryptInput {
        account_secret_hex: ACCOUNT_SECRET.to_owned(),
        pairing_public_key: created.pairing_public_key,
        link_session_id: link_session_id.clone(),
        target_device_id: "electron-process-link".to_owned(),
        server_url: server_url.clone(),
        issued_at_unix_seconds,
        expires_at_unix_seconds: issued_at_unix_seconds + 30,
    })
    .unwrap();
    client
        .post(format!("{server_url}/link-sessions/payload"))
        .json(&UploadLinkPayloadRequest {
            link_session_id: link_session_id.clone(),
            encrypted_payload,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let private_line = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(line) = fs::read_to_string(&result_path)
                && !line.is_empty()
            {
                break line;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("link process did not deliver the private fd3 result");
    let private_result: Value = serde_json::from_str(&private_line).unwrap();
    assert_eq!(private_result["account_secret"], ACCOUNT_SECRET);
    assert_eq!(private_result.as_object().unwrap().len(), 1);
    let awaiting_storage: Option<HttpLinkSessionRecord> = client
        .post(format!("{server_url}/link-sessions/get"))
        .json(&GetLinkSessionRequest {
            link_session_id: link_session_id.clone(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        awaiting_storage.unwrap().state,
        HttpLinkSessionState::Claimed,
        "fd3 delivery must not acknowledge the link before fd4 confirms durable storage"
    );

    confirmation.write_all(b"stored\n").unwrap();
    confirmation.flush().unwrap();
    drop(confirmation);
    let linked_line = line_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("link process did not emit completion after fd4 confirmation")
        .expect("could not read link completion");
    assert_eq!(
        serde_json::from_str::<Value>(&linked_line).unwrap()["event"],
        "linked"
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("link process did not exit after durable-storage confirmation");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success());
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(!ready_line.contains(ACCOUNT_SECRET));
    assert!(!linked_line.contains(ACCOUNT_SECRET));
    assert!(!stderr.contains(ACCOUNT_SECRET));

    let delivered: Option<HttpLinkSessionRecord> = client
        .post(format!("{server_url}/link-sessions/get"))
        .json(&GetLinkSessionRequest { link_session_id })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(delivered.unwrap().state, HttpLinkSessionState::Delivered);
    server.abort();
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
