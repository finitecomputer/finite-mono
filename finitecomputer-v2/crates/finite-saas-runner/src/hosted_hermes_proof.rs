//! Entry point for the disposable Linux process proof. This is compiled only
//! into the test executable, never into Runner's product command surface.
use super::*;
use crate::CoreHttpAgentCreationQueue;
use crate::hosted_hermes_lifecycle::{HostedHermesConfig, HostedHermesLifecycle};
use std::sync::Arc;

#[derive(serde::Deserialize)]
struct Job {
    action: String,
    name: String,
    runtime: String,
    root: PathBuf,
    nerdctl: PathBuf,
    core_url: String,
    core_token: String,
    network: String,
}

#[test]
#[ignore = "requires root in a disposable systemd/containerd fixture; run scripts/proofs/hosted-hermes-process-lifetime.py"]
fn systemd_lifecycle() {
    let path = std::env::var_os("FIN91_PROOF_JOB").expect("proof job file");
    let job: Job = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let hosted = Arc::new(
        HostedHermesLifecycle::new(
            HostedHermesConfig {
                public_origin: "https://localhost:34443".into(),
                listen: "127.0.0.1:34443".parse().unwrap(),
                allowed_origins: vec!["http://localhost:3000".into()],
            },
            job.nerdctl.clone(),
            "finite".into(),
            "proof-host".into(),
            job.root.clone(),
            CoreHttpAgentCreationQueue::new(job.core_url, job.core_token).unwrap(),
        )
        .unwrap(),
    );
    let launcher = KataLauncher::new(KataConfig {
        nerdctl_bin: job.nerdctl,
        namespace: "finite".into(),
        work_root: job.root.clone(),
        source_host_id: "proof-host".into(),
        hosted_hermes: Some(hosted.clone()),
        stop_timeout_secs: 1,
        command_timeout: Duration::from_secs(30),
        ..KataConfig::default()
    });
    match job.action.as_str() {
        "create" => {
            let root = job.root.join("kata").join(&job.runtime);
            std::fs::create_dir_all(root.join("api")).unwrap();
            std::fs::write(root.join("api/status"), &job.runtime).unwrap();
            let command = launcher.command(vec![
                "run".into(),
                "-d".into(),
                "--name".into(),
                job.name.into(),
                "--network".into(),
                job.network.into(),
                "--restart".into(),
                "unless-stopped".into(),
                "--label".into(),
                "computer.finite.v2.runtime=true".into(),
                "--label".into(),
                "computer.finite.v2.source_host_id=proof-host".into(),
                "--label".into(),
                format!("computer.finite.v2.source_machine_id={}", job.runtime).into(),
                "--label".into(),
                "computer.finite.v2.project_id=proof-project".into(),
                "--volume".into(),
                format!("{}:/data", root.display()).into(),
                "finite-hosted-lifecycle-proof:fixture".into(),
                "/bin/httpd".into(),
                "-f".into(),
                "-p".into(),
                "8642".into(),
                "-h".into(),
                "/data".into(),
            ]);
            launcher
                .run_checked(command, Duration::from_secs(30))
                .unwrap();
            hosted.reconcile().unwrap();
        }
        "stop" => launcher.stop_compute(&job.name).unwrap(),
        "start" => launcher.start_compute(&job.name).unwrap(),
        "remove" => launcher.remove_compute(&job.name).unwrap(),
        "rename" => launcher.rename_compute(&job.name, &job.runtime).unwrap(),
        "binding" => assert!(hosted.has_native_binding(&job.name).unwrap()),
        "reconcile" => hosted.reconcile().unwrap(),
        "failed-mutation" => {
            assert!(launcher.start_compute("deliberately-missing").is_err());
            assert!(launcher.start_compute(&job.name).is_err());
            assert!(hosted.reconcile().is_err());
            assert!(Path::new("/run/finite-hosted-hermes/mutation-in-progress").exists());
        }
        "interrupted-remove" => {
            let marker = job.root.join("at-remove");
            std::thread::spawn(move || launcher.remove_compute(&job.name).unwrap());
            let started = Instant::now();
            while !marker.exists() {
                assert!(started.elapsed() < Duration::from_secs(20));
                std::thread::sleep(Duration::from_millis(20));
            }
            // Simulate main-process exit with a provider child still running;
            // skips Drop just like a crash. PID1 must retain the invocation.
            std::process::exit(0);
        }
        other => panic!("unknown fixture action: {other}"),
    }
}
