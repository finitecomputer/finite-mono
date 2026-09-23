//! Actual dashboard onboarding and fresh-user Brain approval proof.
use super::*;

pub(super) async fn dashboard_creation(owner: &str, user: &str, code: &str) -> String {
    let input = json!({"ownerToken": owner, "displayName": user, "launchCode": code}).to_string();
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut child = std::process::Command::new("node")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../apps/dashboard/scripts/substrate-dashboard-proof.mjs"
            ))
            .arg("create")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "actual dashboard creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        if std::env::var_os("FC_TEST_SUBSTRATE_BRAIN_BINARY").is_some() {
            assert_eq!(result["brainApprovalVerified"], true);
            eprintln!("fresh-user dashboard Brain approval and replay guards passed");
        }
        result["requestId"].as_str().unwrap().to_owned()
    })
    .await
    .unwrap()
}
