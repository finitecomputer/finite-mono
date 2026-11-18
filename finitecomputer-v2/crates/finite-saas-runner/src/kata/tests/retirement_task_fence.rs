//! Retirement against the STOPPED task nerdctl leaves behind, and its
//! retries.
use super::*;

/// A `ctr` whose task list is the file `ctr-tasks` beside it, one
/// `<id> <pid> <STATUS>` row per task. `tasks delete` refuses a task that
/// is not STOPPED, as containerd's client does without `--force`. Every call
/// is appended to `ctr-calls`, and to the fake nerdctl's `commands.log` so a
/// test can order the two.
fn stateful_ctr(root: &Path) -> PathBuf {
    let bin = root.join("ctr-stateful");
    write_executable(
        &bin,
        r#"#!/bin/sh
set -eu
dir="$(dirname "$0")"
tasks="$dir/ctr-tasks"
touch "$tasks"
echo "$*" >> "$dir/ctr-calls"
echo "ctr $*" >> "$dir/fake-state/commands.log"
if [ "${1:-}" = "--namespace" ]; then shift 2; fi
case "${1:-} ${2:-}" in
  "tasks list")
    if [ "${3:-}" = "-q" ]; then
      while read -r id pid status; do echo "$id"; done < "$tasks"
    else
      printf 'TASK    PID    STATUS\n'
      while read -r id pid status; do printf '%s    %s    %s\n' "$id" "$pid" "$status"; done < "$tasks"
    fi
    ;;
  "tasks delete")
    if [ "$#" -ne 3 ]; then echo "unexpected delete arguments: $*" >&2; exit 2; fi
    found=""
    : > "$tasks.next"
    while read -r id pid status; do
      if [ "$id" = "$3" ]; then found="$status"; else echo "$id $pid $status" >> "$tasks.next"; fi
    done < "$tasks"
    case "$found" in
      "") echo "ctr: container \"$3\" in namespace: not found" >&2; rm -f "$tasks.next"; exit 1 ;;
      STOPPED) mv "$tasks.next" "$tasks" ;;
      *) echo "ctr: task must be stopped before deletion: $found: failed precondition" >&2; rm -f "$tasks.next"; exit 1 ;;
    esac
    ;;
  *) echo "unsupported fake ctr call: $*" >&2; exit 42 ;;
esac
"#,
    );
    bin
}

/// A stopped canonical container on a host whose `ctr` is
/// [`stateful_ctr`], and a Destroy retirement lease for it.
fn stopped_runtime(
    temp: &tempfile::TempDir,
    port: u16,
) -> (KataLauncher, KataLaunchPlan, RuntimeControlLease, String) {
    let (mut launcher, plan, fake_state) = test_launcher(temp, port);
    configure_test_retirement(&mut launcher, temp.path());
    let retirement = launcher.config.retirement.as_mut().unwrap();
    retirement.ctr_bin = stateful_ctr(temp.path());
    retirement.convergence_timeout = Duration::from_millis(200);
    write_fake_container(
        &fake_state,
        &plan.container_name,
        "runtime:v1",
        "artifact-v1",
        "",
        port,
        &plan.state_root,
    );
    std::fs::write(
        fake_state.join(format!("{}.status", plan.container_name)),
        "exited",
    )
    .unwrap();
    std::fs::create_dir_all(plan.state_root.join("workspace")).unwrap();
    std::fs::write(plan.state_root.join("workspace/note"), "preserved").unwrap();
    let image = "registry.example/runtime@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let mut lease = recovery_lease("runtime_ctl_retirement_task_fence", image, port);
    lease.request.kind = RuntimeControlKind::Destroy;
    let RuntimeSpecEnvelope::V1(spec) = lease.runtime_spec.as_mut().unwrap();
    spec.boot_intent = RuntimeBootIntent::Normal;
    let id = fake_container_id(&plan.container_name);
    (launcher, plan, lease, id)
}

fn set_tasks(temp: &tempfile::TempDir, rows: &[String]) {
    let mut contents = rows.join("\n");
    contents.push('\n');
    std::fs::write(temp.path().join("ctr-tasks"), contents).unwrap();
}

fn tasks(temp: &tempfile::TempDir) -> String {
    std::fs::read_to_string(temp.path().join("ctr-tasks")).unwrap()
}

