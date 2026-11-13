use super::*;

#[test]
fn memory_admission_refreshes_capacity_and_keeps_control_capabilities() {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, _, _) = test_launcher(&temp, 41234);
    launcher.config.memory_admission = true;
    launcher.config.available_memory_bytes = Some(u64::MAX); // stale startup reading
    let path = launcher.config.host_root.join("proc/meminfo");
    std::fs::write(&path, "MemAvailable: 1 kB\n").unwrap();
    let low = launcher.runner_capacity();
    assert!(low.draining);
    assert_eq!(low.available_memory_bytes, Some(1024));
    assert_eq!(
        low.runtime_capabilities,
        Some(launcher.runtime_capabilities())
    );
    std::fs::write(&path, "MemAvailable: 25165824 kB\n").unwrap();
    // Missing /dev/shm evidence still fails closed even when RAM is sufficient.
    let unknown_shared = launcher.runner_capacity();
    assert!(unknown_shared.draining);
    assert_eq!(
        unknown_shared.available_memory_bytes,
        Some(24 * 1024 * 1024 * 1024)
    );
    std::fs::remove_file(path).unwrap();
    assert_eq!(launcher.runner_capacity().available_memory_bytes, None);
}

#[test]
fn memory_admission_rechecks_before_replacing_compute_or_writing_launch_state() {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, fake) = test_launcher(&temp, 41234);
    write_fake_container(
        &fake,
        &plan.container_name,
        OLD_TEST_IMAGE,
        "artifact-v1",
        "",
        41234,
        &plan.state_root,
    );
    let sentinel = plan.state_root.join("keep-agent-data");
    std::fs::write(&sentinel, "existing history").unwrap();
    std::fs::write(fake.join("commands.log"), "").unwrap();
    launcher.config.memory_admission = true;
    let lease = relocation_creation_lease(
        "agent_request_memory_guard",
        "finite-lat-1",
        "finite-lat-1",
        &"0".repeat(64),
        true,
    );
    let error = launcher
        .run_fresh(&plan, &lease, &RuntimeLaunchOptions::default(), &mut || {
            Ok(())
        })
        .unwrap_err();
    assert!(error.to_string().contains("memory observation unavailable"));
    assert!(provider_mutations(&fake).is_empty());
    assert_eq!(
        std::fs::read_to_string(sentinel).unwrap(),
        "existing history"
    );
    assert!(launcher.inspect(&plan.container_name).unwrap().is_some());
    assert!(!plan.env_file.exists());
}
