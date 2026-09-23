//! Opt-in prolonged exhaustion against the disposable two-worker kind pool.
use super::*;
use crate::test_support::TestDb;
use std::process::Command;
use std::time::Instant;

struct Holder(String);

impl Holder {
    fn action(&self, action: &str) -> bool {
        let output = Command::new(required("FC_TEST_SUBSTRATE_ATE_CLI"))
            .args([
                "--kubeconfig",
                &required("FC_TEST_SUBSTRATE_CRASH_KUBECONFIG"),
                "--context",
                "kind-finite-hermes-spike",
                action,
                "actor",
                &self.0,
                "-a",
                "finite-hermes-spike",
            ])
            .output();
        output.is_ok_and(|output| output.status.success())
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        // Keep the explicitly supplied disposable holder stopped even on failure.
        if !self.action("suspend") {
            eprintln!("disposable capacity holder suspension failed");
        }
    }
}

async fn identity(db: &TestDb, request: &str) -> Value {
    db.query_json(
        "SELECT jsonb_build_object('runtime',q.agent_runtime_id,'spec',q.runtime_spec,
         'correlation',p.correlation_id,'credential',c.token_sha256)
         FROM agent_creation_requests q
         JOIN agent_creation_provider_operations p ON p.agent_creation_request_id=q.id
         JOIN runtime_core_credentials c ON c.creation_request_id=q.id
         WHERE q.id=$1",
        &[&request],
    )
    .await
    .into_iter()
    .next()
    .expect("one durable creation and credential")
}

pub(super) struct CapacityWait {
    identity: Value,
    actor: String,
    uid: Value,
    released: Instant,
}

impl CapacityWait {
    pub(super) async fn verify(self, db: &TestDb, request: &str) {
        assert!(
            self.identity == identity(db, request).await,
            "creation identity changed during retry"
        );
        let actor = local_substrate_resource("actor", &self.actor);
        assert_eq!(actor["metadata"]["uid"], self.uid);
        assert_eq!(actor["status"]["state"], "ACTOR_STATE_RUNNING");
        assert!(
            self.released.elapsed() < Duration::from_secs(120),
            "retry waited for the 600-second lease"
        );
        eprintln!(
            "capacity exhaustion recovered with unchanged identity in {:?}",
            self.released.elapsed()
        );
    }
}

pub(super) async fn exhaust(
    db: &TestDb,
    request: &str,
    mut command: Command,
) -> Option<CapacityWait> {
    let name = std::env::var("FC_TEST_SUBSTRATE_CAPACITY_HOLDER").ok()?;
    let initial = local_substrate_resource("actor", &name);
    assert_eq!(initial["status"]["state"], "ACTOR_STATE_SUSPENDED");
    let holder = Holder(name);
    assert!(holder.action("resume"), "resume disposable capacity holder");
    assert_eq!(
        local_substrate_resource("actor", &holder.0)["status"]["state"],
        "ACTOR_STATE_RUNNING"
    );
    let started = Instant::now();
    let output = tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap();
    // Do not echo the Runner's potentially credential-bearing diagnostics.
    assert!(
        output.status.success(),
        "capacity-waiting run-once must succeed"
    );
    let outcome: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(outcome["status"], "capacity_unavailable");
    assert!(
        started.elapsed() >= Duration::from_secs(30),
        "provider retries were not exhausted"
    );
    let creation = db.agent_creation_request(request).await.unwrap();
    assert_eq!(
        creation.status,
        crate::AgentCreationRequestStatus::Launching
    );
    let expired = db.query_json("SELECT to_jsonb(lease_expires_at <= CURRENT_TIMESTAMP) FROM agent_creation_requests WHERE id=$1", &[&request]).await;
    assert_eq!(expired, vec![json!(true)]);
    let identity = identity(db, request).await;
    let actor = creation
        .agent_runtime_id
        .unwrap()
        .replacen("runtime_", "runtime-", 1);
    let uid = local_substrate_resource("actor", &actor)["metadata"]["uid"].clone();
    assert!(uid.is_string());
    drop(holder);
    Some(CapacityWait {
        identity,
        actor,
        uid,
        released: Instant::now(),
    })
}
