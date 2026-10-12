use super::*;

fn archive_fixture(
    temp: &tempfile::TempDir,
) -> (KataLauncher, KataLaunchPlan, PathBuf, RuntimeControlLease) {
    let (mut launcher, plan, fake) = test_launcher(temp, 41234);
    configure_test_retirement(&mut launcher, temp.path());
    launcher.config.retirement.as_mut().unwrap().repository =
        "ssh://archive.example/./repository".into();
    stage_relocation_tree(&plan.state_root);
    write_fake_container(
        &fake,
        &plan.container_name,
        RELOCATION_TEST_IMAGE,
        "artifact-v1",
        "",
        41234,
        &plan.state_root,
    );
    std::fs::write(
        fake.join(format!("{}.status", plan.container_name)),
        "exited",
    )
    .unwrap();
    let mut lease = recovery_lease("runtime_ctl_trial_archive", RELOCATION_TEST_IMAGE, 41234);
    lease.request.kind = RuntimeControlKind::ArchiveTrial;
    lease.archive_principal = Some("npub1sameagent".into());
    let RuntimeSpecEnvelope::V1(spec) = lease.runtime_spec.as_mut().unwrap();
    spec.boot_intent = RuntimeBootIntent::Normal;
    (launcher, plan, fake, lease)
}

fn reclaim_lease(
    archive: &RuntimeControlLease,
    snapshot: finite_saas_core::TrialArchiveSnapshot,
) -> RuntimeControlLease {
    let mut reclaim = archive.clone();
    reclaim.request.id = "runtime_ctl_trial_reclaim".into();
    reclaim.request.kind = RuntimeControlKind::ReclaimTrial;
    let RuntimeSpecEnvelope::V1(spec) = reclaim.runtime_spec.as_mut().unwrap();
    spec.operation_id = reclaim.request.id.clone();
    reclaim.trial_archive = Some(snapshot);
    reclaim
}

#[test]
fn trial_archive_reclaim_restore_preserves_identity_history_and_boot_changes() {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, fake, archive) = archive_fixture(&temp);
    let snapshot = launcher
        .archive_trial_state(&archive, &mut || Ok(()))
        .unwrap();
    assert!(plan.state_root.exists());
    assert!(launcher.inspect(&plan.container_name).unwrap().is_some());
    assert!(temp.path().join("borg-remote/archive.zip").exists());
    assert!(
        !temp
            .path()
            .join("retirement-staging")
            .join(&archive.request.id)
            .exists()
    );
    let reclaim = reclaim_lease(&archive, snapshot.clone());
    launcher
        .reclaim_trial_state(&reclaim, &mut || Ok(()))
        .unwrap();
    assert!(!plan.state_root.exists());
    assert!(launcher.inspect(&plan.container_name).unwrap().is_none());
    assert_eq!(active_kata_container_count(&launcher.config), Some(0));
    launcher
        .reclaim_trial_state(&reclaim, &mut || Ok(()))
        .unwrap();
    let mut restore = relocation_creation_lease(
        "agent_request_trial_restore",
        "finite-lat-1",
        "finite-lat-1",
        &snapshot.durable_state_manifest_sha256,
        true,
    );
    let finite_saas_core::RuntimeRelocationEnvelope::V1(relocation) =
        restore.request.relocation.as_mut().unwrap();
    relocation.trial_archive = Some(snapshot);
    launcher
        .restore_trial_state(&restore, &plan, &mut || Ok(()))
        .unwrap();
    assert_eq!(
        std::fs::read(plan.state_root.join("agent/life")).unwrap(),
        b"everything"
    );
    assert_eq!(
        std::fs::read(plan.state_root.join("agent/identity/identity.json")).unwrap(),
        b"{\"npub\":\"npub1sameagent\"}\n"
    );
    // Crash after boot, before Core completion. Never restore the stale archive
    // over writes made by this operation's already-restored Agent.
    std::fs::write(plan.state_root.join("agent/life"), "new history after boot").unwrap();
    write_fake_container(
        &fake,
        &plan.container_name,
        RELOCATION_TEST_IMAGE,
        "artifact-v1",
        "",
        41234,
        &plan.state_root,
    );
    launcher
        .restore_trial_state(&restore, &plan, &mut || Ok(()))
        .unwrap();
    launcher
        .verify_relocation_state(
            &plan,
            &restore,
            restore.request.relocation.as_ref().unwrap().v1(),
        )
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(plan.state_root.join("agent/life")).unwrap(),
        "new history after boot"
    );
    assert!(
        launcher
            .pause_trial_restore_state(&restore, &plan, &mut || Err(RunnerError::CoreRequest(
                "stale worker after successor completion".into()
            )))
            .is_err()
    );
    assert_eq!(
        launcher
            .inspect(&plan.container_name)
            .unwrap()
            .unwrap()
            .state
            .status,
        "running"
    );
    launcher
        .pause_trial_restore_state(&restore, &plan, &mut || Ok(()))
        .unwrap();
    assert_eq!(
        launcher
            .inspect(&plan.container_name)
            .unwrap()
            .unwrap()
            .state
            .status,
        "exited"
    );
    assert!(plan.state_root.join("agent/life").exists());
}