fn delete_calls(temp: &tempfile::TempDir) -> Vec<String> {
    std::fs::read_to_string(temp.path().join("ctr-calls"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains("tasks delete"))
        .map(str::to_string)
        .collect()
}

#[test]
fn retirement_deletes_the_stopped_task_a_stop_leaves_and_proceeds() {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, lease, id) = stopped_runtime(&temp, 41310);
    // Production, 2026-11-17: the agent was stopped days earlier; its task
    // stayed listed as STOPPED (exit 143) until deleted by hand.
    let other = format!("{:x}", Sha256::digest(b"another-runtime"));
    set_tasks(
        &temp,
        &[
            format!("{id} 4242 STOPPED"),
            format!("{other} 4343 STOPPED"),
        ],
    );

    let receipt = launcher.retire_runtime(&lease, &mut || Ok(())).unwrap();

    assert_eq!(receipt.request_id, lease.request.id);
    assert_eq!(
        delete_calls(&temp),
        vec![format!("--namespace finite tasks delete {id}")],
        "exactly this container's task is deleted, once and without --force"
    );
    // The container had exited on its own. It is still stopped through
    // nerdctl, which records the explicit stop, before its task is deleted.
    let commands = std::fs::read_to_string(temp.path().join("fake-state/commands.log")).unwrap();
    let position = |needle: &str| {
        commands
            .lines()
            .position(|line| line.starts_with(needle))
            .unwrap_or_else(|| panic!("no {needle:?} in {commands}"))
    };
    assert!(position("stop ") < position("ctr --namespace finite tasks delete"));
    assert_eq!(tasks(&temp), format!("{other} 4343 STOPPED\n"));
    assert!(temp.path().join("borg-remote/archive.zip").is_file());
    assert!(launcher.inspect(&plan.container_name).unwrap().is_none());
    assert_eq!(
        std::fs::read_to_string(plan.state_root.join("workspace/note")).unwrap(),
        "preserved"
    );
}

#[test]
fn retirement_fences_a_task_that_is_not_stopped() {
    for status in ["RUNNING", "UNKNOWN", "PAUSED", "PAUSING", "CREATED"] {
        let temp = tempfile::tempdir().unwrap();
        let (mut launcher, plan, lease, id) = stopped_runtime(&temp, 41311);
        let row = format!("{id} 4242 {status}");
        set_tasks(&temp, std::slice::from_ref(&row));

        let error = launcher
            .retire_runtime(&lease, &mut || Ok(()))
            .unwrap_err()
            .to_string();

        assert!(error.contains("did not become absent"), "{status}: {error}");
        assert!(delete_calls(&temp).is_empty(), "{status}: no delete");
        assert_eq!(tasks(&temp), format!("{row}\n"), "{status}");
        assert!(launcher.inspect(&plan.container_name).unwrap().is_some());
        assert!(!temp.path().join("borg-remote/archive.zip").exists());
    }
}

#[test]
fn retirement_refuses_a_stopped_task_that_appears_after_the_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, lease, id) = stopped_runtime(&temp, 41312);
    set_tasks(&temp, &[format!("{id} 4242 STOPPED")]);
    // The first attempt deletes the leftover, snapshots, and fails readback.
    std::fs::write(temp.path().join("borg-fail-extract"), "fail").unwrap();
    assert!(launcher.retire_runtime(&lease, &mut || Ok(())).is_err());
    assert_eq!(delete_calls(&temp).len(), 1);
    assert!(tasks(&temp).trim().is_empty());

    // The same container then ran and stopped again, after the ZIP was
    // taken. The resumed retirement must not tidy that task away.
    std::fs::remove_file(temp.path().join("borg-fail-extract")).unwrap();
    set_tasks(&temp, &[format!("{id} 4343 STOPPED")]);
    let error = launcher
        .retire_runtime(&lease, &mut || Ok(()))
        .unwrap_err()
        .to_string();

    assert!(error.contains("did not become absent"), "{error}");
    assert_eq!(delete_calls(&temp).len(), 1, "no second delete");
    assert_eq!(tasks(&temp), format!("{id} 4343 STOPPED\n"));
    assert!(launcher.inspect(&plan.container_name).unwrap().is_some());
}

