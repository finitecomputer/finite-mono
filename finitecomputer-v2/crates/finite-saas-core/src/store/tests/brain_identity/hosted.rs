use super::*;
use crate::brain_identity::hosted::{HOSTED_OBSERVATION_VERSION, HostedAccountObservationRequest};

fn request(operation: &str, human: &str) -> HostedAccountObservationRequest {
    HostedAccountObservationRequest {
        version: HOSTED_OBSERVATION_VERSION.into(),
        operation_id: format!("hosted_{operation}_00000000"),
        observed_at: stamp(now()),
        public_key_hex: human.into(),
    }
}

async fn register(
    store: &CoreStore,
    user: &str,
    request: &HostedAccountObservationRequest,
    at: time::OffsetDateTime,
) -> Result<ObservationOutcome, BrainObservationError> {
    let subject = store.user(user).await.unwrap().workos_user_id.unwrap();
    store
        .record_hosted_account_observation(&subject, OBSERVATION_ISSUER, request, at)
        .await
}

#[tokio::test]
async fn hosted_registration_resolves_existing_key_without_sharing_or_scoped_records() {
    with_isolated_postgres(|db| async move {
        let user = account(&db, "human@example.test").await;
        let human = key(71);
        let original = request("first", &human);
        let users = db.all("users").await;
        assert_eq!(
            register(&db, &user, &original, now()).await.unwrap(),
            ObservationOutcome::Recorded
        );
        let receipts = db.all("brain_account_observation_receipts").await;
        // An exact lost-response replay remains valid after the freshness window.
        assert_eq!(
            register(&db, &user, &original, now() + time::Duration::hours(2))
                .await
                .unwrap(),
            ObservationOutcome::Recorded
        );
        assert_eq!(db.all("brain_account_observation_receipts").await, receipts);
        assert_eq!(
            register(&db, &user, &request("refresh", &human), now())
                .await
                .unwrap(),
            ObservationOutcome::Unchanged
        );
        assert_eq!(db.all("account_brain_principals").await.len(), 1);
        assert!(db.all("account_brain_sharing_scopes").await.is_empty());
        assert!(db.all("brain_scoped_records").await.is_empty());
        assert_eq!(db.all("users").await, users);
        // Legacy v1 still withholds without a sharing scope. Eligible v2
        // requests can describe the same key using current account facts.
        assert_eq!(
            describe(&db, BRAIN, &[&human]).await[0].state,
            DescriptionState::NotShared
        );
        let named = describe_as(&db, DESCRIPTIONS_VERSION_V2, BRAIN, &[&human])
            .await
            .remove(0);
        assert_eq!(named.state, DescriptionState::Resolved);
        assert_eq!(named.kind, Some(DescribedKind::Human));
        assert_eq!(named.account_email.as_deref(), Some("human@example.test"));
    })
    .await;
}

#[tokio::test]
async fn hosted_registration_refuses_conflicts_stale_evidence_and_cross_protocol_replay() {
    with_isolated_postgres(|db| async move {
        let user = account(&db, "first@example.test").await;
        let other = account(&db, "second@example.test").await;
        let human = key(72);
        let original = request("same", &human);
        register(&db, &user, &original, now()).await.unwrap();
        let before = db.all("account_brain_principals").await;
        let receipts = db.all("brain_account_observation_receipts").await;
        assert!(matches!(
            register(&db, &other, &original, now()).await,
            Err(BrainObservationError::KeyAssociatedElsewhere)
        ));
        let mut changed = original.clone();
        changed.public_key_hex = key(73);
        assert!(matches!(
            register(&db, &user, &changed, now()).await,
            Err(BrainObservationError::OperationReused)
        ));
        let mut brain = human_observation("legacy", &human, BRAIN);
        brain.operation_id = original.operation_id.clone();
        assert!(matches!(
            observe(&db, &user, &brain).await,
            Err(BrainObservationError::OperationReused)
        ));
        for at in [
            now() + time::Duration::minutes(2),
            now() - time::Duration::minutes(2),
        ] {
            assert!(matches!(
                register(&db, &user, &request("stale", &key(73)), at).await,
                Err(BrainObservationError::Stale)
            ));
        }
        assert_eq!(db.all("account_brain_principals").await, before);
        assert_eq!(db.all("brain_account_observation_receipts").await, receipts);
        assert!(db.all("account_brain_sharing_scopes").await.is_empty());
        project(&db, "hosted-project", &user, "Agent").await;
        runtime(
            &db,
            "hosted-runtime",
            "hosted-project",
            &key(74),
            true,
            None,
        )
        .await;
        assert!(matches!(
            register(&db, &user, &request("agent", &key(74)), now()).await,
            Err(BrainObservationError::KeyIsAgentRecord)
        ));
        assert!(matches!(
            db.record_hosted_account_observation(
                "missing-subject",
                OBSERVATION_ISSUER,
                &original,
                now()
            )
            .await,
            Err(BrainObservationError::AccountNotLinked)
        ));
        // The existing Brain action can still establish its own scope later.
        observe(
            &db,
            &user,
            &human_observation("actual_action", &human, BRAIN),
        )
        .await
        .unwrap();
        assert_eq!(db.all("account_brain_sharing_scopes").await.len(), 1);
        assert_eq!(db.all("account_brain_principals").await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn concurrent_hosted_claims_keep_one_account_and_one_receipt() {
    with_isolated_postgres(|db| async move {
        let first = account(&db, "one@example.test").await;
        let second = account(&db, "two@example.test").await;
        let observation = request("race", &key(75));
        let (a, b) = tokio::join!(
            register(&db, &first, &observation, now()),
            register(&db, &second, &observation, now())
        );
        assert!(matches!(
            (&a, &b),
            (Ok(_), Err(BrainObservationError::KeyAssociatedElsewhere))
                | (Err(BrainObservationError::KeyAssociatedElsewhere), Ok(_))
        ));
        assert_eq!(db.all("account_brain_principals").await.len(), 1);
        assert_eq!(db.all("brain_account_observation_receipts").await.len(), 1);
    })
    .await;
}
