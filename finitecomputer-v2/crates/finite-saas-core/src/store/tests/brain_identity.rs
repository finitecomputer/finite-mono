//! Brain identity descriptions v1 against real Postgres: hosted observations,
//! scoped disclosure, lifecycle and conflict rules.

use super::*;
use crate::brain_identity::{
    BrainAccountObservationRequest, BrainIdentityDescriptionsRequest, DESCRIPTIONS_VERSION,
    DescribedKind, DescriptionState, IdentityDescription, OBSERVATION_ISSUER, OBSERVATION_VERSION,
    ObservationActionKind, ObservationOutcome, npub_for_hex,
};
use crate::store::BrainObservationError;

const SERVER: &str = "https://brain.test";
const BRAIN: &str = "brain_alpha";

fn key(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn now() -> time::OffsetDateTime {
    time::OffsetDateTime::now_utc()
}

fn stamp(at: time::OffsetDateTime) -> String {
    at.format(&Rfc3339).unwrap()
}

async fn account(store: &CoreStore, email: &str) -> String {
    store
        .link_verified_user(LinkVerifiedUserInput {
            verified_email: email.to_string(),
            workos_user_id: format!("workos_{}", email.replace(['@', '.'], "_")),
            now: None,
        })
        .await
        .unwrap()
        .id
}

fn human_observation(operation: &str, human: &str, brain: &str) -> BrainAccountObservationRequest {
    BrainAccountObservationRequest {
        version: OBSERVATION_VERSION.to_string(),
        operation_id: format!("operation_{operation}_0000"),
        brain_server: SERVER.to_string(),
        brain_id: brain.to_string(),
        observed_at: stamp(now()),
        action_kind: ObservationActionKind::HumanHostedAction,
        observed_human_public_key_hex: Some(human.to_string()),
        participating_public_key_hex: human.to_string(),
    }
}

fn agent_observation(operation: &str, agent: &str) -> BrainAccountObservationRequest {
    BrainAccountObservationRequest {
        action_kind: ObservationActionKind::OwnedAgentHostedAction,
        observed_human_public_key_hex: None,
        participating_public_key_hex: agent.to_string(),
        ..human_observation(operation, agent, BRAIN)
    }
}

async fn observe(
    store: &CoreStore,
    user: &str,
    request: &BrainAccountObservationRequest,
) -> Result<ObservationOutcome, BrainObservationError> {
    observe_as(store, user, OBSERVATION_ISSUER, request, now()).await
}

/// Observe as the account's verified WorkOS subject, as the route does.
async fn observe_as(
    store: &CoreStore,
    user: &str,
    issuer: &str,
    request: &BrainAccountObservationRequest,
    at: time::OffsetDateTime,
) -> Result<ObservationOutcome, BrainObservationError> {
    let workos = store.user(user).await.unwrap().workos_user_id.unwrap();
    store
        .record_brain_account_observation(&workos, issuer, request, at)
        .await
}

async fn describe(store: &CoreStore, brain: &str, keys: &[&str]) -> Vec<IdentityDescription> {
    store
        .describe_brain_identities(&BrainIdentityDescriptionsRequest {
            version: DESCRIPTIONS_VERSION.to_string(),
            brain_server: SERVER.to_string(),
            brain_id: brain.to_string(),
            requested_by_public_key_hex: key(0xee),
            public_keys_hex: keys.iter().map(|key| key.to_string()).collect(),
        })
        .await
        .unwrap()
}

async fn project(db: &TestDb, id: &str, owner: &str, name: &str) {
    let org = db.store.personal_org_by_owner(owner).await.unwrap().id;
    db.store
        .exec(&format!(
            "INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, agent_email,
                                   created_at, updated_at)
             VALUES ('{id}', '{org}', '{owner}', '{name}', '{id}@finite.vip', NOW(), NOW())"
        ))
        .await;
}

/// One runtime incarnation pinned to `agent_hex`, optionally actively linked.
async fn runtime(
    db: &TestDb,
    id: &str,
    project: &str,
    agent_hex: &str,
    active: bool,
    phase: Option<&str>,
) {
    let npub = npub_for_hex(agent_hex).unwrap();
    pinned_runtime(db, id, project, &npub, phase).await;
    db.store
        .exec(&format!(
            "INSERT INTO project_runtime_links (id, project_id, agent_runtime_id, active, created_at)
             VALUES ('link_{id}', '{project}', '{id}', {active}, NOW())"
        ))
        .await;
}

