//! Faults are restricted to the explicitly configured disposable kind cluster.
use super::*;

pub(super) async fn crash_local_worker(runtime: &str) {
    let Ok(kubeconfig) = std::env::var("FC_TEST_SUBSTRATE_CRASH_KUBECONFIG") else {
        return;
    };
    let actor = runtime.replacen("runtime_", "runtime-", 1);
    let read_actor = || local_substrate_resource("actor", &actor);
    let initial = read_actor();
    assert_eq!(initial["metadata"]["name"], actor);
    assert_eq!(initial["status"]["state"], "ACTOR_STATE_RUNNING");
    let worker = &initial["status"]["workerAssignment"];
    assert_eq!(worker["workerNamespace"], "finite-hermes-spike");
    let pod = worker["workerPod"].as_str().unwrap();
    assert!(pod.starts_with("hermes-"));
    if std::env::var_os("FC_TEST_SUBSTRATE_RESTART_SANDBOX").is_some() {
        restart_sandbox(pod, &kubeconfig);
    } else {
        let output = std::process::Command::new("kubectl")
            .args([
                "--kubeconfig",
                &kubeconfig,
                "--context",
                &local_substrate_context(),
                "--request-timeout=10s",
                "-n",
                "finite-hermes-spike",
                "delete",
                "pod",
                pod,
                "--wait=false",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "delete only the synthetic actor's worker pod"
        );
    }
    for _ in 0..120 {
        let current = read_actor();
        assert_eq!(current["metadata"]["uid"], initial["metadata"]["uid"]);
        if current["status"]["state"] == "ACTOR_STATE_CRASHED" {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    panic!("worker loss did not reach CRASHED");
}

fn restart_sandbox(pod: &str, kubeconfig: &str) {
    assert_eq!(local_substrate_context(), "kind-finite-hermes-restore");
    let output = std::process::Command::new("kubectl")
        .args([
            "--kubeconfig",
            kubeconfig,
            "--context",
            "kind-finite-hermes-restore",
            "-n",
            "finite-hermes-spike",
            "get",
            "pod",
            pod,
            "-o",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "read exact worker pod");
    let worker: Value = serde_json::from_slice(&output.stdout).unwrap();
    let uid = worker["metadata"]["uid"].as_str().unwrap();
    assert_eq!(
        worker["spec"]["nodeName"],
        "finite-hermes-restore-control-plane"
    );
    let output = std::process::Command::new("docker")
        .args([
            "exec",
            "finite-hermes-restore-control-plane",
            "crictl",
            "pods",
            "-o",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "read local CRI sandboxes");
    let inventory: Value = serde_json::from_slice(&output.stdout).unwrap();
    let sandboxes: Vec<_> = inventory["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|sandbox| sandbox["metadata"]["uid"] == uid && sandbox["state"] == "SANDBOX_READY")
        .collect();
    assert_eq!(
        sandboxes.len(),
        1,
        "exactly one live sandbox for the selected worker UID"
    );
    let id = sandboxes[0]["id"].as_str().unwrap();
    let output = std::process::Command::new("docker")
        .args([
            "exec",
            "finite-hermes-restore-control-plane",
            "crictl",
            "stopp",
            id,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stop only the selected worker's CRI sandbox"
    );
    eprintln!(
        "restarted worker sandbox for {pod} UID {uid}; previous IP {}",
        worker["status"]["podIP"]
    );
}
