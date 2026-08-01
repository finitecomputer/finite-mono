use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

fn one_response_server(
    expected_request_text: &'static str,
    body: &'static str,
) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request
                .windows(expected_request_text.len())
                .any(|window| window == expected_request_text.as_bytes())
            {
                break;
            }
            if request.len() > 32 * 1024 {
                break;
            }
        }
        sender
            .send(String::from_utf8_lossy(&request).into_owned())
            .unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    (format!("http://{address}"), receiver)
}

#[test]
fn managed_agent_nip05_in_email_flag_fails_before_sites_or_mail_delivery() {
    let managed = "clanky-02588d85a5aa5698@finite.vip";
    let body = r#"{"name":"clanky-02588d85a5aa5698@finite.vip","pubkey":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","npub":"npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6","kind":"managed_agent"}"#;
    let (identity_url, request) = one_response_server(managed, body);
    let finite_home = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_fsite"))
        .args([
            "project",
            "grant",
            "demo",
            "--email",
            managed,
            "--send-invite",
        ])
        .env("FINITE_HOME", finite_home.path())
        .env("FINITE_IDENTITY_AUTHORITY", identity_url)
        .env("FINITE_SITES_API", "http://127.0.0.1:9")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Managed Agent NIP-05"));
    assert!(stderr.contains("--nip05"));
    assert!(stderr.contains("no email was sent"));
    assert!(!stderr.contains("is finitesitesd running"));

    let request = request.recv().unwrap();
    assert!(request.starts_with("POST /api/v1/nip05-resolution "));
    assert!(request.contains(managed));
    assert!(!finite_home.path().join("identity").exists());
}
