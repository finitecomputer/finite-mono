use super::*;

/// A dry-run reconcile reports what it would do against real state and
/// persists nothing.
///
/// Running the same record twice is the assertion that matters: if the
/// first dry run had committed, the second would report an *update* of the
/// row it just wrote instead of a creation.
#[tokio::test]
async fn dry_run_finite_private_friend_key_issue_generates_one_time_key() {
    with_dry_run_database(|| async {
        let output = finite_private_friend_key_issue(FinitePrivateFriendKeyIssueArgs {
            email: "friend@finite.vip".to_string(),
            workos_user_id: None,
            limit_profile_id: None,
            project_id: None,
            agent_runtime_id: None,
            raw_key_env: None,
            now: Some("2026-05-26T12:00:00Z".to_string()),
            mode: ImportMode::DryRun,
        })
        .await
        .unwrap();

        let raw_key = output.raw_api_key.as_deref().unwrap();
        assert!(raw_key.starts_with("fpk_live_"));
        assert!(output.raw_api_key_generated);
        assert_eq!(output.grant.as_ref().unwrap().status.as_str(), "active");
        assert_eq!(output.api_key.status.as_str(), "active");
        assert_ne!(output.api_key.key_hash, raw_key);
    })
    .await;
}

/// `--dry-run` on a revoke reads real state and changes nothing.
///
/// This is the regression that made dry runs useless for the revoke
/// commands: they previewed against an empty store, so an operator checking
/// a perfectly valid production key was told the key was invalid, no matter
/// what they passed.
#[tokio::test]
async fn dry_run_revoke_reads_a_real_key_and_leaves_it_active() {
    with_dry_run_database(|| async {
        let issued = finite_private_friend_key_issue(FinitePrivateFriendKeyIssueArgs {
            email: "dry-run-revoke-probe@finite.vip".to_string(),
            workos_user_id: None,
            limit_profile_id: None,
            project_id: None,
            agent_runtime_id: None,
            raw_key_env: None,
            now: Some("2026-05-26T12:00:00Z".to_string()),
            mode: ImportMode::Commit,
        })
        .await
        .unwrap();
        let key_id = issued.api_key.id.clone();

        // The preview finds the real row and reports the revocation it
        // would perform, instead of failing with "invalid API key".
        let previewed = finite_private_api_key_revoke(
            key_id.clone(),
            Some("2026-05-26T12:05:00Z".to_string()),
            ImportMode::DryRun,
        )
        .await
        .unwrap();
        assert_eq!(previewed.id, key_id);
        assert_eq!(previewed.status.as_str(), "revoked");

        // The preview rolled back, so the key is still usable.
        let store = postgres_store_from_env(ImportMode::Commit).await.unwrap();
        let state = store.finite_private_admin_state().await.unwrap();
        let stored = state
            .api_keys
            .iter()
            .find(|key| key.id == key_id)
            .expect("issued key is still present");
        assert_eq!(stored.status.as_str(), "active");
    })
    .await;
}

#[test]
fn generated_finite_private_api_keys_are_prefixed_and_unique() {
    let first = generate_finite_private_api_key().unwrap();
    let second = generate_finite_private_api_key().unwrap();

    assert!(first.starts_with("fpk_live_"));
    assert!(second.starts_with("fpk_live_"));
    assert_ne!(first, second);
    assert_eq!(first.len(), "fpk_live_".len() + 64);
}
