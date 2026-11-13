//! Synthetic exact-scope operations and disclosure against ephemeral PostgreSQL.
use super::*;
use crate::brain_identity::scoped::*;
mod account_names;
mod guards;

fn record(
    n: u8,
    kind: ScopedRecordKind,
    account: Option<&str>,
    project: Option<&str>,
) -> ScopedRecordInput {
    ScopedRecordInput {
        public_key_hex: key(n),
        kind,
        account_id: account.map(str::to_string),
        workos_subject: account.map(|_| "workos_sam_example_org".to_string()),
        project_id: project.map(str::to_string),
        annotation_label: matches!(
            kind,
            ScopedRecordKind::HistoricalIdentity | ScopedRecordKind::InvitationKey
        )
        .then(|| "Historical Agent: example".to_string()),
        evidence: ScopedEvidence {
            sha256: key(0xab),
            reference: "private-evidence/synthetic.json".to_string(),
            observed_at: "2026-01-01T00:00:00Z".to_string(),
        },
    }
}
fn manifest(operation: &str, records: Vec<ScopedRecordInput>) -> ScopedManifest {
    ScopedManifest {
        version: SCOPED_MANIFEST_VERSION.to_string(),
        operation_id: format!("operation_{operation}_0000"),
        action: ScopedAction::Apply,
        brain_server: SERVER.to_string(),
        brain_id: BRAIN.to_string(),
        operator: ScopedOperator {
            email: "operator@example.org".to_string(),
            workos_subject: "workos_operator_example_org".to_string(),
        },
        evidence_pack_sha256: key(0xcd),
        not_before: stamp(now() - time::Duration::minutes(1)),
        expires_at: stamp(now() + time::Duration::hours(1)),
        target_operation_id: None,
        records,
    }
}
fn retirement(source: &ScopedManifest, operation: &str) -> ScopedManifest {
    let mut input = manifest(operation, vec![]);
    input.action = ScopedAction::Retire;
    input.target_operation_id = Some(source.operation_id.clone());
    input
}
async fn setup(db: &TestDb) -> String {
    account(db, "operator@example.org").await;
    account(db, "sam@example.org").await
}
async fn run(db: &CoreStore, input: &ScopedManifest) -> CoreResult<ScopedReceipt> {
    db.execute_brain_scoped_manifest(input, SERVER, now()).await
}
async fn page(
    db: &CoreStore,
    brain: &str,
    keys: &[u8],
    participating: &[u8],
) -> Vec<ScopedIdentityDescription> {
    db.describe_scoped_brain_identities(&ScopedDescriptionsRequest {
        version: DESCRIPTIONS_VERSION_V3.to_string(),
        brain_server: SERVER.to_string(),
        brain_id: brain.to_string(),
        requested_by_public_key_hex: key(0xee),
        public_keys_hex: keys.iter().map(|n| key(*n)).collect(),
        participating_public_keys_hex: participating.iter().map(|n| key(*n)).collect(),
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn scoped_preview_apply_replay_repeat_migration_and_retire_preserve_other_state() {
    with_isolated_postgres(|db| async move {
        let sam = setup(&db).await;
        project(&db, "project-scoped", &sam, "Example Agent").await;
        runtime(&db, "runtime-scoped", "project-scoped", &key(2), true, None).await;
        let input = manifest(
            "all_kinds",
            vec![
                record(1, ScopedRecordKind::AccountChatKey, Some(&sam), None),
                record(
                    2,
                    ScopedRecordKind::ManagedAgent,
                    Some(&sam),
                    Some("project-scoped"),
                ),
                record(3, ScopedRecordKind::HistoricalIdentity, None, None),
                record(4, ScopedRecordKind::InvitationKey, None, None),
            ],
        );
        // Match current project creation: an older additive migration fills
        // this field when it encounters intentionally sparse legacy fixtures.
        db.exec("UPDATE projects SET hosting_tier='standard' WHERE id='project-scoped'")
            .await;
        let mut before = Vec::new();
        let tables = [
            "users",
            "projects",
            "agent_runtimes",
            "project_runtime_links",
            "account_brain_principals",
            "account_brain_sharing_scopes",
            "brain_account_observation_receipts",
        ];
        for table in tables {
            before.push(db.all(table).await);
        }
        let dry = CoreStore::connect_dry_run(&db.url).await.unwrap();
        let preview = run(&dry, &input).await.unwrap();
        assert_eq!(preview.records.len(), 4);
        assert!(db.all("brain_scoped_records").await.is_empty());
        assert!(db.all("brain_scoped_record_operations").await.is_empty());
        let receipt = run(&db, &input).await.unwrap();
        assert_eq!(receipt.manifest_sha256, preview.manifest_sha256);
        assert_eq!(run(&db, &input).await.unwrap(), receipt);
        assert_eq!(
            db.execute_brain_scoped_manifest(&input, SERVER, now() + time::Duration::days(2))
                .await
                .unwrap(),
            receipt
        );
        let rows = page(&db, BRAIN, &[1, 2, 3, 4], &[]).await;
        assert_eq!(rows[0].account_email.as_deref(), Some("sam@example.org"));
        assert_eq!(rows[1].display_name.as_deref(), Some("Example Agent"));
        assert!(
            rows[1]
                .responsible_account
                .as_ref()
                .unwrap()
                .human_public_keys_hex
                .is_empty()
        );
        assert_eq!(
            rows[2].operator_annotation.as_ref().unwrap().kind,
            AnnotationKind::HistoricalIdentity
        );
        assert_eq!(
            rows[3].operator_annotation.as_ref().unwrap().kind,
            AnnotationKind::InvitationKey
        );
        assert!(
            rows.iter()
                .all(|r| r.operator_record.as_ref().unwrap().state == OperatorRecordState::Used)
        );
        // Additive schema startup is repeat-safe and legacy readers ignore it.
        db.migrate().await.unwrap();
        assert_eq!(run(&db, &input).await.unwrap(), receipt);
        let legacy = describe_as(&db, DESCRIPTIONS_VERSION_V2, BRAIN, &[&key(1), &key(3)]).await;
        assert!(
            legacy
                .iter()
                .all(|r| r.state == DescriptionState::NotShared)
        );
        let undo = retirement(&input, "undo_all");
        run(&dry, &undo).await.unwrap();
        assert_eq!(
            page(&db, BRAIN, &[1], &[]).await[0].state,
            DescriptionState::Resolved
        );
        let undone = run(&db, &undo).await.unwrap();
        assert_eq!(undone.records.len(), 4);
        assert_eq!(run(&db, &undo).await.unwrap(), undone);
        assert!(
            page(&db, BRAIN, &[1, 2, 3, 4], &[])
                .await
                .iter()
                .all(|r| r.state == DescriptionState::NotShared && r.operator_record.is_none())
        );
        assert_eq!(db.all("brain_scoped_records").await.len(), 4);
        for (index, table) in tables.iter().enumerate() {
            assert_eq!(db.all(table).await, before[index], "{table} changed");
        }
    })
    .await;
}

#[tokio::test]
async fn scoped_noops_are_not_owned_and_nonidentical_batches_are_atomic() {
    with_isolated_postgres(|db| async move {
        setup(&db).await;
        let first = manifest(
            "first",
            vec![record(2, ScopedRecordKind::HistoricalIdentity, None, None)],
        );
        run(&db, &first).await.unwrap();
        let same = manifest("second", first.records.clone());
        let receipt = run(&db, &same).await.unwrap();
        assert_eq!(receipt.records[0].disposition, ScopedDisposition::Existing);
        assert!(
            run(&db, &retirement(&same, "undo_second"))
                .await
                .unwrap()
                .records
                .is_empty()
        );
        assert!(
            page(&db, BRAIN, &[2], &[]).await[0]
                .operator_annotation
                .is_some()
        );
        let mut conflict = manifest(
            "conflict",
            vec![
                record(1, ScopedRecordKind::HistoricalIdentity, None, None),
                first.records[0].clone(),
            ],
        );
        conflict.records[1].annotation_label = Some("Different historical attribution".to_string());
        assert!(run(&db, &conflict).await.is_err());
        assert_eq!(db.all("brain_scoped_records").await.len(), 1);
        assert!(
            page(&db, BRAIN, &[1], &[]).await[0]
                .operator_record
                .is_none()
        );
        let mut reused = first.clone();
        reused.records[0].evidence.reference = "different-evidence.json".to_string();
        assert!(run(&db, &reused).await.is_err());
    })
    .await;
}

#[tokio::test]
async fn scoped_retirement_afterimage_drift_rolls_back_every_row() {
    with_isolated_postgres(|db| async move {
        setup(&db).await;
        let input = manifest("two", vec![record(1, ScopedRecordKind::HistoricalIdentity, None, None), record(2, ScopedRecordKind::InvitationKey, None, None)]);
        run(&db, &input).await.unwrap();
        db.exec(&format!("UPDATE brain_scoped_records SET annotation_label='Changed attribution' WHERE public_key_hex='{}'", key(2))).await;
        let before = db.all("brain_scoped_records").await;
        let operations = db.all("brain_scoped_record_operations").await;
        assert!(run(&db, &retirement(&input, "undo_drift")).await.is_err());
        assert_eq!(db.all("brain_scoped_records").await, before);
        assert_eq!(db.all("brain_scoped_record_operations").await, operations);
    }).await;
}

#[tokio::test]
async fn scoped_scope_operator_freshness_and_global_conflicts_fail_closed() {
    with_isolated_postgres(|db| async move {
        let sam = setup(&db).await;
        let input = manifest(
            "account",
            vec![record(
                1,
                ScopedRecordKind::AccountChatKey,
                Some(&sam),
                None,
            )],
        );
        assert!(
            db.execute_brain_scoped_manifest(&input, "https://other.test", now())
                .await
                .is_err()
        );
        let mut bad = input.clone();
        bad.operator.workos_subject = "workos_wrong".to_string();
        assert!(run(&db, &bad).await.is_err());
        assert!(
            db.execute_brain_scoped_manifest(&input, SERVER, now() + time::Duration::days(1))
                .await
                .is_err()
        );
        run(&db, &input).await.unwrap();
        for participating in [vec![], vec![1]] {
            let other = page(&db, "other_brain", &[1], &participating).await;
            assert_eq!(other[0].state, DescriptionState::NotShared);
            assert!(other[0].operator_record.is_none());
        }
        observe(&db, &sam, &human_observation("unscoped", &key(5), BRAIN))
            .await
            .unwrap();
        let rows = page(&db, BRAIN, &[1, 5], &[]).await;
        assert_eq!(rows[0].state, DescriptionState::Resolved);
        assert_eq!(rows[1].state, DescriptionState::NotShared);
        let other = account(&db, "other@example.org").await;
        observe(
            &db,
            &other,
            &human_observation("contradiction", &key(1), BRAIN),
        )
        .await
        .unwrap();
        db.exec(&format!(
            "UPDATE users SET link_status='pending', workos_user_id=NULL WHERE id='{other}'"
        ))
        .await;
        let row = &page(&db, BRAIN, &[1], &[]).await[0];
        assert_eq!(row.state, DescriptionState::NotShared);
        assert_eq!(
            row.operator_record.as_ref().unwrap().reason,
            Some(OperatorRecordReason::IdentityEvidenceConflict)
        );
        let new_op = manifest("conflicting_account", input.records.clone());
        assert!(run(&db, &new_op).await.is_err());
    })
    .await;
}

#[tokio::test]
async fn scoped_agent_hints_use_only_participation_and_never_operator_links() {
    with_isolated_postgres(|db| async move {
        let sam = setup(&db).await;
        project(&db, "hint-project", &sam, "Hint Agent").await;
        runtime(&db, "hint-runtime", "hint-project", &key(1), true, None).await;
        for n in [2, 3] {
            observe(
                &db,
                &sam,
                &human_observation(&format!("hint{n}"), &key(n), BRAIN),
            )
            .await
            .unwrap();
        }
        let input = manifest(
            "hint_agent",
            vec![record(
                1,
                ScopedRecordKind::ManagedAgent,
                Some(&sam),
                Some("hint-project"),
            )],
        );
        run(&db, &input).await.unwrap();
        let operator = page(&db, BRAIN, &[1, 2, 3], &[2]).await;
        assert!(
            operator[0]
                .responsible_account
                .as_ref()
                .unwrap()
                .human_public_keys_hex
                .is_empty()
        );
        assert_eq!(operator[2].state, DescriptionState::NotShared);
        let normal = page(&db, BRAIN, &[1, 2, 3], &[1, 2]).await;
        assert_eq!(
            normal[0]
                .responsible_account
                .as_ref()
                .unwrap()
                .human_public_keys_hex,
            vec![key(2)]
        );
        assert_eq!(
            normal[0].source.as_ref().unwrap().kind,
            "finiteRuntimeRecord"
        );
        assert_eq!(
            normal[0].operator_record.as_ref().unwrap().reason,
            Some(OperatorRecordReason::NormalResultPrecedence)
        );
    })
    .await;
}

#[tokio::test]
async fn scoped_concurrent_exact_retries_and_same_key_races_do_not_partially_commit() {
    with_isolated_postgres(|db| async move {
        setup(&db).await;
        let input = manifest(
            "concurrent",
            vec![record(1, ScopedRecordKind::HistoricalIdentity, None, None)],
        );
        let (a, b) = tokio::join!(run(&db, &input), run(&db, &input));
        assert_eq!(a.unwrap(), b.unwrap());
        let left = manifest(
            "left",
            vec![record(2, ScopedRecordKind::InvitationKey, None, None)],
        );
        let right = manifest("right", left.records.clone());
        let (a, b) = tokio::join!(run(&db, &left), run(&db, &right));
        assert!(a.is_ok() || b.is_ok());
        assert_eq!(db.all("brain_scoped_records").await.len(), 2);
        assert_eq!(page(&db, BRAIN, &[1, 2], &[]).await.len(), 2);
    })
    .await;
}

#[tokio::test]
async fn scoped_missing_contact_subject_change_and_new_normal_observation_are_distinct() {
    with_isolated_postgres(|db| async move {
        let sam = setup(&db).await;
        let input = manifest("contact", vec![record(1, ScopedRecordKind::AccountChatKey, Some(&sam), None)]);
        run(&db, &input).await.unwrap();
        db.exec(&format!("UPDATE users SET normalized_email='new@example.org' WHERE id='{sam}'")).await;
        assert_eq!(page(&db, BRAIN, &[1], &[]).await[0].account_email.as_deref(), Some("new@example.org"));
        db.exec(&format!("UPDATE users SET normalized_email=repeat('x',255) WHERE id='{sam}'")).await;
        let row = &page(&db, BRAIN, &[1], &[]).await[0];
        assert_eq!(row.state, DescriptionState::Unknown);
        assert_eq!(row.operator_record.as_ref().unwrap().reason, Some(OperatorRecordReason::ContactUnavailable));
        assert!(row.account_email.is_none());
        db.exec(&format!("UPDATE users SET normalized_email='new@example.org',workos_user_id='replacement_subject' WHERE id='{sam}'")).await;
        let row = &page(&db, BRAIN, &[1], &[]).await[0];
        assert_eq!(row.state, DescriptionState::NotShared);
        assert_eq!(row.operator_record.as_ref().unwrap().reason, Some(OperatorRecordReason::AccountBindingChanged));
        db.exec(&format!("UPDATE users SET workos_user_id='workos_sam_example_org' WHERE id='{sam}'")).await;
        observe(&db, &sam, &human_observation("real_action", &key(1), BRAIN)).await.unwrap();
        let row = &page(&db, BRAIN, &[1], &[1]).await[0];
        assert_eq!(row.source.as_ref().unwrap().kind, "hostedDeviceObservation");
        assert_eq!(row.operator_record.as_ref().unwrap().state, OperatorRecordState::Suppressed);
        run(&db, &retirement(&input, "undo_contact")).await.unwrap();
        assert_eq!(page(&db, BRAIN, &[1], &[1]).await[0].state, DescriptionState::Resolved);
    }).await;
}

#[tokio::test]
async fn scoped_annotations_never_cover_current_evidence_and_agent_bindings_follow_lifecycle() {
    with_isolated_postgres(|db| async move {
        let sam = setup(&db).await;
        let input = manifest("annotation", vec![record(1, ScopedRecordKind::HistoricalIdentity, None, None)]);
        run(&db, &input).await.unwrap();
        observe(&db, &sam, &human_observation("annotation_later", &key(1), BRAIN)).await.unwrap();
        for participation in [vec![], vec![1]] {
            let row = &page(&db, BRAIN, &[1], &participation).await[0];
            assert!(row.operator_annotation.is_none());
            assert_eq!(row.operator_record.as_ref().unwrap().reason, Some(OperatorRecordReason::AnnotationSuperseded));
            assert_eq!(row.state, if participation.is_empty() { DescriptionState::NotShared } else { DescriptionState::Resolved });
        }
        project(&db, "lifecycle-project", &sam, "Lifecycle Agent").await;
        runtime(&db, "lifecycle-runtime", "lifecycle-project", &key(2), true, None).await;
        let input = manifest("agent", vec![record(2, ScopedRecordKind::ManagedAgent, Some(&sam), Some("lifecycle-project"))]);
        run(&db, &input).await.unwrap();
        db.exec("UPDATE project_runtime_links SET active=false WHERE agent_runtime_id='lifecycle-runtime'; UPDATE agent_runtimes SET offboarding_phase='archived' WHERE id='lifecycle-runtime'").await;
        assert_eq!(page(&db, BRAIN, &[2], &[]).await[0].lifecycle, Some(crate::brain_identity::AgentLifecycle::Retired));
        let other = account(&db, "next@example.org").await;
        db.exec(&format!("UPDATE projects SET owner_user_id='{other}' WHERE id='lifecycle-project'")).await;
        let row = &page(&db, BRAIN, &[2], &[]).await[0];
        assert_eq!(row.state, DescriptionState::NotShared);
        assert_eq!(row.operator_record.as_ref().unwrap().reason, Some(OperatorRecordReason::AgentBindingChanged));
        // Normal participating evidence wins independently, but stale scoped
        // ownership remains observable and never supplies the contact.
        let row = &page(&db, BRAIN, &[2], &[2]).await[0];
        assert_eq!(row.responsible_account.as_ref().unwrap().email, "next@example.org");
        assert_eq!(row.operator_record.as_ref().unwrap().state, OperatorRecordState::Conflict);
    }).await;
}

#[tokio::test]
async fn scoped_backup_restores_receipts_and_reader_results_onto_empty_target() {
    with_isolated_postgres(|source| async move {
        let sam = setup(&source).await;
        let input = manifest(
            "backup",
            vec![
                record(1, ScopedRecordKind::AccountChatKey, Some(&sam), None),
                record(2, ScopedRecordKind::HistoricalIdentity, None, None),
            ],
        );
        run(&source, &input).await.unwrap();
        let before = page(&source, BRAIN, &[1, 2], &[]).await;
        let rows = source.all("brain_scoped_records").await;
        let receipts = source.all("brain_scoped_record_operations").await;
        let archive = tempfile::NamedTempFile::new().unwrap();
        let dump = std::process::Command::new("pg_dump")
            .args(["--format=custom", "--no-owner", "--no-acl", "--file"])
            .arg(archive.path())
            .arg(&source.url)
            .output()
            .unwrap();
        assert!(
            dump.status.success(),
            "{}",
            String::from_utf8_lossy(&dump.stderr)
        );
        with_isolated_postgres(|target| async move {
            // This target is a disposable per-test database, never a service.
            target
                .exec("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
                .await;
            let restore = std::process::Command::new("pg_restore")
                .args(["--exit-on-error", "--no-owner", "--no-acl", "--dbname"])
                .arg(&target.url)
                .arg(archive.path())
                .output()
                .unwrap();
            assert!(
                restore.status.success(),
                "{}",
                String::from_utf8_lossy(&restore.stderr)
            );
            assert_eq!(target.all("brain_scoped_records").await, rows);
            assert_eq!(target.all("brain_scoped_record_operations").await, receipts);
            assert_eq!(page(&target, BRAIN, &[1, 2], &[]).await, before);
            assert_eq!(
                run(&target, &input).await.unwrap().operation_id,
                input.operation_id
            );
            run(&target, &retirement(&input, "undo_restored"))
                .await
                .unwrap();
            assert!(
                page(&target, BRAIN, &[1, 2], &[])
                    .await
                    .iter()
                    .all(|r| r.operator_record.is_none())
            );
        })
        .await;
    })
    .await;
}
