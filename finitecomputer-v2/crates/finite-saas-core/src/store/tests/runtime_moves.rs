//! FIN-463: the operator stop of one exact Runtime, and the `moved` mark
//! the finite.chat mover's roster leaves out.

use super::*;

const RUN: &str = "fin463-move";
const HOST: &str = "fin463-move-host";
/// A canonical npub (32 bytes of 0x01).
const NPUB: &str = "npub1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqs8j9gdm";
/// Another canonical npub (32 bytes of 0x02).
const OTHER_NPUB: &str = "npub1qgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpq2yz0cn";

fn admin() -> (String, String) {
    (
        format!("{RUN}-admin@finite.vip"),
        format!("workos_{RUN}_admin"),
    )
}

fn stop_input(project_id: &str, runtime_id: &str, host: &str) -> AdminRuntimeStopExactInput {
    let (admin_verified_email, admin_workos_user_id) = admin();
    AdminRuntimeStopExactInput {
        admin_verified_email,
        admin_workos_user_id,
        project_id: project_id.to_string(),
        expected_agent_runtime_id: runtime_id.to_string(),
        expected_source_host_id: host.to_string(),
        expected_source_machine_id: format!("{RUN}-agent-001"),
        now: None,
    }
}

fn mark_input(
    project_id: &str,
    runtime_id: &str,
    moved: RuntimeMoved,
    agent_npub: Option<&str>,
) -> AdminMarkRuntimeMovedExactInput {
    let (admin_verified_email, admin_workos_user_id) = admin();
    AdminMarkRuntimeMovedExactInput {
        admin_verified_email,
        admin_workos_user_id,
        project_id: project_id.to_string(),
        expected_agent_runtime_id: runtime_id.to_string(),
        expected_source_host_id: HOST.to_string(),
        expected_source_machine_id: format!("{RUN}-agent-001"),
        moved,
        agent_npub: agent_npub.map(str::to_string),
        now: None,
    }
}

async fn moves(store: &TestDb) -> Vec<serde_json::Value> {
    store
        .query_json(
            "SELECT jsonb_build_object('runtime', agent_runtime_id, 'project', project_id,
                    'moved', moved, 'npub', agent_npub)
             FROM agent_runtime_moves ORDER BY agent_runtime_id",
            &[],
        )
        .await
}