async fn pinned_runtime(db: &TestDb, id: &str, project: &str, pin: &str, phase: Option<&str>) {
    let phase = phase.map_or("NULL".to_string(), |phase| format!("'{phase}'"));
    db.store
        .exec(&format!(
            "INSERT INTO agent_runtimes (id, project_id, source_host_id, source_machine_id,
                                         source_import_key, host_facts, created_at, updated_at,
                                         health_reporting_npub, offboarding_phase)
             VALUES ('{id}', '{project}', 'host-test', '{id}', 'host-test:{id}', '{{}}'::jsonb,
                     NOW(), NOW(), '{pin}', {phase})"
        ))
        .await;
}

async fn count(db: &TestDb, sql: &str) -> i64 {
    db.query_json(&format!("SELECT to_jsonb(({sql}))"), &[])
        .await[0]
        .as_i64()
        .unwrap()
}

#[tokio::test]
async fn exact_retries_return_the_stored_outcome_even_after_expiry_or_revocation() {
    with_isolated_postgres(|db| async move {
        let sam = account(&db.store, "sam@example.org").await;
        let human = key(1);
        let first = human_observation("first", &human, BRAIN);
        assert_eq!(
            observe(&db.store, &sam, &first).await.unwrap(),
            ObservationOutcome::Recorded
        );
        // A second, different qualifying action changes nothing durable.
        let second = human_observation("second", &human, BRAIN);
        assert_eq!(
            observe(&db.store, &sam, &second).await.unwrap(),
            ObservationOutcome::Unchanged
        );

        // Lost response, retried after the freshness window: stored outcome.
        let late = now() + time::Duration::minutes(10);
        let retried = observe_as(&db.store, &sam, OBSERVATION_ISSUER, &first, late)
            .await
            .unwrap();
        assert_eq!(retried, ObservationOutcome::Recorded);

        // After the account's scope is revoked, a retry must not reopen it.
        db.store
            .exec("UPDATE account_brain_sharing_scopes SET revoked_at = NOW()")
            .await;
        assert_eq!(
            observe(&db.store, &sam, &first).await.unwrap(),
            ObservationOutcome::Recorded
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM account_brain_sharing_scopes WHERE revoked_at IS NULL"
            )
            .await,
            0
        );
        assert_eq!(
            describe(&db.store, BRAIN, &[&human]).await[0].state,
            DescriptionState::NotShared
        );

        // Same operation id with another payload or issuer is refused.
        let mut reused = first.clone();
        reused.brain_id = "brain_other".to_string();
        assert!(matches!(
            observe(&db.store, &sam, &reused).await,
            Err(BrainObservationError::OperationReused)
        ));
        assert!(matches!(
            observe_as(&db.store, &sam, "other-issuer", &first, now()).await,
            Err(BrainObservationError::OperationReused)
        ));
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM account_brain_principals").await,
            1
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM brain_account_observation_receipts"
            )
            .await,
            2
        );
    })
    .await;
}

#[tokio::test]
async fn new_operations_outside_the_window_write_nothing() {
    with_isolated_postgres(|db| async move {
        let sam = account(&db.store, "sam@example.org").await;
        for offset in [-91, 31] {
            let mut stale = human_observation(&format!("stale{offset}"), &key(1), BRAIN);
            stale.observed_at = stamp(now() + time::Duration::seconds(offset));
            assert!(matches!(
                observe(&db.store, &sam, &stale).await,
                Err(BrainObservationError::Stale)
            ));
        }
        for table in [
            "account_brain_principals",
            "account_brain_sharing_scopes",
            "brain_account_observation_receipts",
        ] {
            assert_eq!(
                count(&db, &format!("SELECT COUNT(*) FROM {table}")).await,
                0
            );
        }
    })
    .await;
}

#[tokio::test]
async fn one_key_never_moves_between_accounts_even_under_concurrency() {
    with_isolated_postgres(|db| async move {
        let sam = account(&db.store, "sam@example.org").await;
        let lee = account(&db.store, "lee@example.net").await;
        let shared = key(7);
        let sam_request = human_observation("sam", &shared, BRAIN);
        let lee_request = human_observation("lee", &shared, BRAIN);
        let (sam_result, lee_result) = tokio::join!(
            observe(&db.store, &sam, &sam_request),
            observe(&db.store, &lee, &lee_request)
        );
        let outcomes = [&sam_result, &lee_result];
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            outcomes
                .iter()
                .filter(|result| matches!(
                    result,
                    Err(BrainObservationError::KeyAssociatedElsewhere)
                ))
                .count(),
            1
        );
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM account_brain_principals").await,
            1
        );
        // The refused operation committed neither a scope nor a receipt.
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM account_brain_sharing_scopes").await,
            1
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM brain_account_observation_receipts"
            )
            .await,
            1
        );
    })
    .await;
}

