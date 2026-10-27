//! Local browser composition: real dashboard routes, Core, signed WorkOS
//! fixture requests and disposable Postgres. The unrelated Agent/Brain roster
//! transport is a fixture; its names come from Core's actual description API.
use super::*;
use crate::auth::test_support::access_token_with_profile;
use crate::store::runtime_credentials::tests::{complete, register, requested};
use tokio::io::AsyncWriteExt;

#[tokio::test]
#[ignore = "requires built dashboard, locked pnpm dependencies and Chromium"]
async fn key_alias_browser_composition() {
    with_isolated_postgres(|db| async move {
        let creation = requested(&db).await;
        let runtime = register(&db, &creation).await;
        complete(&db, &creation).await.unwrap();
        let owner = access_token_with_profile(
            "runtime-auth-user",
            "runtime-auth@finite.test",
            Some(serde_json::json!("Ada")),
            Some(serde_json::json!("Lovelace")),
        );
        let other =
            access_token_with_subject("alias-browser-other", "other@finite.test", true, None);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let core_url = format!("http://{}", listener.local_addr().unwrap());
        let app = router(db.store.clone(), test_auth());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let identity_url = format!("http://{}", listener.local_addr().unwrap());
        let app = brain_identity::identity_app(db.store.clone());
        let identity_server =
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let keys: Vec<_> = [21, 22, 23, 24, 25, 26]
            .into_iter()
            .map(key_aliases::valid_key)
            .collect();
        let npubs: Vec<_> = keys
            .iter()
            .map(|key| crate::brain_identity::npub_for_hex(key).unwrap())
            .collect();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut child = tokio::process::Command::new("node")
            .args([
                "--import",
                "tsx",
                "scripts/brain-key-alias-browser-proof.ts",
            ])
            .current_dir(root.join("finitecomputer-v2/apps/dashboard"))
            .stdin(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let config = serde_json::json!({
            "coreUrl":core_url, "identityUrl":identity_url, "runtimeId":runtime,
            "owner":owner, "other":other, "keys":keys, "npubs":npubs,
            "observationToken":brain_identity::OBSERVATION_TOKEN,
            "descriptionToken":brain_identity::DESCRIPTION_TOKEN,
        });
        child
            .stdin
            .take()
            .unwrap()
            .write_all(config.to_string().as_bytes())
            .await
            .unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(180), child.wait()).await;
        server.abort();
        identity_server.abort();
        assert!(
            result
                .expect("browser proof exceeded deadline")
                .unwrap()
                .success()
        );
    })
    .await;
}
