//! The retirement fence against the STOPPED task nerdctl leaves behind.
use super::*;

/// A `ctr` whose task list is the file `ctr-tasks` beside it, one
/// `<id> <pid> <STATUS>` row per task. `tasks delete` refuses a task that
/// is not STOPPED, as containerd's client does without `--force`. Every call
/// is appended to `ctr-calls`.
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
