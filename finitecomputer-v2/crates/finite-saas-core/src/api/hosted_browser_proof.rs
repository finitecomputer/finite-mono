//! Opt-in composition proof. Only WorkOS's external identity source and the
//! unrelated gateway/Chat children are fixtures. Core, Postgres, dashboard,
//! agentd's pull/supervisor, native Hermes and the rendered Caddy routes are real.
use super::*;
use crate::auth::test_support::{access_token_with_subject, core_auth};
use crate::store::runtime_credentials::tests::{
    complete, provision, register, requested, upgrade, upgrade_input,
};
use crate::test_support::with_isolated_postgres;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

#[tokio::test]
#[ignore = "requires pinned Node/Hermes/Caddy, built Runner/agentd/dashboard and Chromium; see scripts/proofs/README.md"]
async fn hosted_browser_composition() {
    // Both cases reserve Hermes port 8642, so run them sequentially.
    for existing_agent in [false, true] {
        with_isolated_postgres(move |db| async move {
        let request = requested(&db).await;
        let launch_credential = if existing_agent { None } else {
            Some(db.provision_runtime_credential(provision(&request)).await.unwrap().secret)
        };
        let runtime = register(&db, &request).await;
        complete(&db, &request).await.unwrap();
        let bootstrap = match launch_credential {
            Some(secret) => secret,
            None => {
                assert!(!db.hosted_access(&runtime, "runtime-auth-user").await.unwrap().enrolled);
                let lease = upgrade(&db, &request).await;
                db.provision_upgrade_credential(upgrade_input(&lease)).await.unwrap().secret
            }
        };
        let owner = access_token_with_subject("runtime-auth-user", "runtime-auth@finite.test", true, None);
        let other = access_token_with_subject("other-user", "other@finite.test", true, None);
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let runtime_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let runtime_url = format!("http://{}", runtime_listener.local_addr().unwrap());
        let mut child = Command::new("node")
            .arg("scripts/proofs/hosted-hermes-dashboard.mjs")
            .current_dir(&directory)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true).spawn().unwrap();
        let mut input = child.stdin.take().unwrap();
        input.write_all(format!("{}\n", serde_json::json!({"enrollment":if existing_agent { "existing-upgrade" } else { "new-launch" },"runtimeId":runtime,"bootstrap":bootstrap,"runtimeUrl":runtime_url,"owner":owner,"other":other})).as_bytes()).await.unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
        let prepared = tokio::time::timeout(std::time::Duration::from_secs(60), output.next_line()).await.unwrap().unwrap().expect("browser fixture exited before TLS setup");
        let prepared: Value = serde_json::from_str(&prepared).unwrap();
        let port = prepared["port"].as_u64().unwrap() as u16;
        let origins = HostedHermesOrigins::from_json(&serde_json::json!({"auth-host":format!("https://hermes-proof.test:{port}")}).to_string()).unwrap();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .resolve("hermes-proof.test", ([127,0,0,1], port).into())
            .add_root_certificate(reqwest::Certificate::from_pem(prepared["ca"].as_str().unwrap().as_bytes()).unwrap())
            .build().unwrap();
        let app = router_from_state(CoreApiState {
            store: db.store.clone(), auth: core_auth("service", "runner", "usage"),
            standard_stripe_price_id: None, agent_creation_placement: None,
            runtime_upgrades_enabled: false, runtime_retirement_enabled: false,
            hosted_hermes_origins: origins.clone(),
            native_sessions: crate::hosted_hermes_session::NativeSessions::with_test_client(client),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let core_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let runtime_app = runtime_router(db.store.clone(), origins);
        let runtime_server = tokio::spawn(async move { axum::serve(runtime_listener, runtime_app).await.unwrap() });
        input.write_all(format!("{}\n", serde_json::json!({"coreUrl":core_url})).as_bytes()).await.unwrap();
        drop(input);
        let result = tokio::time::timeout(std::time::Duration::from_secs(240), async {
            while let Some(line) = output.next_line().await.unwrap() { println!("{line}"); }
            child.wait().await.unwrap()
        }).await;
        server.abort();
        runtime_server.abort();
        assert!(result.expect("browser composition exceeded deadline").success());
    }).await;
    }
}