#[tokio::test]
async fn owned_agent_actions_require_the_current_owner_and_a_live_agent() {
    with_isolated_postgres(|db| async move {
        let sam = account(&db.store, "sam@example.org").await;
        let lee = account(&db.store, "lee@example.net").await;
        project(&db, "project_ada", &sam, "Ada").await;
        runtime(&db, "runtime_ada", "project_ada", &key(2), true, None).await;
        project(&db, "project_old", &sam, "Old").await;
        runtime(
            &db,
            "runtime_old",
            "project_old",
            &key(3),
            false,
            Some("archived"),
        )
        .await;

        assert!(matches!(
            observe(&db.store, &lee, &agent_observation("lee", &key(2))).await,
            Err(BrainObservationError::AgentNotOwned)
        ));
        assert!(matches!(
            observe(&db.store, &sam, &agent_observation("old", &key(3))).await,
            Err(BrainObservationError::AgentNotOwned)
        ));
        assert!(matches!(
            observe(&db.store, &sam, &agent_observation("none", &key(9))).await,
            Err(BrainObservationError::AgentNotOwned)
        ));
        assert_eq!(
            observe(&db.store, &sam, &agent_observation("sam", &key(2)))
                .await
                .unwrap(),
            ObservationOutcome::Recorded
        );
        // An agent key is never recorded as a human key.
        assert!(matches!(
            observe(&db.store, &sam, &human_observation("human", &key(2), BRAIN)).await,
            Err(BrainObservationError::KeyIsAgentRecord)
        ));
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM account_brain_principals").await,
            0
        );
    })
    .await;
}

#[tokio::test]
async fn descriptions_disclose_only_to_the_shared_brain_and_hide_account_existence() {
    with_isolated_postgres(|db| async move {
        let sam = account(&db.store, "sam@example.org").await;
        let lee = account(&db.store, "lee@example.net").await;
        project(&db, "project_ada", &sam, "Ada").await;
        runtime(&db, "runtime_ada", "project_ada", &key(2), true, None).await;
        project(&db, "project_lee", &lee, "Lee Agent").await;
        runtime(&db, "runtime_lee", "project_lee", &key(4), true, None).await;
        observe(&db.store, &sam, &human_observation("sam", &key(1), BRAIN))
            .await
            .unwrap();
        // Lee has a human key but shared only with another Brain.
        observe(
            &db.store,
            &lee,
            &human_observation("lee", &key(5), "brain_other"),
        )
        .await
        .unwrap();

        let [human, ada, lee_agent, lee_human, unknown] = <[IdentityDescription; 5]>::try_from(
            describe(
                &db.store,
                BRAIN,
                &[&key(1), &key(2), &key(4), &key(5), &key(9)],
            )
            .await,
        )
        .unwrap();
        assert_eq!(human.state, DescriptionState::Resolved);
        assert_eq!(human.kind, Some(DescribedKind::Human));
        assert_eq!(human.account_email.as_deref(), Some("sam@example.org"));
        assert_eq!(human.display_name, None);

        assert_eq!(ada.state, DescriptionState::Resolved);
        assert_eq!(ada.kind, Some(DescribedKind::Agent));
        assert_eq!(ada.display_name.as_deref(), Some("Ada"));
        // A reserved Core agent name is not published NIP-05 evidence.
        assert!(!serde_json::to_string(&ada).unwrap().contains("finite.vip"));
        assert_eq!(ada.account_email, None);
        let responsible = ada.responsible_account.as_ref().unwrap();
        assert_eq!(responsible.email, "sam@example.org");
        assert_eq!(responsible.human_public_keys_hex, vec![key(1)]);
        assert_eq!(ada.source.as_ref().unwrap().kind, "finiteRuntimeRecord");
        assert!(
            !ada.source
                .as_ref()
                .unwrap()
                .revision
                .contains("runtime_ada")
        );

        // Known-but-unshared rows are byte-identical to a key Core never saw.
        let shape = |row: &IdentityDescription, key_hex: &str| {
            serde_json::to_string(row).unwrap().replace(key_hex, "KEY")
        };
        let unknown_shape = shape(&unknown, &key(9));
        assert_eq!(unknown.state, DescriptionState::NotShared);
        assert_eq!(shape(&lee_agent, &key(4)), unknown_shape);
        assert_eq!(shape(&lee_human, &key(5)), unknown_shape);

        // Another Brain sees nothing of Sam.
        for row in describe(&db.store, "brain_other", &[&key(1), &key(2)]).await {
            assert_eq!(row.state, DescriptionState::NotShared);
        }
        // Results follow request order exactly.
        let order = describe(&db.store, BRAIN, &[&key(9), &key(1)]).await;
        assert_eq!(order[0].public_key_hex, key(9));
        assert_eq!(order[1].public_key_hex, key(1));
    })
    .await;
}