#[tokio::test]
async fn postgres_admin_stop_exact_stops_only_the_bound_runtime() {
    with_isolated_postgres(|store| async move {
        let (project_id, runtime_id) = launched_kata_runtime(&store, RUN, HOST).await;

        // A binding that no longer matches the active Runtime enqueues
        // nothing.
        let mismatch = store
            .admin_request_runtime_stop_exact(stop_input(&project_id, &runtime_id, "other-host"))
            .await
            .unwrap_err();
        assert!(
            matches!(mismatch, CoreError::RuntimeSpecMismatch),
            "{mismatch:?}"
        );
        assert!(store.all("runtime_control_requests").await.is_empty());

        let stop = store
            .admin_request_runtime_stop_exact(stop_input(&project_id, &runtime_id, HOST))
            .await
            .unwrap();
        assert_eq!(stop.kind, RuntimeControlKind::Stop);
        assert_eq!(stop.agent_runtime_id, runtime_id);
        assert_eq!(stop.status, RuntimeControlRequestStatus::Requested);
        // Asking again while it is in flight returns the same request.
        let again = store
            .admin_request_runtime_stop_exact(stop_input(&project_id, &runtime_id, HOST))
            .await
            .unwrap();
        assert_eq!(again.id, stop.id);

        let lease = store
            .lease_runtime_control_request(LeaseRuntimeControlRequestInput {
                runner_id: format!("runner-{RUN}"),
                lease_token: format!("ctl-stop-{RUN}"),
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
            .expect("the stop leases to the Runtime's host");
        assert_eq!(lease.request.id, stop.id);
        let completed = store
            .complete_runtime_control_request(CompleteRuntimeControlRequestInput {
                trial_archive: None,
                request_id: stop.id.clone(),
                runner_id: format!("runner-{RUN}"),
                lease_token: format!("ctl-stop-{RUN}"),
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
        assert_eq!(completed.status, RuntimeControlRequestStatus::Stopped);

        let audited = store
            .query_json(
                "SELECT to_jsonb(t) FROM finite_private_admin_audit_events t
                 WHERE action = 'runtime.admin_stop' AND target_id = $1",
                &[&runtime_id],
            )
            .await;
        assert_eq!(audited.len(), 2, "each operator call is audited");
    })
    .await;
}

#[tokio::test]
async fn postgres_mark_moved_exact_is_idempotent_and_only_moves_forward() {
    with_isolated_postgres(|store| async move {
        let (project_id, runtime_id) = launched_kata_runtime(&store, RUN, HOST).await;

        // Phase 2: archived at the cutoff, no finite.chat agent yet.
        let archived = store
            .admin_mark_runtime_moved_exact(mark_input(
                &project_id,
                &runtime_id,
                RuntimeMoved::Archived,
                None,
            ))
            .await
            .unwrap();
        assert_eq!((archived.moved, archived.agent_npub.as_deref()), (RuntimeMoved::Archived, None));
        let again = store
            .admin_mark_runtime_moved_exact(mark_input(
                &project_id,
                &runtime_id,
                RuntimeMoved::Archived,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(again, archived, "the same mark returns the stored receipt");

        // A late import makes it migrated.
        let migrated = store
            .admin_mark_runtime_moved_exact(mark_input(
                &project_id,
                &runtime_id,
                RuntimeMoved::Migrated,
                Some(NPUB),
            ))
            .await
            .unwrap();
        assert_eq!((migrated.moved, migrated.agent_npub.as_deref()), (RuntimeMoved::Migrated, Some(NPUB)));
        assert_eq!(
            store
                .admin_mark_runtime_moved_exact(mark_input(
                    &project_id,
                    &runtime_id,
                    RuntimeMoved::Migrated,
                    Some(NPUB),
                ))
                .await
                .unwrap(),
            migrated
        );

        // Nothing moves it back or to another agent.
        for refused in [
            mark_input(&project_id, &runtime_id, RuntimeMoved::Archived, None),
            mark_input(&project_id, &runtime_id, RuntimeMoved::Migrated, Some(OTHER_NPUB)),
        ] {
            let error = store.admin_mark_runtime_moved_exact(refused).await.unwrap_err();
            assert!(matches!(error, CoreError::RuntimeMoveConflict), "{error:?}");
        }
        assert_eq!(
            moves(&store).await,
            vec![json!({"runtime": runtime_id, "project": project_id, "moved": "migrated", "npub": NPUB})]
        );
        let audited = store
            .query_json(
                "SELECT to_jsonb(t) FROM finite_private_admin_audit_events t
                 WHERE action = 'runtime.admin_mark_moved' AND target_id = $1",
                &[&runtime_id],
            )
            .await;
        assert_eq!(audited.len(), 2, "only the two changes are audited");
    })
    .await;
}

#[tokio::test]
async fn postgres_mark_moved_exact_refuses_a_wrong_binding_or_npub() {
    with_isolated_postgres(|store| async move {
        let (project_id, runtime_id) = launched_kata_runtime(&store, RUN, HOST).await;
        let mut wrong_host =
            mark_input(&project_id, &runtime_id, RuntimeMoved::Migrated, Some(NPUB));
        wrong_host.expected_source_host_id = "other-host".to_string();
        let mut wrong_machine =
            mark_input(&project_id, &runtime_id, RuntimeMoved::Migrated, Some(NPUB));
        wrong_machine.expected_source_machine_id = "other-machine".to_string();
        let wrong_project = mark_input(
            "other-project",
            &runtime_id,
            RuntimeMoved::Migrated,
            Some(NPUB),
        );
        let bad_npubs = [
            mark_input(&project_id, &runtime_id, RuntimeMoved::Migrated, None),
            mark_input(&project_id, &runtime_id, RuntimeMoved::Archived, Some(NPUB)),
            mark_input(
                &project_id,
                &runtime_id,
                RuntimeMoved::Migrated,
                Some(&NPUB.to_uppercase()),
            ),
            mark_input(
                &project_id,
                &runtime_id,
                RuntimeMoved::Migrated,
                Some(&NPUB[..60]),
            ),
            mark_input(
                &project_id,
                &runtime_id,
                RuntimeMoved::Migrated,
                Some("nsec1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqstywftw"),
            ),
            mark_input(
                &project_id,
                &runtime_id,
                RuntimeMoved::Migrated,
                Some("npub1notbech32"),
            ),
        ];
        for (input, expected) in [
            (wrong_host, "mismatch"),
            (wrong_machine, "mismatch"),
            (wrong_project, "not found"),
        ]
        .into_iter()
        .chain(bad_npubs.into_iter().map(|input| (input, "npub")))
        {
            let error = store
                .admin_mark_runtime_moved_exact(input)
                .await
                .unwrap_err();
            let matched = match expected {
                "mismatch" => matches!(error, CoreError::RuntimeSpecMismatch),
                "not found" => matches!(error, CoreError::ProjectRuntimeNotFound),
                _ => matches!(error, CoreError::InvalidRuntimeMoveNpub),
            };
            assert!(matched, "{expected}: {error:?}");
        }
        assert!(
            moves(&store).await.is_empty(),
            "no refused mark writes a row"
        );
    })
    .await;
}

/// The table's own constraints hold without the writer, and the migration
/// reapplies at every startup without touching its rows.
#[tokio::test]
async fn postgres_runtime_moves_constraints_and_reapply() {
    with_isolated_postgres(|store| async move {
        let (project_id, runtime_id) = launched_kata_runtime(&store, RUN, HOST).await;
        store
            .admin_mark_runtime_moved_exact(mark_input(
                &project_id,
                &runtime_id,
                RuntimeMoved::Migrated,
                Some(NPUB),
            ))
            .await
            .unwrap();
        let before = moves(&store).await;
        let client = store.connection().await.unwrap();
        client.batch_execute(CORE_SCHEMA_SQL).await.unwrap();
        client.batch_execute(CORE_SCHEMA_SQL).await.unwrap();
        assert_eq!(moves(&store).await, before);
        for (moved, npub) in [
            ("migrated", None),
            ("archived", Some(NPUB)),
            ("migrated", Some("npub1short")),
            ("gone", None),
        ] {
            let result = client
                .execute(
                    "UPDATE agent_runtime_moves SET moved = $2, agent_npub = $3
                     WHERE agent_runtime_id = $1",
                    &[&runtime_id, &moved, &npub],
                )
                .await;
            assert!(
                result.is_err(),
                "{moved} {npub:?} must violate a constraint"
            );
        }
        let orphan = client
            .execute(
                "INSERT INTO agent_runtime_moves (agent_runtime_id, project_id, moved, moved_at)
                 VALUES ('no-such-runtime', $1, 'archived', now())",
                &[&project_id],
            )
            .await;
        assert!(orphan.is_err(), "a mark needs a real Runtime");
    })
    .await;
}

/// The dashboard's redirect: a person is sent to finite.chat once every
/// agent they belong to has moved, offboarded ones included, and not while
/// any agent they can see still runs here.
#[tokio::test]
async fn postgres_moved_to_finite_chat_once_every_agent_a_person_sees_has_moved() {
    with_isolated_postgres(|store| async move {
        let owner = format!("workos_{RUN}_owner");
        let moved = |who: String| {
            let store = &store;
            async move { store.moved_to_finite_chat_for_workos_user(&who).await.unwrap() }
        };
        let (project_id, runtime_id) = launched_kata_runtime(&store, RUN, HOST).await;
        assert!(!moved(owner.clone()).await, "an agent still running here");
        assert!(!moved("workos_nobody".into()).await, "no agents at all");

        // They also belong to another person's agent, which still runs.
        const OTHER: &str = "fin463-other";
        let (other_project, other_runtime) =
            launched_kata_runtime(&store, OTHER, "fin463-other-host").await;
        let client = store.connection().await.unwrap();
        let shared = client
            .execute(
                "INSERT INTO project_room_memberships (id, project_id, chat_identity_id, role, created_at)
                 SELECT 'membership-fin463-shared', $1, identity.id, 'member', now()
                 FROM chat_identities AS identity
                 JOIN users AS person ON person.id = identity.user_id
                 WHERE person.workos_user_id = $2",
                &[&other_project, &owner],
            )
            .await
            .unwrap();
        assert_eq!(shared, 1);

        store
            .admin_mark_runtime_moved_exact(mark_input(
                &project_id,
                &runtime_id,
                RuntimeMoved::Migrated,
                Some(NPUB),
            ))
            .await
            .unwrap();
        assert!(!moved(owner.clone()).await, "their own moved, a shared one not");
        // What offboarding leaves: the membership archived, the link inactive.
        client
            .execute(
                "UPDATE project_room_memberships SET archived_at = now() WHERE project_id = $1",
                &[&project_id],
            )
            .await
            .unwrap();
        client
            .execute(
                "UPDATE project_runtime_links SET active = FALSE WHERE agent_runtime_id = $1",
                &[&runtime_id],
            )
            .await
            .unwrap();
        assert!(!moved(owner.clone()).await, "offboarded, and the shared one still runs");

        let (admin_verified_email, admin_workos_user_id) = admin();
        store
            .admin_mark_runtime_moved_exact(AdminMarkRuntimeMovedExactInput {
                admin_verified_email,
                admin_workos_user_id,
                project_id: other_project.clone(),
                expected_agent_runtime_id: other_runtime.clone(),
                expected_source_host_id: "fin463-other-host".into(),
                expected_source_machine_id: format!("{OTHER}-agent-001"),
                moved: RuntimeMoved::Archived,
                agent_npub: None,
                now: None,
            })
            .await
            .unwrap();
        assert!(moved(owner).await, "every agent they saw has moved");
        assert!(moved(format!("workos_{OTHER}_owner")).await);
    })
    .await;
}