#[test]
fn trial_reclaim_corrupt_remote_preserves_local_compute_and_data() {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, _, archive) = archive_fixture(&temp);
    let snapshot = launcher
        .archive_trial_state(&archive, &mut || Ok(()))
        .unwrap();
    std::fs::write(
        temp.path().join("borg-remote/archive.zip"),
        "partial upload",
    )
    .unwrap();
    let reclaim = reclaim_lease(&archive, snapshot);
    assert!(
        launcher
            .reclaim_trial_state(&reclaim, &mut || Ok(()))
            .is_err()
    );
    assert!(plan.state_root.join("agent/life").exists());
    assert!(launcher.inspect(&plan.container_name).unwrap().is_some());
}

#[test]
fn trial_reclaim_changed_local_data_reserves_capacity_and_preserves_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, _, archive) = archive_fixture(&temp);
    let snapshot = launcher
        .archive_trial_state(&archive, &mut || Ok(()))
        .unwrap();
    std::fs::write(plan.state_root.join("agent/life"), "unexpected new history").unwrap();
    let reclaim = reclaim_lease(&archive, snapshot);
    assert!(
        launcher
            .reclaim_trial_state(&reclaim, &mut || Ok(()))
            .is_err()
    );
    assert!(plan.state_root.join("agent/life").exists());
    assert_eq!(active_kata_container_count(&launcher.config), Some(1));
    assert!(temp.path().join("borg-remote/archive.zip").exists());
}

#[test]
fn trial_archive_failed_readback_or_stale_lease_never_removes_local_state() {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, _, archive) = archive_fixture(&temp);
    std::fs::write(temp.path().join("borg-fail-extract"), "1").unwrap();
    assert!(
        launcher
            .archive_trial_state(&archive, &mut || Ok(()))
            .is_err()
    );
    assert!(plan.state_root.join("agent/life").exists());
    assert!(launcher.inspect(&plan.container_name).unwrap().is_some());
    std::fs::remove_file(temp.path().join("borg-fail-extract")).unwrap();
    let snapshot = launcher
        .archive_trial_state(&archive, &mut || Ok(()))
        .unwrap();
    assert!(
        launcher
            .reclaim_trial_state(&reclaim_lease(&archive, snapshot), &mut || Err(
                RunnerError::CoreRequest("stale lease".into())
            ))
            .is_err()
    );
    assert!(plan.state_root.exists());
}

#[test]
fn trial_archive_rejects_unencrypted_or_local_repository() {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, _, archive) = archive_fixture(&temp);
    launcher.config.retirement.as_mut().unwrap().repository = "/local/repository".into();
    assert!(
        launcher
            .archive_trial_state(&archive, &mut || Ok(()))
            .is_err()
    );
    launcher.config.retirement.as_mut().unwrap().repository =
        "ssh://archive.example/./repository".into();
    let borg = &launcher.config.retirement.as_ref().unwrap().borg_bin;
    let script = std::fs::read_to_string(borg)
        .unwrap()
        .replace("repokey", "none");
    std::fs::write(borg, script).unwrap();
    assert!(
        launcher
            .archive_trial_state(&archive, &mut || Ok(()))
            .is_err()
    );
    assert!(plan.state_root.join("agent/life").exists());
    assert!(launcher.inspect(&plan.container_name).unwrap().is_some());
}

fn restored_fixture(
    temp: &tempfile::TempDir,
) -> (KataLauncher, KataLaunchPlan, PathBuf, AgentCreationLease) {
    let (mut launcher, plan, fake, archive) = archive_fixture(temp);
    let snapshot = launcher
        .archive_trial_state(&archive, &mut || Ok(()))
        .unwrap();
    launcher
        .reclaim_trial_state(&reclaim_lease(&archive, snapshot.clone()), &mut || Ok(()))
        .unwrap();
    let mut restore = relocation_creation_lease(
        "agent_request_trial_restore",
        "finite-lat-1",
        "finite-lat-1",
        &snapshot.durable_state_manifest_sha256,
        true,
    );
    let finite_saas_core::RuntimeRelocationEnvelope::V1(relocation) =
        restore.request.relocation.as_mut().unwrap();
    relocation.trial_archive = Some(snapshot);
    launcher.trial_restore_private_key(&restore).unwrap();
    launcher
        .restore_trial_state(&restore, &plan, &mut || Ok(()))
        .unwrap();
    (launcher, plan, fake, restore)
}

