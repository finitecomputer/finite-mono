//! Recover disposable launches after transport loss or invalid Core boot flags.
use super::*;
use crate::test_support::TestDb;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

pub(super) struct Fault {
    armed: Arc<AtomicBool>,
    attempts: Arc<AtomicUsize>,
    invalid_config: bool,
}

impl Fault {
    pub fn new() -> Self {
        Self {
            armed: Arc::new(AtomicBool::new(
                std::env::var_os("FC_TEST_SUBSTRATE_STALLED_BOOT").is_some()
                    || std::env::var_os("FC_TEST_SUBSTRATE_INVALID_BOOT").is_some(),
            )),
            attempts: Arc::new(AtomicUsize::new(0)),
            invalid_config: std::env::var_os("FC_TEST_SUBSTRATE_INVALID_BOOT").is_some(),
        }
    }

    pub fn layer(
        &self,
        store: crate::store::CoreStore,
        origins: HostedHermesOrigins,
        environment: &BTreeMap<String, String>,
    ) -> axum::Router {
        let mut invalid_environment = environment.clone();
        invalid_environment.insert("FINITE_AGENTD_BRIDGE_ADDR".into(), "0.0.0.0:37633".into());
        // Use the real authenticated Core endpoint and configuration validator.
        let invalid_router = runtime_router(
            store
                .clone()
                .with_runtime_environment(invalid_environment)
                .unwrap(),
            origins.clone(),
        );
        let router = runtime_router(store, origins);
        let invalid_config = self.invalid_config;
        let armed = self.armed.clone();
        let attempts = self.attempts.clone();
        router.layer(axum::middleware::from_fn(
            move |request: Request<Body>, next: axum::middleware::Next| {
                let (armed, attempts) = (armed.clone(), attempts.clone());
                let invalid_router = invalid_router.clone();
                async move {
                    if armed.load(Ordering::SeqCst)
                        && request.uri().path() == "/api/core/v1/runtime/environment"
                    {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        if invalid_config {
                            return invalid_router.oneshot(request).await.unwrap();
                        }
                        return axum::response::IntoResponse::into_response(
                            StatusCode::SERVICE_UNAVAILABLE,
                        );
                    }
                    next.run(request).await
                }
            },
        ))
    }

    pub async fn interrupt(
        &self,
        db: &TestDb,
        request: &str,
        command: std::process::Command,
    ) -> Option<Value> {
        if !self.armed.load(Ordering::SeqCst) {
            return None;
        }
        let mut command = tokio::process::Command::from(command);
        let mut child = command
            .env("FC_RUNNER_LEASE_SECONDS", "60")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        if self.invalid_config {
            let status = tokio::time::timeout(Duration::from_secs(360), child.wait())
                .await
                .expect("Runner must return after failed boot")
                .unwrap();
            assert!(!status.success(), "invalid boot unexpectedly succeeded");
            assert!(
                self.attempts.load(Ordering::SeqCst) > 0,
                "runtime never fetched invalid config"
            );
            assert_eq!(
                db.agent_creation_request(request).await.unwrap().status,
                crate::AgentCreationRequestStatus::Launching,
                "failed boot must preserve the retryable creation"
            );
        } else {
            tokio::time::timeout(Duration::from_secs(120), async {
                while self.attempts.load(Ordering::SeqCst) < 3 {
                    assert!(
                        child.try_wait().unwrap().is_none(),
                        "Runner exited before bootstrap failure"
                    );
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            })
            .await
            .expect("bootstrap did not exhaust its environment retries");
            // Allow the third HTTP response to reach the boot process before killing Runner.
            tokio::time::sleep(Duration::from_secs(1)).await;
            child.kill().await.unwrap();
            child.wait().await.unwrap();
        }
        self.armed.store(false, Ordering::SeqCst);
        let creation = db.agent_creation_request(request).await.unwrap();
        let actor = creation
            .agent_runtime_id
            .as_deref()
            .unwrap()
            .replacen("runtime_", "runtime-", 1);
        let before = local_substrate_resource("actor", &actor);
        assert_eq!(
            before["status"]["state"], "ACTOR_STATE_RESUMING",
            "fixture must reproduce a stranded bootstrap"
        );
        let identity = capacity::identity(db, request).await;
        let expiry = time::OffsetDateTime::parse(
            creation.lease_expires_at.as_deref().unwrap(),
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();
        let remaining = (expiry - time::OffsetDateTime::now_utc())
            .whole_milliseconds()
            .max(0) as u64;
        tokio::time::sleep(Duration::from_millis(remaining + 100)).await;
        eprintln!(
            "reproduced failed boot (invalid configuration: {}); corrected Core config and retrying after natural lease expiry",
            self.invalid_config
        );
        Some(identity)
    }
}
