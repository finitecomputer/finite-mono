use super::*;

const PROTECTED: &str = r#"{"events":[{"lease":{"state":"refusal_v1"}}]}"#;
const OLD_IMAGE: &str = "ghcr.io/finitecomputer/agent-runtime:v1@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn restart_excludes_a_live_upgrade_writer_even_before_it_creates_protected_state() {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, fake) = test_launcher(&temp, 41306);
    write_fake_container(
        &fake,
        &plan.container_name,
        OLD_IMAGE,
        "artifact-v1",
        "",
        41306,
        &plan.state_root,
    );
    std::fs::write(
        fake.join(format!("{}.status", plan.container_name)),
        "exited",
    )
    .unwrap();
    let candidate = stale_created_candidate(&fake, &plan, "runtime_ctl_interrupted", 41306);
    std::fs::write(fake.join(format!("{candidate}.status")), "running").unwrap();
    std::fs::create_dir_all(plan.state_root.join("agent")).unwrap();
    std::fs::write(
        plan.state_root.join("agent/hermes-inbox.json"),
        r#"{"events":[]}"#,
    )
    .unwrap();
    let mut restart = recovery_lease("runtime_ctl_later_restart", OLD_IMAGE, 41306);
    restart.request.kind = RuntimeControlKind::Restart;
    let RuntimeSpecEnvelope::V1(spec) = restart.runtime_spec.as_mut().unwrap();
    spec.boot_intent = RuntimeBootIntent::Normal;
    let error = launcher
        .restart_runtime(&restart, &RuntimeRestartOptions::default())
        .unwrap_err();
    assert!(
        error.to_string().contains("upgrade candidate is writing"),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(fake.join(format!("{candidate}.status"))).unwrap(),
        "running"
    );
    assert_eq!(
        std::fs::read_to_string(fake.join(format!("{}.status", plan.container_name))).unwrap(),
        "exited"
    );
    assert!(
        !std::fs::read_to_string(fake.join("commands.log"))
            .unwrap()
            .contains("restart --time")
    );
}

#[test]
fn interrupted_swap_preserves_candidate_and_rollback_when_inbox_is_protected() {
    let temp = tempfile::tempdir().unwrap();
    let (launcher, plan, fake) = test_launcher(&temp, 41306);
    let request = "runtime_ctl_refusal_interrupted";
    let rollback = kata_upgrade_helper_name(&plan.container_name, "rollback", request);
    write_fake_container(
        &fake,
        &rollback,
        OLD_IMAGE,
        "artifact-v1",
        "",
        41306,
        &plan.state_root,
    );
    std::fs::write(fake.join(format!("{rollback}.status")), "exited").unwrap();
    let candidate = stale_created_candidate(&fake, &plan, request, 41306);
    std::fs::create_dir_all(plan.state_root.join("agent")).unwrap();
    std::fs::write(plan.state_root.join("agent/hermes-inbox.json"), PROTECTED).unwrap();
    let error = launcher
        .reconcile_interrupted_upgrade(&plan, "project-1", request, &target_artifact())
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("pending durable command refusals"),
        "{error}"
    );
    assert!(fake.join(format!("{candidate}.image")).exists());
    assert!(fake.join(format!("{rollback}.image")).exists());
    assert!(!fake.join(format!("{}.image", plan.container_name)).exists());
}

#[test]
fn capable_reader_and_drained_legacy_reader_can_take_over() {
    let temp = tempfile::tempdir().unwrap();
    let (launcher, plan, fake) = test_launcher(&temp, 41306);
    write_fake_container(
        &fake,
        &plan.container_name,
        OLD_IMAGE,
        "artifact-v1",
        "",
        41306,
        &plan.state_root,
    );
    std::fs::create_dir_all(plan.state_root.join("agent")).unwrap();
    let path = plan.state_root.join("agent/hermes-inbox.json");
    std::fs::write(&path, PROTECTED).unwrap();
    let mut inspected = launcher.inspect(&plan.container_name).unwrap().unwrap();
    assert!(launcher.check_container_chat_reader(&inspected).is_err());
    use finitechat_hermes::inbox_compatibility::{READER_LABEL, REFUSAL_V1_READER};
    inspected
        .config
        .labels
        .insert(READER_LABEL.into(), REFUSAL_V1_READER.into());
    launcher.check_container_chat_reader(&inspected).unwrap();
    inspected.config.labels.remove(READER_LABEL);
    std::fs::write(&path, r#"{"events":[],"acked":[{"key":"settled","refusal_reply":{"seq":2,"message_id":"reply"}}]}"#).unwrap();
    launcher.check_container_chat_reader(&inspected).unwrap();
}

#[test]
fn failed_upgrade_preserves_capable_candidate_when_refusals_block_rollback() {
    let old = TestHttpServer::start("npub1sameagent");
    let candidate = TestHttpServer::start("npub1differentagent");
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, fake) = test_launcher(&temp, candidate.port);
    write_fake_container(
        &fake,
        &plan.container_name,
        OLD_IMAGE,
        "artifact-v1",
        "",
        old.port,
        &plan.state_root,
    );
    std::fs::write(fake.join("protected-inbox-on-run"), PROTECTED).unwrap();
    let lease = upgrade_lease("runtime_ctl_refusal_rollback");
    let error = launcher
        .upgrade_runtime(&lease, &RuntimeRestartOptions::default())
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("pending durable command refusals"),
        "{error}"
    );
    let name = kata_upgrade_helper_name(&plan.container_name, "candidate", &lease.request.id);
    assert!(
        fake.join(format!("{name}.image")).exists(),
        "capable handle must survive"
    );
    assert_ne!(
        std::fs::read_to_string(fake.join(format!("{}.status", plan.container_name))).unwrap(),
        "running"
    );
    assert_eq!(
        std::fs::read_to_string(plan.state_root.join("agent/hermes-inbox.json")).unwrap(),
        PROTECTED
    );
    let retry = launcher
        .upgrade_runtime(&lease, &RuntimeRestartOptions::default())
        .unwrap_err();
    assert!(
        retry
            .to_string()
            .contains("pending durable command refusals"),
        "{retry}"
    );
    assert!(fake.join(format!("{name}.image")).exists());
    let mut restart = recovery_lease("runtime_ctl_refusal_restart", OLD_IMAGE, old.port);
    restart.request.kind = RuntimeControlKind::Restart;
    let RuntimeSpecEnvelope::V1(spec) = restart.runtime_spec.as_mut().unwrap();
    spec.boot_intent = RuntimeBootIntent::Normal;
    let error = launcher
        .restart_runtime(&restart, &RuntimeRestartOptions::default())
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("pending durable command refusals"),
        "{error}"
    );
    assert_ne!(
        std::fs::read_to_string(fake.join(format!("{}.status", plan.container_name))).unwrap(),
        "running"
    );
}

