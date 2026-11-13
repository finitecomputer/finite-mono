use finite_saas_core::{
    auth::{CoreAuth, RunnerCredentialConfig, WorkosAuthenticator, WorkosAuthenticatorConfig},
    launch_codes::IssueLaunchCodeBatchInput,
    store::CoreStore,
    *,
};
use tokio_postgres::{Client, NoTls};

pub(super) struct CoreFixture {
    pub base_url: String,
    pub runner_token: String,
    pub request_id: String,
    runtime: tokio::runtime::Runtime,
    admin: Client,
    raw: Client,
    db_name: String,
    server: tokio::task::JoinHandle<()>,
}

impl CoreFixture {
    pub fn new() -> Self {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (admin, raw, db_name, base_url, runner_token, request_id, server) =
            runtime.block_on(async {
                let admin_url = std::env::var("FC_CORE_POSTGRES_TEST_URL")
                    .expect("use devfinity for the admission/Core proof");
                let (admin, connection) = tokio_postgres::connect(&admin_url, NoTls).await.unwrap();
                tokio::spawn(async move {
                    connection.await.unwrap();
                });
                let db_name = format!("runner_admission_{:032x}", rand::random::<u128>());
                admin
                    .batch_execute(&format!("CREATE DATABASE \"{db_name}\""))
                    .await
                    .unwrap();
                let mut url = url::Url::parse(&admin_url).unwrap();
                url.set_path(&db_name);
                let store = CoreStore::connect(url.as_str()).await.unwrap();
                store.migrate().await.unwrap();
                let (raw, connection) = tokio_postgres::connect(url.as_str(), NoTls).await.unwrap();
                tokio::spawn(async move {
                    let _ = connection.await;
                });
                store
                    .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                        id: "artifact-admission".into(),
                        kind: RuntimeArtifactKind::OciImage,
                        reference: format!(
                            "ghcr.io/finitecomputer/agent-runtime@sha256:{}",
                            "a".repeat(64)
                        ),
                        version_label: "admission-test".into(),
                        source_git_sha: None,
                        finitec_version: None,
                        hermes_source_ref: None,
                        finite_platform_plugin_ref: None,
                        state_schema_version: "state-v1".into(),
                        base_image: None,
                        canary_runtime_id: None,
                        recover_known_good_chat: false,
                        promoted: true,
                        now: None,
                    })
                    .await
                    .unwrap();
                let code = store
                    .issue_launch_code_batch(IssueLaunchCodeBatchInput {
                        name: "Admission test".into(),
                        code_count: 1,
                        expires_in_hours: Some(24),
                        hosting_tier: None,
                        created_by_workos_user_id: "test-operator".into(),
                        now: None,
                    })
                    .await
                    .unwrap()
                    .codes[0]
                    .code
                    .clone();
                let request = store
                    .request_agent_creation_configured(
                        RequestAgentCreationInput {
                            verified_email: "admission@finite.test".into(),
                            workos_user_id: "admission-owner".into(),
                            display_name: "Admission test".into(),
                            launch_code: code,
                            idempotency_key: "admission-test".into(),
                            now: None,
                        },
                        AgentCreationConfiguration {
                            placement: Some(RuntimePlacement {
                                runner_class: RunnerClass::Kata,
                                runtime_resource_class: RuntimeResourceClass::Vcpu4Memory8Gib,
                            }),
                            requested_hosting_tier: Some(HostingTier::Standard),
                            profile_picture_url: None,
                            owner_chat_account_id: None,
                        },
                    )
                    .await
                    .unwrap();
                // This proof covers the supported unenrolled-email creation path;
                // it does not call an external Identity Authority or real VM.
                raw.execute(
                    "UPDATE projects SET agent_email=NULL WHERE id=$1",
                    &[&request.request.project_id],
                )
                .await
                .unwrap();
                let runner_token = "r".repeat(64);
                let workos = WorkosAuthenticator::new(WorkosAuthenticatorConfig {
                    client_id: "test-client".into(),
                    issuer: "https://example.test".into(),
                    operator_org_id: "test-org".into(),
                    api_key: "unused-test-key".into(),
                    api_base_url: "http://127.0.0.1:9".into(),
                    jwks_url: "http://127.0.0.1:9/jwks".into(),
                })
                .unwrap();
                let auth = CoreAuth::new_with_runner_credentials(
                    workos,
                    "s".repeat(64),
                    vec![RunnerCredentialConfig {
                        credential_id: "admission-test".into(),
                        token: runner_token.clone(),
                        runner_id: "memory-runner".into(),
                        runner_classes: vec![RunnerClass::Kata],
                        source_host_id: "memory-host".into(),
                        revoked: false,
                    }],
                    "u".repeat(64),
                )
                .unwrap();
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let base_url = format!("http://{}", listener.local_addr().unwrap());
                let server = tokio::spawn(async move {
                    axum::serve(listener, api::router(store, auth))
                        .await
                        .unwrap();
                });
                (
                    admin,
                    raw,
                    db_name,
                    base_url,
                    runner_token,
                    request.request.id,
                    server,
                )
            });
        Self {
            base_url,
            runner_token,
            request_id,
            runtime,
            admin,
            raw,
            db_name,
            server,
        }
    }

    pub fn request_state(&self) -> serde_json::Value {
        self.runtime.block_on(async {
            self.raw
                .query_one(
                    "SELECT to_jsonb(r) FROM agent_creation_requests r WHERE id=$1",
                    &[&self.request_id],
                )
                .await
                .unwrap()
                .get(0)
        })
    }

    pub fn key_counts(&self) -> (i64, i64) {
        self.runtime.block_on(async {
            let row = self.raw.query_one("SELECT (SELECT count(*) FROM finite_private_api_keys), (SELECT count(*) FROM runtime_core_credentials)", &[]).await.unwrap();
            (row.get(0), row.get(1))
        })
    }

    pub fn expire_lease(&self) {
        self.runtime.block_on(self.raw.execute("UPDATE agent_creation_requests SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1", &[&self.request_id])).unwrap();
    }
}

impl Drop for CoreFixture {
    fn drop(&mut self) {
        self.server.abort();
        self.runtime
            .block_on(self.admin.batch_execute(&format!(
                "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
                self.db_name
            )))
            .unwrap();
    }
}
