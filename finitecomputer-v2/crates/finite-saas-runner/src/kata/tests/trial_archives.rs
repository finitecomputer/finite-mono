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