#[test]
fn hosted_classifier_allows_image_inspection_but_rejects_image_removal() {
    let temp = tempfile::tempdir().unwrap();
    let (launcher, _, _) = test_launcher(&temp, 41306);
    let lifecycle = crate::hosted_hermes_lifecycle::HostedHermesLifecycle::for_failed_mutation_test(
        launcher.config.nerdctl_bin.clone(),
        launcher.config.namespace.clone(),
        temp.path(),
    );
    let inspect = launcher.command(vec!["image".into(), "inspect".into(), OLD_IMAGE.into()]);
    let invoked = std::cell::Cell::new(false);
    let result = lifecycle.execute(&inspect, Duration::from_secs(1), |_, _| {
        invoked.set(true);
        Err(RunnerError::RuntimeLaunch("test transport reached".into()))
    });
    assert!(invoked.get());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("test transport reached")
    );
    let remove = launcher.command(vec!["image".into(), "rm".into(), OLD_IMAGE.into()]);
    assert!(
        lifecycle
            .execute(&remove, Duration::from_secs(1), |_, _| panic!(
                "mutation escaped fence"
            ))
            .is_err()
    );
}

#[test]
fn noncanonical_chat_home_blocks_handoff_without_guessing_the_state_path() {
    let temp = tempfile::tempdir().unwrap();
    let (launcher, plan, fake) = test_launcher(&temp, 41306);
    write_fake_container(
        &fake,
        &plan.container_name,
        OLD_IMAGE,
        "artifact-v1",
        "",
        41306,
        &plan.state_root,
    );
    let mut inspected = launcher.inspect(&plan.container_name).unwrap().unwrap();
    inspected
        .config
        .environment
        .push("FINITECHAT_HOME=/data/other-chat".into());
    assert!(
        launcher
            .check_container_chat_reader(&inspected)
            .unwrap_err()
            .to_string()
            .contains("canonical FINITECHAT_HOME")
    );
}

#[test]
fn failed_adopted_rollback_keeps_expected_principal_marker() {
    let target = TestHttpServer::start("npub1differentagent");
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, fake) = test_launcher(&temp, target.port);
    let lease = upgrade_lease("runtime_ctl_refusal_identity");
    let rollback = kata_upgrade_helper_name(&plan.container_name, "rollback", &lease.request.id);
    write_fake_container(
        &fake,
        &plan.container_name,
        &target_artifact().reference,
        "artifact-v2",
        &lease.request.id,
        target.port,
        &plan.state_root,
    );
    write_fake_container(
        &fake,
        &rollback,
        OLD_IMAGE,
        "artifact-v1",
        "",
        target.port,
        &plan.state_root,
    );
    std::fs::write(fake.join(format!("{rollback}.status")), "exited").unwrap();
    write_kata_upgrade_expected_npub(&plan, &lease.request.id, "npub1sameagent").unwrap();
    std::fs::create_dir_all(plan.state_root.join("agent")).unwrap();
    std::fs::write(plan.state_root.join("agent/hermes-inbox.json"), PROTECTED).unwrap();
    assert!(
        launcher
            .upgrade_runtime(&lease, &RuntimeRestartOptions::default())
            .is_err()
    );
    assert_eq!(
        read_kata_upgrade_expected_npub(&plan, &lease.request.id).unwrap(),
        "npub1sameagent"
    );
    assert!(fake.join(format!("{}.image", plan.container_name)).exists());
    assert!(fake.join(format!("{rollback}.image")).exists());
}

#[test]
fn adopted_candidate_is_not_deleted_before_rollback_compatibility_check() {
    let server = TestHttpServer::start("npub1sameagent");
    let temp = tempfile::tempdir().unwrap();
    let (launcher, plan, fake) = test_launcher(&temp, server.port);
    let rollback = "rollback-test";
    write_fake_container(
        &fake,
        &plan.container_name,
        OLD_IMAGE,
        "candidate",
        "",
        server.port,
        &plan.state_root,
    );
    write_fake_container(
        &fake,
        rollback,
        OLD_IMAGE,
        "artifact-v1",
        "",
        server.port,
        &plan.state_root,
    );
    std::fs::create_dir_all(plan.state_root.join("agent")).unwrap();
    std::fs::write(plan.state_root.join("agent/hermes-inbox.json"), PROTECTED).unwrap();
    assert!(
        launcher
            .restore_rollback_after_adopted_target_failure(&plan, rollback, None)
            .is_err()
    );
    assert!(fake.join(format!("{}.image", plan.container_name)).exists());
    assert!(fake.join(format!("{rollback}.image")).exists());
}
