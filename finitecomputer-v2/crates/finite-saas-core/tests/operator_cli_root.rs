//! The built `finite-saas-core` binary refuses `agent-create-for-owner-exact`
//! for a non-root OS user, even with every credential in its environment.

#[test]
fn agent_create_for_owner_exact_refuses_a_non_root_os_user() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_finite-saas-core"))
        .args([
            "agent-create-for-owner-exact",
            "--owner-email",
            "frankie@finite.vip",
            "--display-name",
            "Frankie Bot",
            "--reason",
            "box1 migration",
            "--idempotency-key",
            "box1-frankie",
            "--admin-email",
            "austin@finite.vip",
            "--admin-workos-user-id",
            "user_admin",
            "--execute",
        ])
        // Would-be-usable configuration: none of it may be reached.
        .env(
            "FC_CORE_DATABASE_URL",
            "postgresql://127.0.0.1:1/finite_core",
        )
        .env("FC_HOSTED_WEB_DEVICE_URL", "http://127.0.0.1:1")
        .env("FINITECHAT_HOSTED_API_TOKEN", "token")
        .env("WORKOS_API_KEY", "key")
        .env("FC_WORKOS_OPERATOR_ORG_ID", "org")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("requires OS root"), "{stderr}");
    assert!(output.stdout.is_empty());
}
