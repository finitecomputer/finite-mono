//! FIN-463's hold: a Runtime the mover stopped for a move is started by
//! nothing but the mover's own `restart-exact` (staff gave it back), and a
//! Runtime marked moved by nothing at all. Every start path is tried.

use super::*;

const RUN: &str = "fin463-hold";
const HOST: &str = "fin463-hold-host";
const TARGET_HOST: &str = "fin463-hold-target";

fn owner(project_id: &str) -> RequestRuntimeRestartInput {
    RequestRuntimeRestartInput {
        verified_email: format!("{RUN}-owner@finite.vip"),
        workos_user_id: format!("workos_{RUN}_owner"),
        project_id: project_id.to_string(),
        now: None,
    }
}

fn admin(project_id: &str) -> AdminRuntimeControlInput {
    AdminRuntimeControlInput {
        admin_verified_email: format!("{RUN}-admin@finite.vip"),
        admin_workos_user_id: format!("workos_{RUN}_admin"),
        project_id: project_id.to_string(),
        now: None,
    }
}

fn exact(project_id: &str, runtime_id: &str) -> AdminRuntimeStopExactInput {
    AdminRuntimeStopExactInput {
        admin_verified_email: format!("{RUN}-admin@finite.vip"),
        admin_workos_user_id: format!("workos_{RUN}_admin"),
        project_id: project_id.to_string(),
        expected_agent_runtime_id: runtime_id.to_string(),
        expected_source_host_id: HOST.to_string(),
        expected_source_machine_id: format!("{RUN}-agent-001"),
        now: None,
    }
}

/// Lease and complete a control request on the Runtime's host, as the
/// Runner does.
async fn run_control(store: &TestDb, request_id: &str, token: &str) {
    let lease = store
        .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
            runner_id: format!("runner-{RUN}"),
            lease_token: token.to_string(),
            lease_seconds: Some(60),
            source_host_id: Some(HOST.to_string()),
            runner_capacity: Some(crate::RunnerLeaseCapacity {
                runner_classes: vec![crate::RunnerClass::Kata],
                runtime_capabilities: Some(kata_runtime_capabilities()),
                ..crate::RunnerLeaseCapacity::default()
            }),
            now: None,
        })
        .await
        .unwrap()
        .expect("the control leases to the Runtime's host");
    assert_eq!(lease.request.id, request_id);
    store
        .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
            trial_archive: None,
            request_id: request_id.to_string(),
            runner_id: format!("runner-{RUN}"),
            lease_token: token.to_string(),
            runtime_artifact_id: None,
            state_schema_version: None,
            runtime_capabilities: None,
            runtime_host: None,
            published_app_urls: None,
            retirement_snapshot: None,
            now: None,
        })
        .await
        .unwrap();
}

/// A launched Runtime that supports every control, so no path is refused
/// for a capability before the hold is asked.
async fn launched(store: &TestDb) -> (String, String) {
    let (project_id, runtime_id) = launched_kata_runtime(store, RUN, HOST).await;
    let everything =
        serde_json::to_string(&RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            restart: true,
            recover_known_good_chat: true,
            runtime_upgrade: true,
            stop: true,
            runtime_retirement: true,
            trial_archive: false,
        }))
        .unwrap();
    store
        .exec(&format!(
            "UPDATE agent_runtimes SET runtime_capabilities = '{everything}'::jsonb
             WHERE id = '{runtime_id}'"
        ))
        .await;
    (project_id, runtime_id)
}

/// The mover's stop, run to stopped.
async fn stopped_for_a_move(store: &TestDb) -> (String, String) {
    let (project_id, runtime_id) = launched(store).await;
    let stop = store
        .admin_request_runtime_stop_exact(exact(&project_id, &runtime_id))
        .await
        .unwrap();
    run_control(store, &stop.id, "ctl-move-stop").await;
    (project_id, runtime_id)
}

/// Every way a start reaches Core.
const STARTS: [&str; 7] = [
    "owner restart (dashboard)",
    "owner recover (dashboard)",
    "staff restart",
    "staff recover",
    "staff upgrade",
    "trial resume (billing sweep)",
    "staff relocate (trial restore takes the same relocation insert)",
];

