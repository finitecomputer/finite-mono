//! Optional operator barrier for the destructive, isolated-cluster restore drill.
//! Core stays outside the simulated cluster failure; this adds no product API.
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;

pub(super) async fn wait_for_cluster_restore(database_url: &str, runtimes: &[String]) {
    let Some(directory) = std::env::var_os("FC_TEST_SUBSTRATE_RECOVERY_DIRECTORY") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    let resume = directory.join("resume");
    assert!(
        !resume.exists(),
        "restore barrier must use a fresh directory"
    );
    let nonce = crate::store::runtime_credentials::new_secret().unwrap();
    let context = serde_json::json!({
        "coreDatabaseUrl": database_url,
        "runtimes": runtimes,
        "resumeNonce": nonce,
    });
    let ready = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join("ready.json"))
        .unwrap();
    serde_json::to_writer(&ready, &context).unwrap();
    ready.sync_all().unwrap();
    eprintln!(
        "Recovery barrier: both native histories are verified; waiting for isolated cluster restore"
    );
    for _ in 0..900 {
        match std::fs::read_to_string(&resume) {
            Ok(value) => {
                assert_eq!(
                    value.trim(),
                    nonce,
                    "restore barrier belongs to another run"
                );
                return;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("read restore barrier: {error}"),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    panic!("isolated cluster restore did not finish within 15 minutes");
}
