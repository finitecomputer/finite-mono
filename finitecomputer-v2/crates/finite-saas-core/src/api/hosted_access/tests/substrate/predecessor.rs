//! Opt-in actual predecessor binary against current Core and Substrate backlog.
use super::*;

pub(super) async fn idle(current: std::process::Command, token: &str) {
    let Some(binary) = std::env::var_os("FC_TEST_SUBSTRATE_PREDECESSOR_RUNNER") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let mut command = tokio::process::Command::new(binary);
    command.args(current.get_args());
    for (key, value) in current.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    command
        .env("FC_CORE_RUNNER_API_TOKEN", token)
        .env("FC_RUNNER_ID", "predecessor")
        .env("FC_RUNNER_CLASS", "local_docker")
        .env("FC_RUNNER_WORK_ROOT", directory.path())
        .env("FC_RUNNER_DRAIN", "false")
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(60), command.output())
        .await
        .expect("predecessor runner did not finish")
        .unwrap();
    if !output.status.success() {
        use std::io::Write;
        let mut log = tempfile::NamedTempFile::new().unwrap();
        log.write_all(&output.stderr).unwrap();
        let (_, path) = log.keep().unwrap();
        panic!(
            "predecessor runner failed; private diagnostics: {}",
            path.display()
        );
    }
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result,
        json!({"status": "idle"}),
        "old Docker runner claimed Substrate work"
    );
    eprintln!("actual predecessor runner read current Core and left Substrate backlog unclaimed");
}