/// Start the Runtime by `path`: what Core says.
async fn start(store: &TestDb, path: &str, project_id: &str, runtime_id: &str) -> String {
    let said = |result: CoreResult<String>| match result {
        Ok(kind) => format!("started: {kind}"),
        Err(error) => format!("{error:?}"),
    };
    let control = |result: CoreResult<RuntimeControlRequest>| {
        said(result.map(|request| request.kind.as_str().to_string()))
    };
    match path {
        "owner restart (dashboard)" => {
            control(store.request_runtime_restart(owner(project_id)).await)
        }
        "owner recover (dashboard)" => control(
            store
                .request_runtime_recover_known_good_chat(owner(project_id))
                .await,
        ),
        "staff restart" => control(store.admin_request_runtime_restart(admin(project_id)).await),
        "staff recover" => control(
            store
                .admin_request_runtime_recover_known_good_chat(admin(project_id))
                .await,
        ),
        "staff upgrade" => {
            store
                .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                    id: format!("artifact-{RUN}-v2"),
                    kind: RuntimeArtifactKind::OciImage,
                    reference: format!(
                        "ghcr.io/finitecomputer/finite-agent-runtime:{RUN}-v2@sha256:{}",
                        "7".repeat(64)
                    ),
                    version_label: format!("{RUN}-v2"),
                    source_git_sha: None,
                    finitec_version: None,
                    hermes_source_ref: None,
                    finite_platform_plugin_ref: None,
                    state_schema_version: "state-v1".to_string(),
                    base_image: None,
                    canary_runtime_id: None,
                    recover_known_good_chat: false,
                    promoted: true,
                    now: None,
                })
                .await
                .unwrap();
            control(
                store
                    .admin_request_runtime_upgrade(AdminRuntimeUpgradeInput {
                        admin_verified_email: format!("{RUN}-admin@finite.vip"),
                        admin_workos_user_id: format!("workos_{RUN}_admin"),
                        project_id: project_id.to_string(),
                        target_runtime_artifact_id: format!("artifact-{RUN}-v2"),
                        now: None,
                    })
                    .await,
            )
        }
        "trial resume (billing sweep)" => {
            // Billing's sweep enqueues exactly this.
            let mut client = store.connection().await.unwrap();
            let tx = client.transaction().await.unwrap();
            let project = select_project(&*tx, project_id).await.unwrap().unwrap();
            let result = postgres_enqueue_runtime_control_request_bound(
                &*tx,
                &project,
                &project.owner_user_id,
                RuntimeControlKind::Restart,
                None,
                &current_time_iso().unwrap(),
                None,
            )
            .await;
            if result.is_ok() {
                tx.commit().await.unwrap();
            }
            control(result)
        }
        _ => said(
            store
                .admin_request_runtime_relocate_exact(AdminRuntimeRelocateExactInput {
                    admin_verified_email: format!("{RUN}-admin@finite.vip"),
                    admin_workos_user_id: format!("workos_{RUN}_admin"),
                    project_id: project_id.to_string(),
                    expected_agent_runtime_id: runtime_id.to_string(),
                    expected_source_host_id: HOST.to_string(),
                    expected_source_machine_id: format!("{RUN}-agent-001"),
                    target_source_host_id: TARGET_HOST.to_string(),
                    expected_agent_npub: format!("npub1{}", "q".repeat(58)),
                    durable_state_manifest_sha256: "b".repeat(64),
                    operator_observed_compute_absent: true,
                    now: None,
                })
                .await
                .map(|_| "relocation".to_string()),
        ),
    }
}

/// Refused by the hold (by name, so this test also runs against a Core
/// without it).
fn held<T>(result: &CoreResult<T>) -> bool {
    matches!(result, Err(error) if format!("{error:?}") == "RuntimeMoving")
}

/// Each start path, each on a Runtime of its own set up by `setup`: the
/// ones that weren't refused by the hold, and what they said.
async fn not_refused<F, Fut>(setup: F) -> Vec<String>
where
    F: Fn(TestDb) -> Fut + Copy,
    Fut: std::future::Future<Output = (TestDb, String, String)>,
{
    let mut started = Vec::new();
    for path in STARTS {
        let said = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let into = said.clone();
        with_isolated_postgres(|store| async move {
            let (store, project_id, runtime_id) = setup(store).await;
            *into.lock().unwrap() = start(&store, path, &project_id, &runtime_id).await;
        })
        .await;
        let said = said.lock().unwrap().clone();
        if said != "RuntimeMoving" {
            started.push(format!("{path}: {said}"));
        }
    }
    started
}