#[tokio::test]
async fn lifecycle_ambiguity_and_unsupported_records_fail_closed() {
    with_isolated_postgres(|db| async move {
        let sam = account(&db.store, "sam@example.org").await;
        let lee = account(&db.store, "lee@example.net").await;
        for (user, operation, human) in [(&sam, "sam", key(1)), (&lee, "lee", key(5))] {
            observe(&db.store, user, &human_observation(operation, &human, BRAIN))
                .await
                .unwrap();
        }
        // Retired: every incarnation finished retirement.
        project(&db, "project_retired", &sam, "Retired").await;
        runtime(&db, "runtime_ret", "project_retired", &key(11), false, Some("archived")).await;
        // Offboarding but still linked.
        project(&db, "project_leaving", &sam, "Leaving").await;
        runtime(&db, "runtime_leaving", "project_leaving", &key(12), true, Some("compute_removed"))
            .await;
        // Unsupported: an archived row plus an inactive, never-offboarded
        // sibling (legacy re-registration). Neither row is picked.
        project(&db, "project_limbo", &sam, "Limbo").await;
        runtime(&db, "runtime_limbo_old", "project_limbo", &key(13), false, Some("archived")).await;
        runtime(&db, "runtime_limbo", "project_limbo", &key(13), false, None).await;
        // One pin on two projects with different owners.
        project(&db, "project_shared_a", &sam, "Shared").await;
        project(&db, "project_shared_b", &lee, "Shared").await;
        runtime(&db, "runtime_sa", "project_shared_a", &key(14), true, None).await;
        runtime(&db, "runtime_sb", "project_shared_b", &key(14), true, None).await;
        // A pin whose runtime is actively linked to a foreign project.
        project(&db, "project_foreign", &sam, "Foreign").await;
        runtime(&db, "runtime_foreign", "project_foreign", &key(15), false, None).await;
        project(&db, "project_elsewhere", &sam, "Elsewhere").await;
        db.store
            .exec(
                "INSERT INTO project_runtime_links (id, project_id, agent_runtime_id, active, created_at)
                 VALUES ('link_foreign', 'project_elsewhere', 'runtime_foreign', TRUE, NOW())",
            )
            .await;
        // Human key that is also pinned as an agent (staged directly).
        project(&db, "project_dual", &sam, "Dual").await;
        runtime(&db, "runtime_dual", "project_dual", &key(1), true, None).await;
        // Malformed stored pins never break a valid batch.
        project(&db, "project_junk", &sam, "Junk").await;
        pinned_runtime(&db, "runtime_junk1", "project_junk", "npub1qqqqqqqq", None).await;
        pinned_runtime(&db, "runtime_junk2", "project_junk", "NPUB1XYZ", None).await;

        let rows = describe(
            &db.store,
            BRAIN,
            &[&key(11), &key(12), &key(13), &key(14), &key(15), &key(1)],
        )
        .await;
        let lifecycle = |index: usize| rows[index].lifecycle.map(|value| format!("{value:?}"));
        assert_eq!(lifecycle(0).as_deref(), Some("Retired"));
        assert_eq!(
            rows[0].responsible_account.as_ref().unwrap().email,
            "sam@example.org"
        );
        assert_eq!(lifecycle(1).as_deref(), Some("Offboarding"));
        assert_eq!(rows[2].state, DescriptionState::Unknown, "retired row with an unoffboarded sibling");
        assert_eq!(rows[2].responsible_account, None);
        assert_eq!(rows[3].state, DescriptionState::Ambiguous);
        assert_eq!(rows[3].responsible_account, None);
        assert_eq!(rows[4].state, DescriptionState::NotShared, "foreign active link");
        assert_eq!(rows[5].state, DescriptionState::Ambiguous, "human and agent");
        // The conflicting key is never offered as Sam's human-key hint.
        for row in &rows[..2] {
            let hints = &row.responsible_account.as_ref().unwrap().human_public_keys_hex;
            assert!(!hints.contains(&key(1)), "{hints:?}");
        }

        // Ambiguity is only stated to an audience every owner shared with.
        db.store
            .exec(&format!(
                "UPDATE account_brain_sharing_scopes SET revoked_at = NOW() WHERE user_id = '{lee}'"
            ))
            .await;
        assert_eq!(
            describe(&db.store, BRAIN, &[&key(14)]).await[0].state,
            DescriptionState::NotShared
        );
    })
    .await;
}