#[test]
fn retirement_keeps_the_task_while_the_restart_monitor_would_restart_it() {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, lease, id) = stopped_runtime(&temp, 41313);
    let row = format!("{id} 4242 STOPPED");
    set_tasks(&temp, std::slice::from_ref(&row));
    // The agent exited on its own, so nothing recorded an explicit stop, and
    // the retirement's stop fails in nerdctl's network cleanup before it can.
    // Its VM is gone, so the failed stop is accepted. Under unless-stopped,
    // containerd's restart monitor would still restart it, and the STOPPED
    // task is all that holds the fence.
    std::fs::write(
        temp.path().join(format!(
            "fake-state/{}.stop-fails-early",
            plan.container_name
        )),
        "1",
    )
    .unwrap();

    let error = launcher
        .retire_runtime(&lease, &mut || Ok(()))
        .unwrap_err()
        .to_string();

    assert!(error.contains("did not become absent"), "{error}");
    assert!(delete_calls(&temp).is_empty());
    assert_eq!(tasks(&temp), format!("{row}\n"));
    assert!(!temp.path().join("borg-remote/archive.zip").exists());
}

#[test]
fn restart_labels_decide_whether_the_monitor_leaves_a_container_stopped() {
    let labels = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(key, value)| (format!("containerd.io/restart.{key}"), value.to_string()))
            .collect::<BTreeMap<_, _>>()
    };
    for (pairs, leaves_stopped) in [
        (vec![], true),
        (vec![("status", "stopped"), ("policy", "always")], true),
        (
            vec![
                ("status", "running"),
                ("policy", "unless-stopped"),
                ("explicitly-stopped", "true"),
            ],
            true,
        ),
        (
            vec![("status", "running"), ("policy", "unless-stopped")],
            false,
        ),
        (
            vec![
                ("status", "running"),
                ("policy", "unless-stopped"),
                ("explicitly-stopped", "false"),
            ],
            false,
        ),
        (
            vec![
                ("status", "running"),
                ("policy", "always"),
                ("explicitly-stopped", "true"),
            ],
            false,
        ),
        (
            vec![
                ("status", "running"),
                ("policy", "on-failure"),
                ("explicitly-stopped", "true"),
            ],
            false,
        ),
        (vec![("status", "running")], false),
    ] {
        assert_eq!(
            restart_monitor_leaves_stopped(&labels(&pairs)),
            leaves_stopped,
            "{pairs:?}"
        );
    }
}

#[test]
fn retirement_remakes_a_zip_an_interrupted_attempt_left_unverifiable() {
    let temp = tempfile::tempdir().unwrap();
    let (mut launcher, plan, lease, id) = stopped_runtime(&temp, 41314);
    set_tasks(&temp, &[format!("{id} 4242 STOPPED")]);
    // An earlier attempt saw the agent running, recorded its principal in
    // the ZIP, and the Runner died before writing retirement progress. The
    // stopped agent can no longer give that principal.
    let (_, zip, _) = launcher
        .retirement_staging_paths(&lease.request.id)
        .unwrap();
    let earlier = launcher
        .retirement_context(&lease, Some("npub1earlierattempt".to_string()))
        .unwrap();
    create_recovery_zip(&plan.state_root, &zip, &earlier).unwrap();

    let receipt = launcher.retire_runtime(&lease, &mut || Ok(())).unwrap();

    assert_eq!(receipt.request_id, lease.request.id);
    let archived = verify_recovery_zip(&temp.path().join("borg-remote/archive.zip"), None).unwrap();
    assert_eq!(archived.manifest.agent_principal, None);
    assert!(launcher.inspect(&plan.container_name).unwrap().is_none());
}

#[test]
fn identity_is_reached_only_through_real_directories() {
    let temp = tempfile::tempdir().unwrap();
    // A host directory holding an identity.json, outside every tree.
    let host = temp.path().join("host-identity");
    std::fs::create_dir_all(host.join("identity")).unwrap();
    std::fs::write(host.join("identity.json"), b"host").unwrap();
    std::fs::write(host.join("identity/identity.json"), b"host").unwrap();

    let real = temp.path().join("real");
    stage_relocation_tree(&real);
    assert!(staged_agent_identity_metadata(&real).unwrap().is_file());

    // Absolute links resolve to host paths on the host, so neither the
    // identity directory nor the agent home may be one.
    let linked_identity = temp.path().join("linked-identity");
    std::fs::create_dir_all(linked_identity.join("agent")).unwrap();
    std::os::unix::fs::symlink(&host, linked_identity.join("agent/identity")).unwrap();
    let linked_agent = temp.path().join("linked-agent");
    std::fs::create_dir_all(&linked_agent).unwrap();
    std::os::unix::fs::symlink(&host, linked_agent.join("agent")).unwrap();
    for root in [&linked_identity, &linked_agent] {
        let error = staged_agent_identity_metadata(root)
            .unwrap_err()
            .to_string();
        assert!(error.contains("real directories"), "{error}");
    }
}