#[tokio::test]
async fn postgres_a_runtime_stopped_for_a_move_is_started_by_no_other_path() {
    let started = not_refused(|store| async move {
        let (project_id, runtime_id) = stopped_for_a_move(&store).await;
        (store, project_id, runtime_id)
    })
    .await;
    assert!(started.is_empty(), "held, yet:\n{}", started.join("\n"));
}

/// The control: without a move, every path starts it, so each one above is
/// refused by the hold and nothing else.
#[tokio::test]
async fn postgres_a_runtime_never_stopped_for_a_move_is_started_by_every_path() {
    let started = not_refused(|store| async move {
        let (project_id, runtime_id) = launched(&store).await;
        let stop = store
            .request_runtime_stop(owner(&project_id))
            .await
            .unwrap();
        run_control(&store, &stop.id, "ctl-owner-stop").await;
        (store, project_id, runtime_id)
    })
    .await;
    assert_eq!(started.len(), STARTS.len(), "{started:#?}");
    for line in &started {
        assert!(
            line.contains(": started: ") || line.ends_with("started: relocation"),
            "{line}"
        );
    }
}

/// The mover's stop asked again keeps the hold; nothing but its stop is
/// enqueued.
#[tokio::test]
async fn postgres_the_movers_stop_again_keeps_the_hold() {
    with_isolated_postgres(|store| async move {
        let (project_id, runtime_id) = stopped_for_a_move(&store).await;
        store
            .admin_request_runtime_stop_exact(exact(&project_id, &runtime_id))
            .await
            .unwrap();
        assert!(held(
            &store.request_runtime_restart(owner(&project_id)).await
        ));
        assert_eq!(store.all("runtime_control_requests").await.len(), 2);
    })
    .await;
}

#[tokio::test]
async fn postgres_a_held_runtime_may_still_be_stopped_and_destroyed() {
    with_isolated_postgres(|store| async move {
        let (project_id, _) = stopped_for_a_move(&store).await;
        let stop = store
            .request_runtime_stop(owner(&project_id))
            .await
            .unwrap();
        assert_eq!(stop.kind, RuntimeControlKind::Stop);
        run_control(&store, &stop.id, "ctl-owner-stop").await;
        let destroy = store
            .request_runtime_destroy(owner(&project_id))
            .await
            .unwrap();
        assert_eq!(destroy.kind, RuntimeControlKind::Destroy);
    })
    .await;
}

/// Staff gave the old agent back: the mover's `restart-exact` starts it and
/// clears the hold, so the owner's dashboard may start it again after.
#[tokio::test]
async fn postgres_the_movers_restart_gives_it_back() {
    with_isolated_postgres(|store| async move {
        let (project_id, runtime_id) = stopped_for_a_move(&store).await;
        let restart = store
            .admin_request_runtime_restart_exact(exact(&project_id, &runtime_id))
            .await
            .unwrap();
        assert_eq!(restart.kind, RuntimeControlKind::Restart);
        run_control(&store, &restart.id, "ctl-move-restart").await;
        let again = store.request_runtime_restart(owner(&project_id)).await;
        assert!(
            again
                .as_ref()
                .is_ok_and(|request| request.kind == RuntimeControlKind::Restart),
            "given back, the owner may restart it: {again:?}"
        );
    })
    .await;
}

