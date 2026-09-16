// Real Core HTTP + Runner orchestration, with a response lost after commit.
use finite_saas_core::launch_codes::IssueLaunchCodeBatchInput;
pub use finite_saas_core::*;
#[allow(dead_code)]
#[path = "../src/test_support.rs"]
mod test_support;
use sha2::{Digest, Sha256};
use test_support::{TestDb, with_isolated_postgres};
fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn hex_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn runner_auth() -> auth::CoreAuth {
    auth::CoreAuth::new_with_runner_credentials(
        auth::WorkosAuthenticator::new(auth::WorkosAuthenticatorConfig {
            client_id: "test-client".into(),
            issuer: "https://example.invalid".into(),
            operator_org_id: "test-org".into(),
            api_key: "unused".into(),
            api_base_url: "https://example.invalid".into(),
            jwks_url: "https://example.invalid/jwks".into(),
        })
        .unwrap(),
        "service",
        vec![auth::RunnerCredentialConfig {
            credential_id: "runner".into(),
            token: "runner-secret".into(),
            runner_id: "auth-runner".into(),
            runner_classes: vec![RunnerClass::Kata],
            source_host_id: "auth-host".into(),
            revoked: false,
        }],
        "usage",
    )
    .unwrap()
}
async fn expire(db: &TestDb, request: &str) {
    db.query_json("UPDATE agent_creation_requests SET lease_expires_at=clock_timestamp()-INTERVAL '1 second' WHERE id=$1 RETURNING to_jsonb(id)", &[&request]).await;
}
async fn requested(db: &TestDb) -> String {
    let code = db
        .issue_launch_code_batch(IssueLaunchCodeBatchInput {
            name: "runtime bootstrap".into(),
            code_count: 1,
            expires_in_hours: Some(24),
            hosting_tier: None,
            created_by_workos_user_id: "operator".into(),
            now: None,
        })
        .await
        .unwrap()
        .codes[0]
        .code
        .clone();
    let request = db
        .request_agent_creation(RequestAgentCreationInput {
            verified_email: "runtime-auth@finite.test".into(),
            workos_user_id: "runtime-auth-user".into(),
            display_name: "Runtime auth".into(),
            launch_code: code,
            idempotency_key: "runtime-auth-create".into(),
            now: None,
        })
        .await
        .unwrap()
        .request;
    db.lease_agent_creation_request(LeaseAgentCreationRequestInput {
        runner_id: "auth-runner".into(),
        source_host_id: Some("auth-host".into()),
        lease_token: "test-launch-lease".into(),
        lease_seconds: Some(300),
        runner_capacity: None,
        now: None,
    })
    .await
    .unwrap()
    .unwrap();
    request.id
}
#[tokio::test]
async fn runtime_bootstrap_runner_recovers_committed_but_lost_response() {
    use finite_saas_runner::{
        AgentCreationRunner, CoreHttpAgentCreationQueue, RandomLeaseTokenSource, RunOnceOutcome,
        RunnerError, RuntimeLaunchFacts, RuntimeLaunchOptions, RuntimeLauncher,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    struct ProcessDeliveryProbe(Arc<Mutex<Option<String>>>);
    impl RuntimeLauncher for ProcessDeliveryProbe {
        fn validate_ready(&self) -> Result<(), RunnerError> {
            Ok(())
        }
        fn runtime_capabilities(&self) -> finite_saas_core::RuntimeCapabilitiesEnvelope {
            finite_saas_core::RuntimeCapabilitiesEnvelope::V1(
                finite_saas_core::RuntimeCapabilitiesV1::default(),
            )
        }
        fn runner_class(&self) -> finite_saas_core::RunnerClass {
            finite_saas_core::RunnerClass::Kata
        }
        fn runner_capacity(&self) -> finite_saas_core::RunnerLeaseCapacity {
            finite_saas_core::RunnerLeaseCapacity {
                runner_classes: vec![finite_saas_core::RunnerClass::Kata],
                ..Default::default()
            }
        }
        fn source_host_id(&self) -> Option<&str> {
            Some("auth-host")
        }
        fn launch(
            &mut self,
            _lease: &finite_saas_core::AgentCreationLease,
            options: &RuntimeLaunchOptions,
        ) -> Result<RuntimeLaunchFacts, RunnerError> {
            let output = std::process::Command::new("/bin/sh")
                .args([
                    "-c",
                    "test -n \"$FINITE_CORE_URL\" && printf %s \"$FINITE_CORE_CREDENTIAL\"",
                ])
                .env_clear()
                .envs(&options.environment)
                .envs(&options.secret_environment)
                .output()
                .unwrap();
            assert!(output.status.success());
            let delivered = String::from_utf8(output.stdout).unwrap();
            assert!(hex_secret(&delivered));
            *self.0.lock().unwrap() = Some(digest(&delivered));
            // Fault boundary after a real child received its environment;
            // this test does not simulate successful provider readiness.
            Err(RunnerError::RuntimeLaunch(
                "local delivery probe complete".into(),
            ))
        }
    }
    with_isolated_postgres(|db| async move {
            let request=requested(&db).await; expire(&db,&request).await;
            let auth = runner_auth();
            let lost=Arc::new(AtomicBool::new(false));let middleware_lost=lost.clone();
            let app=crate::api::router(db.store.clone(),auth).layer(axum::middleware::from_fn(move |request: axum::extract::Request,next:axum::middleware::Next| {
                let lost=middleware_lost.clone();
                async move {
                    let issuance=request.uri().path().ends_with("/runtime-credential");
                    let response=next.run(request).await;
                    if issuance && response.status().is_success() && !lost.swap(true,Ordering::SeqCst) {
                        use axum::response::IntoResponse;
                        // Core has really committed. A transient edge failure
                        // now withholds that successful response from Runner.
                        axum::http::StatusCode::BAD_GATEWAY.into_response()
                    } else {response}
                }
            }));
            let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base=format!("http://{}",listener.local_addr().unwrap());
            let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
            let observed=Arc::new(Mutex::new(None));
            let run=|base:String,observed:Arc<Mutex<Option<String>>>|tokio::task::spawn_blocking(move || {
                let queue=CoreHttpAgentCreationQueue::new(base.clone(),"runner-secret").unwrap();
                let mut runner=AgentCreationRunner::new(queue,ProcessDeliveryProbe(observed),RandomLeaseTokenSource,"auth-runner",300).unwrap().with_runtime_core_bootstrap(base).unwrap().with_default_finite_private_inference(Default::default());
                runner.run_once()
            });
            assert!(matches!(run(base.clone(),observed.clone()).await.unwrap(),Err(RunnerError::RuntimeBootstrapUnavailable)));
            assert!(lost.load(Ordering::SeqCst));
            assert!(observed.lock().unwrap().is_none());
            let rows=db.query_json("SELECT jsonb_build_object('status',q.status,'verifier',c.token_sha256) FROM agent_creation_requests q JOIN runtime_core_credentials c ON c.creation_request_id=q.id WHERE q.id=$1", &[&request]).await;
            assert_eq!(rows[0]["status"],"launching");
            let expected=rows[0]["verifier"].as_str().unwrap().to_string();
            expire(&db,&request).await;
            let result = run(base,observed.clone()).await.unwrap().unwrap();
            match result { RunOnceOutcome::LaunchFailed { failure_message, .. } => assert!(failure_message.contains("local delivery probe complete"), "unexpected launch failure: {failure_message}"), _ => panic!("expected delivery probe to finish launch") };
            assert_eq!(observed.lock().unwrap().as_deref(),Some(expected.as_str()));
            server.abort();let _=server.await;
        }).await;
}