#[tokio::test]
async fn same_project_legacy_rows_resolve_as_the_actively_linked_agent() {
    // Current relocation keeps one runtime id; several rows per project
    // exist only in legacy state. They are one agent when one is linked.
    with_isolated_postgres(|db| async move {
        let sam = account(&db.store, "sam@example.org").await;
        observe(&db.store, &sam, &human_observation("sam", &key(1), BRAIN))
            .await
            .unwrap();
        project(&db, "project_moved", &sam, "Moved").await;
        runtime(
            &db,
            "runtime_old",
            "project_moved",
            &key(10),
            false,
            Some("archived"),
        )
        .await;
        runtime(&db, "runtime_new", "project_moved", &key(10), true, None).await;
        let row = &describe(&db.store, BRAIN, &[&key(10)]).await[0];
        assert_eq!(row.state, DescriptionState::Resolved);
        assert_eq!(
            row.lifecycle.map(|value| format!("{value:?}")).as_deref(),
            Some("Active")
        );
    })
    .await;
}

#[tokio::test]
async fn owner_transfer_and_email_change_read_the_current_account() {
    with_isolated_postgres(|db| async move {
        let sam = account(&db.store, "sam@example.org").await;
        let lee = account(&db.store, "lee@example.net").await;
        observe(&db.store, &sam, &human_observation("sam", &key(1), BRAIN))
            .await
            .unwrap();
        project(&db, "project_ada", &sam, "Ada").await;
        runtime(&db, "runtime_ada", "project_ada", &key(2), true, None).await;
        assert_eq!(
            describe(&db.store, BRAIN, &[&key(2)]).await[0].state,
            DescriptionState::Resolved
        );

        // Core has no transfer writer; stage the owner change directly. The
        // former owner's scope must not release the successor's contact.
        db.store
            .exec(&format!(
                "UPDATE projects SET owner_user_id = '{lee}' WHERE id = 'project_ada'"
            ))
            .await;
        assert_eq!(
            describe(&db.store, BRAIN, &[&key(2)]).await[0].state,
            DescriptionState::NotShared
        );
        observe(&db.store, &lee, &human_observation("lee", &key(5), BRAIN))
            .await
            .unwrap();
        let row = &describe(&db.store, BRAIN, &[&key(2)]).await[0];
        assert_eq!(
            row.responsible_account.as_ref().unwrap().email,
            "lee@example.net"
        );

        // An email change keeps the same account and association.
        db.store
            .exec(&format!(
                "UPDATE users SET normalized_email = 'sam@new-domain.example' WHERE id = '{sam}'"
            ))
            .await;
        let human = &describe(&db.store, BRAIN, &[&key(1)]).await[0];
        assert_eq!(
            human.account_email.as_deref(),
            Some("sam@new-domain.example")
        );
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM account_brain_principals").await,
            2
        );
    })
    .await;
}