#[test]
fn trial_restore_launch_rechecks_authority_after_host_lock() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    for denial in ["successor completed", "billing denied"] {
        let temp = tempfile::tempdir().unwrap();
        let (launcher, plan, fake, restore) = restored_fixture(&temp);
        write_fake_container(
            &fake,
            &plan.container_name,
            RELOCATION_TEST_IMAGE,
            "artifact-v1",
            "",
            41234,
            &plan.state_root,
        );
        std::fs::write(plan.state_root.join("agent/life"), "successor writes").unwrap();
        if denial == "billing denied" {
            std::fs::write(
                fake.join(format!("{}.status", plan.container_name)),
                "exited",
            )
            .unwrap();
        }
        let before_status = launcher
            .inspect(&plan.container_name)
            .unwrap()
            .unwrap()
            .state
            .status;
        std::fs::write(fake.join("commands.log"), "").unwrap();
        let lock = launcher.acquire_runtime_operation_lock(&plan).unwrap();
        let allowed = Arc::new(AtomicBool::new(true));
        let called = Arc::new(AtomicBool::new(false));
        let allowed_worker = Arc::clone(&allowed);
        let called_worker = Arc::clone(&called);
        let (tx, rx) = mpsc::channel();
        let mut worker = KataLauncher::new(launcher.config.clone());
        let lease = restore.clone();
        let thread = std::thread::spawn(move || {
            tx.send(()).unwrap();
            worker.launch_trial_restore(&lease, &RuntimeLaunchOptions::default(), &mut || {
                called_worker.store(true, Ordering::SeqCst);
                if allowed_worker.load(Ordering::SeqCst) {
                    Ok(())
                } else {
                    Err(RunnerError::CoreRequest(denial.into()))
                }
            })
        });
        rx.recv().unwrap();
        // Worker A's initial authorization becomes stale while the host lock is held.
        allowed.store(false, Ordering::SeqCst);
        assert!(!called.load(Ordering::SeqCst));
        drop(lock);
        assert!(thread.join().unwrap().is_err());
        assert!(called.load(Ordering::SeqCst));
        let mut direct = KataLauncher::new(launcher.config.clone());
        assert!(
            direct
                .launch(&restore, &RuntimeLaunchOptions::default())
                .is_err()
        );
        assert_eq!(
            launcher
                .inspect(&plan.container_name)
                .unwrap()
                .unwrap()
                .state
                .status,
            before_status
        );
        assert_eq!(
            std::fs::read_to_string(plan.state_root.join("agent/life")).unwrap(),
            "successor writes"
        );
        let commands = std::fs::read_to_string(fake.join("commands.log")).unwrap();
        assert!(!commands.lines().any(|line| line.starts_with("rm ")
            || line.starts_with("run ")
            || line.starts_with("stop ")));
    }
}

#[test]
fn trial_restore_key_survives_runner_restart_and_rejects_missing_or_unsafe_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, _, restore) = restored_fixture(&temp);
    let key = launcher.trial_restore_private_key(&restore).unwrap();
    let mut restarted = KataLauncher::new(launcher.config.clone());
    assert!(restarted.trial_restore_private_key(&restore).unwrap() == key);
    let path = plan
        .metadata_root
        .join(format!("trial-key-{}.json", restore.request.id));
    assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    let stored = std::fs::read(&path).unwrap();
    std::fs::write(&path, "partial").unwrap();
    assert!(launcher.trial_restore_private_key(&restore).is_err());
    std::fs::write(&path, &stored).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(launcher.trial_restore_private_key(&restore).is_err());
    std::fs::remove_file(&path).unwrap();
    assert!(launcher.trial_restore_private_key(&restore).is_err());
    let victim = temp.path().join("unrelated");
    std::fs::write(&victim, "keep").unwrap();
    std::os::unix::fs::symlink(&victim, &path).unwrap();
    assert!(launcher.trial_restore_private_key(&restore).is_err());
    assert_eq!(std::fs::read_to_string(victim).unwrap(), "keep");
}

#[test]
fn trial_restore_authorized_launch_preserves_identity_and_uses_durable_key() {
    let server = TestHttpServer::start("npub1sameagent");
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, fake, restore) = restored_fixture(&temp);
    std::fs::write(fake.join("candidate-port"), server.port.to_string()).unwrap();
    let key = launcher.trial_restore_private_key(&restore).unwrap();
    let options = RuntimeLaunchOptions {
        finite_private: Some(FinitePrivateLaunchKey {
            api_key_id: "operation-key".into(),
            raw_api_key: key.clone(),
            base_url: "https://private.example".into(),
            model: "test".into(),
            revoke_on_launch_failure: true,
        }),
        ..Default::default()
    };
    let mut renewals = 0;
    let facts = launcher
        .launch_trial_restore(&restore, &options, &mut || {
            renewals += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(renewals, 1);
    assert_eq!(facts.source_machine_id, plan.container_name);
    assert!(launcher.trial_restore_private_key(&restore).unwrap() == key);
    assert_eq!(
        std::fs::read_to_string(plan.state_root.join("agent/life")).unwrap(),
        "everything"
    );
}
