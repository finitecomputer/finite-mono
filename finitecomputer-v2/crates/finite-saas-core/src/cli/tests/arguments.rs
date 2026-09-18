use super::*;

#[test]
fn canary_retry_requires_exact_ids_and_defaults_to_preview() {
    let args = [
        "finite-saas-core",
        "launch-code-retry-target-exact",
        "--code-id",
        "new-code",
        "--expected-batch-id",
        "new-batch",
        "--previous-code-id",
        "old-code",
        "--expected-previous-request-id",
        "old-request",
        "--expected-previous-project-id",
        "old-project",
        "--expected-previous-runtime-id",
        "old-runtime",
        "--expected-previous-source-host-id",
        "wrong-host",
        "--target-source-host-id",
        "target-host",
        "--operator-email",
        "operator@finite.vip",
        "--operator-workos-user-id",
        "operator",
    ];
    assert!(matches!(
        Args::try_parse_from(args).unwrap().command,
        Some(Command::LaunchCodeRetryTargetExact { execute: false, .. })
    ));
    assert!(matches!(
        Args::try_parse_from(args.into_iter().chain(["--execute"]))
            .unwrap()
            .command,
        Some(Command::LaunchCodeRetryTargetExact { execute: true, .. })
    ));
    assert!(Args::try_parse_from(&args[..args.len() - 2]).is_err());
}

#[test]
fn host_release_requires_exact_ids_and_defaults_to_preview() {
    let args = [
        "finite-saas-core",
        "launch-host-release-exact",
        "--reservation-code-id",
        "root",
        "--source-host-id",
        "host",
        "--expected-canary-runtime-id",
        "runtime",
        "--operator-email",
        "operator@finite.vip",
        "--operator-workos-user-id",
        "operator",
    ];
    assert!(matches!(
        Args::try_parse_from(args).unwrap().command,
        Some(Command::LaunchHostReleaseExact { execute: false, .. })
    ));
    assert!(matches!(
        Args::try_parse_from(args.into_iter().chain(["--execute"]))
            .unwrap()
            .command,
        Some(Command::LaunchHostReleaseExact { execute: true, .. })
    ));
    assert!(Args::try_parse_from(&args[..args.len() - 2]).is_err());
    assert!(Args::try_parse_from(["finite-saas-core", "launch-code-batch-target-exact"]).is_err());
}

#[test]
fn rollout_cli_requires_one_scope_and_bounds_wait_timeout() {
    let common = [
        "finite-saas-core",
        "runtime-artifact-rollout",
        "--artifact-id",
        "artifact-v2",
        "--source-host-id",
        "lat1",
        "--admin-email",
        "admin@finite.vip",
        "--admin-workos-user-id",
        "workos-admin",
    ];
    assert!(Args::try_parse_from(common.into_iter().chain(["--project-id", "project-a"])).is_ok());
    let exact = Args::try_parse_from(common.into_iter().chain([
        "--project-id",
        "project-a",
        "--expected-agent-runtime-id",
        "runtime-a",
        "--expected-source-machine-id",
        "finite-kata-a",
    ]))
    .unwrap();
    let Some(Command::RuntimeArtifactRollout(exact)) = exact.command else {
        panic!("expected rollout command");
    };
    assert!(RuntimeArtifactRolloutInput::try_from(exact).is_ok());
    assert!(Args::try_parse_from(common).is_err());
    assert!(
        Args::try_parse_from(common.into_iter().chain([
            "--all",
            "--canary-project-id",
            "project-a",
            "--project-id",
            "project-b",
        ]))
        .is_err()
    );
    assert!(
        Args::try_parse_from(common.into_iter().chain([
            "--project-id",
            "project-a",
            "--wait-timeout-seconds",
            "0",
        ]))
        .is_err()
    );
    assert!(
        Args::try_parse_from(common.into_iter().chain([
            "--project-id",
            "project-a",
            "--wait-timeout-seconds",
            "3601",
        ]))
        .is_err()
    );
}

#[test]
fn unrecoverable_archive_cli_requires_all_three_acknowledgements() {
    let common = [
        "finite-saas-core",
        "runtime-archive-unrecoverable",
        "--project-id",
        "project-a",
        "--expected-agent-runtime-id",
        "runtime-a",
        "--expected-source-host-id",
        "lat1",
        "--expected-source-machine-id",
        "finite-kata-a",
        "--expected-owner-email",
        "owner@finite.vip",
        "--admin-email",
        "admin@finite.vip",
        "--admin-workos-user-id",
        "workos-admin",
    ];
    assert!(Args::try_parse_from(common).is_err());
    assert!(
        Args::try_parse_from(common.into_iter().chain([
            "--confirm-compute-absent",
            "--confirm-durable-state-absent",
            "--confirm-owner-acknowledged-unrecoverable",
        ]))
        .is_ok()
    );
}

#[test]
fn offboard_retired_cli_requires_confirm_compute_absent() {
    let common = [
        "finite-saas-core",
        "runtime-offboard-retired-exact",
        "--project-id",
        "project-a",
        "--expected-agent-runtime-id",
        "runtime-a",
        "--expected-source-host-id",
        "lat1",
        "--expected-source-machine-id",
        "finite-kata-a",
        "--expected-owner-email",
        "owner@finite.vip",
        "--admin-email",
        "admin@finite.vip",
        "--admin-workos-user-id",
        "workos-admin",
    ];
    assert!(Args::try_parse_from(common).is_err());
    assert!(Args::try_parse_from(common.into_iter().chain(["--confirm-compute-absent"])).is_ok());
}

#[test]
fn exact_retirement_cli_requires_the_complete_binding_and_bounded_timeout() {
    let common = [
        "finite-saas-core",
        "runtime-retire-exact",
        "--project-id",
        "project-a",
        "--expected-agent-runtime-id",
        "runtime-a",
        "--expected-source-host-id",
        "lat1",
        "--expected-source-machine-id",
        "finite-kata-a",
        "--admin-email",
        "admin@finite.vip",
        "--admin-workos-user-id",
        "workos-admin",
    ];
    assert!(Args::try_parse_from(common).is_ok());
    assert!(
        Args::try_parse_from(common.into_iter().chain(["--wait-timeout-seconds", "59"])).is_err()
    );
    assert!(
        Args::try_parse_from(common.into_iter().chain(["--wait-timeout-seconds", "3601"])).is_err()
    );
}

#[test]
fn artifact_upsert_accepts_explicit_unpromoted_canary_without_changing_legacy_flags() {
    let base = [
        "finite-saas-core",
        "runtime-artifact-upsert",
        "--artifact-id",
        "candidate",
        "--reference",
        "image",
        "--version-label",
        "candidate",
    ];
    for extra in [vec![], vec!["--promoted"], vec!["--promoted", "true"]] {
        assert!(matches!(
            Args::try_parse_from(base.into_iter().chain(extra))
                .unwrap()
                .command,
            Some(Command::RuntimeArtifactUpsert {
                promoted: true,
                canary_runtime_id: None,
                ..
            })
        ));
    }
    assert!(matches!(Args::try_parse_from(base.into_iter().chain([
        "--promoted", "false", "--canary-runtime-id", "runtime-exact",
    ])).unwrap().command,
        Some(Command::RuntimeArtifactUpsert { promoted: false, canary_runtime_id: Some(id), .. }) if id == "runtime-exact"));
}