/// Marked moved, it stays held for good: even the mover's restart is
/// refused, and the moved mark holds it with no hold row at all.
#[tokio::test]
async fn postgres_a_runtime_marked_moved_is_started_by_nothing() {
    async fn moved(store: TestDb) -> (TestDb, String, String) {
        let (project_id, runtime_id) = stopped_for_a_move(&store).await;
        store
            .admin_mark_runtime_moved_exact(AdminMarkRuntimeMovedExactInput {
                admin_verified_email: format!("{RUN}-admin@finite.vip"),
                admin_workos_user_id: format!("workos_{RUN}_admin"),
                project_id: project_id.clone(),
                expected_agent_runtime_id: runtime_id.clone(),
                expected_source_host_id: HOST.to_string(),
                expected_source_machine_id: format!("{RUN}-agent-001"),
                moved: RuntimeMoved::Migrated,
                agent_npub: None,
                now: None,
            })
            .await
            .unwrap();
        let restart = store
            .admin_request_runtime_restart_exact(exact(&project_id, &runtime_id))
            .await;
        assert!(held(&restart), "{restart:?}");
        store
            .exec(&format!(
                "DELETE FROM agent_runtime_move_holds WHERE agent_runtime_id = '{runtime_id}'"
            ))
            .await;
        (store, project_id, runtime_id)
    }
    let started = not_refused(moved).await;
    assert!(started.is_empty(), "moved, yet:\n{}", started.join("\n"));
}

fn mark(project_id: &str, runtime_id: &str) -> AdminMarkRuntimeMovedExactInput {
    AdminMarkRuntimeMovedExactInput {
        admin_verified_email: format!("{RUN}-admin@finite.vip"),
        admin_workos_user_id: format!("workos_{RUN}_admin"),
        project_id: project_id.to_string(),
        expected_agent_runtime_id: runtime_id.to_string(),
        expected_source_host_id: HOST.to_string(),
        expected_source_machine_id: format!("{RUN}-agent-001"),
        moved: RuntimeMoved::Migrated,
        agent_npub: None,
        now: None,
    }
}

/// A hold or a moved mark is taken only when no start is queued or running,
/// so a held Runtime never has one a Runner could lease: a relocation, or a
/// restart, recovery or upgrade asked before.
#[tokio::test]
async fn postgres_no_hold_or_mark_is_taken_over_a_start_already_asked() {
    let mut taken = Vec::new();
    for (start_path, take) in [
        ("staff relocate", "stop-exact"),
        ("staff relocate", "mark moved"),
        ("owner restart (dashboard)", "mark moved"),
        ("staff upgrade", "mark moved"),
    ] {
        let said = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let into = said.clone();
        with_isolated_postgres(|store| async move {
            let (project_id, runtime_id) = launched(&store).await;
            let asked = start(&store, start_path, &project_id, &runtime_id).await;
            assert!(asked.starts_with("started"), "{start_path}: {asked}");
            let result = match take {
                "stop-exact" => store
                    .admin_request_runtime_stop_exact(exact(&project_id, &runtime_id))
                    .await
                    .map(drop),
                _ => store
                    .admin_mark_runtime_moved_exact(mark(&project_id, &runtime_id))
                    .await
                    .map(drop),
            };
            let holds = store
                .query_json("SELECT to_jsonb(h) FROM agent_runtime_move_holds h", &[])
                .await
                .len();
            let moves = store
                .query_json("SELECT to_jsonb(m) FROM agent_runtime_moves m", &[])
                .await
                .len();
            *into.lock().unwrap() = format!("{result:?}, holds {holds}, moves {moves}");
        })
        .await;
        let said = said.lock().unwrap().clone();
        if said != "Err(RuntimeControlOperationConflict), holds 0, moves 0" {
            taken.push(format!("{start_path} then {take}: {said}"));
        }
    }
    assert!(
        taken.is_empty(),
        "taken over a start:\n{}",
        taken.join("\n")
    );
}

/// The moved Runtime's refused restart leaves its hold where it was.
#[tokio::test]
async fn postgres_a_refused_restart_keeps_the_hold() {
    with_isolated_postgres(|store| async move {
        let (project_id, runtime_id) = stopped_for_a_move(&store).await;
        store
            .admin_mark_runtime_moved_exact(mark(&project_id, &runtime_id))
            .await
            .unwrap();
        let restart = store
            .admin_request_runtime_restart_exact(exact(&project_id, &runtime_id))
            .await;
        assert!(held(&restart), "{restart:?}");
        assert_eq!(
            store
                .query_json("SELECT to_jsonb(h) FROM agent_runtime_move_holds h", &[])
                .await
                .len(),
            1
        );
    })
    .await;
}