#[tokio::test]
async fn responsible_human_keys_are_bounded_and_migrations_reapply() {
    with_isolated_postgres(|db| async move {
        let sam = account(&db.store, "sam@example.org").await;
        observe(&db.store, &sam, &human_observation("sam", &key(1), BRAIN))
            .await
            .unwrap();
        for n in 20..32_u8 {
            let hex = key(n);
            db.store
                .exec(&format!(
                    "INSERT INTO account_brain_principals (id, user_id, public_key_hex, source, issuer,
                       status, first_observed_at, last_observed_at, revision)
                     VALUES ('extra_{n}', '{sam}', '{hex}', 'hosted_device_observation',
                       'finite-dashboard', 'active', NOW(), NOW(), 1)"
                ))
                .await;
        }
        project(&db, "project_ada", &sam, "Ada").await;
        runtime(&db, "runtime_ada", "project_ada", &key(2), true, None).await;
        let keys = describe(&db.store, BRAIN, &[&key(2)]).await[0]
            .responsible_account
            .clone()
            .unwrap()
            .human_public_keys_hex;
        assert_eq!(keys.len(), crate::brain_identity::MAX_RESPONSIBLE_HUMAN_KEYS);
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);

        let before = db.all("account_brain_principals").await;
        db.store.migrate().await.unwrap();
        db.store.migrate().await.unwrap();
        assert_eq!(db.all("account_brain_principals").await, before);
    })
    .await;
}

#[tokio::test]
async fn observations_never_enroll_or_link_an_account() {
    with_isolated_postgres(|db| async move {
        let users = db.all("users").await;
        let orgs = db.all("customer_orgs").await;
        let request = human_observation("unknown", &key(1), BRAIN);
        assert!(matches!(
            db.store
                .record_brain_account_observation("workos_never_seen", OBSERVATION_ISSUER, &request, now())
                .await,
            Err(BrainObservationError::AccountNotLinked)
        ));
        // A pending (invited, unlinked) account is refused and stays pending.
        db.store
            .exec(
                "INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at)
                 VALUES ('user_pending', 'pending@example.org', 'pending', NULL, NOW(), NOW())",
            )
            .await;
        let pending = db.all("users").await;
        assert!(matches!(
            db.store
                .record_brain_account_observation("workos_pending", OBSERVATION_ISSUER, &request, now())
                .await,
            Err(BrainObservationError::AccountNotLinked)
        ));
        assert_eq!(db.all("users").await, pending);
        assert_eq!(pending.len(), users.len() + 1);
        assert_eq!(db.all("customer_orgs").await, orgs);
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM brain_account_observation_receipts").await,
            0
        );
    })
    .await;
}

#[tokio::test]
async fn concurrent_duplicates_of_one_operation_share_one_outcome() {
    with_isolated_postgres(|db| async move {
        let sam = account(&db.store, "sam@example.org").await;
        let request = human_observation("same", &key(1), BRAIN);
        let (first, second) = tokio::join!(
            observe(&db.store, &sam, &request),
            observe(&db.store, &sam, &request)
        );
        assert_eq!(first.unwrap(), ObservationOutcome::Recorded);
        assert_eq!(second.unwrap(), ObservationOutcome::Recorded);
        for table in [
            "account_brain_principals",
            "account_brain_sharing_scopes",
            "brain_account_observation_receipts",
        ] {
            assert_eq!(
                count(&db, &format!("SELECT COUNT(*) FROM {table}")).await,
                1
            );
        }
    })
    .await;
}

#[tokio::test]
async fn oversized_or_control_character_contact_fields_are_never_sent() {
    with_isolated_postgres(|db| async move {
        let sam = account(&db.store, "sam@example.org").await;
        observe(&db.store, &sam, &human_observation("sam", &key(1), BRAIN))
            .await
            .unwrap();
        project(&db, "project_long", &sam, &"N".repeat(201)).await;
        runtime(&db, "runtime_long", "project_long", &key(2), true, None).await;
        let rows = describe(&db.store, BRAIN, &[&key(2)]).await;
        assert_eq!(rows[0].state, DescriptionState::Unknown);
        assert_eq!(rows[0].display_name, None);
        db.store
            .exec(&format!(
                "UPDATE users SET normalized_email = E'sam\\x07@example.org' WHERE id = '{sam}'"
            ))
            .await;
        let human = &describe(&db.store, BRAIN, &[&key(1)]).await[0];
        assert_eq!(human.state, DescriptionState::Unknown);
        assert_eq!(human.account_email, None);
        // Oversized stored email: withheld by the SQL read, never truncated.
        let long = format!("{}@example.org", "s".repeat(300));
        db.store
            .exec(&format!(
                "UPDATE users SET normalized_email = '{long}' WHERE id = '{sam}'"
            ))
            .await;
        let human = &describe(&db.store, BRAIN, &[&key(1)]).await[0];
        assert_eq!(human.state, DescriptionState::Unknown);
        assert_eq!(human.account_email, None);
    })
    .await;
}
