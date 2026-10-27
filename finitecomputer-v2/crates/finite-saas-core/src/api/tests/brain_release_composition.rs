//! Opt-in local composition of Brain deletion, people management and
//! account names/private aliases. Real here: Core's router and private
//! identity listener over disposable Postgres. The dashboard script starts
//! the real Brain server, Hosted Device, dashboard, Hermes and Caddy. Fixtures:
//! the WorkOS provider and the Agent's Brain-key pin on its runtime row.
use super::*;
use crate::auth::test_support::access_token_with_profile;
use crate::brain_identity::{BrainIdentityConfig, npub_for_hex};
use crate::store::runtime_credentials::tests::{complete, register, requested};
use tokio::io::AsyncWriteExt;

/// Synthetic Agent secret: 32 bytes of this value.
const AGENT_SECRET_BYTE: u8 = 0x4d;

async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    (
        url,
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }),
    )
}

#[tokio::test]
#[ignore = "requires a built dashboard, Chromium and the FC_BRAIN_COMPOSITION_* binaries"]
async fn brain_release_composition() {
    with_isolated_postgres(|db| async move {
        let creation = requested(&db).await;
        let runtime = register(&db, &creation).await;
        complete(&db, &creation).await.unwrap();
        // Production pins this from the runtime's authenticated health report.
        let agent = npub_for_hex(&key_aliases::valid_key(AGENT_SECRET_BYTE)).unwrap();
        db.store
            .exec(&format!(
                "UPDATE agent_runtimes SET health_reporting_npub='{agent}' WHERE id='{runtime}'"
            ))
            .await;
        // Two accounts whose WorkOS records carry the same name.
        let ada = |subject: &str, email: &str| {
            let (first, last) = (serde_json::json!("Ada"), serde_json::json!("Lovelace"));
            access_token_with_profile(subject, email, Some(first), Some(last))
        };
        let owner = ada("runtime-auth-user", "runtime-auth@finite.test");
        let other = ada("composition-other", "other@finite.test");
        // The identity listener accepts exactly one Brain server identity, so
        // the Brain server's port is chosen before Core starts.
        let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let brain_port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let auth = scoped_test_auth();
        let config = BrainIdentityConfig::from_values(
            Some("127.0.0.1:0".to_string()),
            Some(format!("http://127.0.0.1:{brain_port}")),
            Some(brain_identity::OBSERVATION_TOKEN.to_string()),
            Some(brain_identity::DESCRIPTION_TOKEN.to_string()),
            &auth,
        )
        .unwrap()
        .unwrap();
        let (core_url, core) = serve(router(db.store.clone(), test_auth())).await;
        let (identity_url, identity) =
            serve(brain_identity_router(db.store.clone(), auth, config)).await;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut child = tokio::process::Command::new("node")
            .args([
                "--import",
                "tsx",
                "scripts/brain-release-composition-proof.ts",
            ])
            .current_dir(root.join("finitecomputer-v2/apps/dashboard"))
            .stdin(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = serde_json::json!({
            "coreUrl": core_url, "identityUrl": identity_url, "brainPort": brain_port,
            "runtimeId": runtime, "owner": owner, "other": other,
            "agentSecretHex": format!("{AGENT_SECRET_BYTE:02x}").repeat(32), "agentNpub": agent,
            "observationToken": brain_identity::OBSERVATION_TOKEN,
            "descriptionToken": brain_identity::DESCRIPTION_TOKEN,
        });
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(input.to_string().as_bytes()).await.unwrap();
        drop(stdin);
        let result = tokio::time::timeout(std::time::Duration::from_secs(900), child.wait()).await;
        core.abort();
        identity.abort();
        assert!(
            result
                .expect("composition proof exceeded its deadline")
                .unwrap()
                .success()
        );
    })
    .await;
}
