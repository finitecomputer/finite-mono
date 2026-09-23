//! Optional operator barrier for the destructive, isolated-cluster restore drill.
//! Core stays outside the simulated cluster failure; this adds no product API.
use super::*;
use crate::test_support::TestDb;
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
        "actors": if std::env::var_os("FC_TEST_SUBSTRATE_NODE_RESTART").is_some() {
            runtimes.iter().map(|runtime| {
                let actor = local_substrate_resource("actor", &runtime.replace('_', "-"));
                serde_json::json!({"uid": actor["metadata"]["uid"], "volumes": actor["status"]["actorVolumes"]})
            }).collect::<Vec<_>>()
        } else { Vec::new() },
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

pub(super) async fn recover_node(
    db: &TestDb,
    runtimes: &[String],
    runner: impl Fn() -> std::process::Command,
) -> bool {
    if std::env::var_os("FC_TEST_SUBSTRATE_NODE_RESTART").is_none() {
        return false;
    }
    assert_eq!(local_substrate_context(), "kind-finite-hermes-restore");
    let directory = std::path::PathBuf::from(required("FC_TEST_SUBSTRATE_RECOVERY_DIRECTORY"));
    let before: Value =
        serde_json::from_slice(&std::fs::read(directory.join("ready.json")).unwrap()).unwrap();
    assert_eq!(before["runtimes"], serde_json::json!(runtimes));
    for _ in 0..120 {
        if runtimes.iter().all(|runtime| {
            local_substrate_resource("actor", &runtime.replace('_', "-"))["status"]["state"]
                == "ACTOR_STATE_CRASHED"
        }) {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    for runtime in runtimes {
        assert_eq!(
            local_substrate_resource("actor", &runtime.replace('_', "-"))["status"]["state"],
            "ACTOR_STATE_CRASHED",
            "node restart must surface lost compute before recovery"
        );
    }
    for _ in runtimes {
        let mut command = tokio::process::Command::from(runner());
        command.env("FC_RUNNER_DRAIN", "true").kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(330), command.output())
            .await
            .expect("node recovery Runner timed out")
            .unwrap();
        assert!(
            output.status.success(),
            "ordinary Runner cycle failed after node restart"
        );
    }
    for (index, runtime) in runtimes.iter().enumerate() {
        let actor = local_substrate_resource("actor", &runtime.replace('_', "-"));
        assert_eq!(actor["status"]["state"], "ACTOR_STATE_RUNNING");
        assert_eq!(actor["metadata"]["uid"], before["actors"][index]["uid"]);
        assert_eq!(
            actor["status"]["actorVolumes"],
            before["actors"][index]["volumes"]
        );
        let requests = db.query_json("SELECT jsonb_build_object('kind',kind,'status',status,'user',requested_by_user_id) FROM runtime_control_requests WHERE agent_runtime_id=$1", &[runtime]).await;
        assert_eq!(
            requests,
            vec![serde_json::json!({"kind":"restart", "status":"succeeded", "user":null})]
        );
    }
    eprintln!(
        "both agents recovered after node restart through ordinary Runner observation under admission drain"
    );
    true
}
