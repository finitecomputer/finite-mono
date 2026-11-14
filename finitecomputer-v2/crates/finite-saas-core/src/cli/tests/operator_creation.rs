//! `agent-create-for-owner-exact`: its flags, the root boundary, and the
//! real preview and execute wiring against a test database.

use super::*;
use finite_saas_core::owner_identity::{
    OwnerIdentityBackend, OwnerIdentityProvisioner, WorkosUserRecord,
};
use finite_saas_core::{CoreResult, LinkVerifiedUserInput};
use futures_util::future::BoxFuture;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const BASE: [&str; 14] = [
    "finite-saas-core",
    "agent-create-for-owner-exact",
    "--owner-email",
    "frankie@finite.vip",
    "--display-name",
    "Frankie Bot",
    "--reason",
    "box1 migration",
    "--idempotency-key",
    "box1-frankie",
    "--admin-email",
    "austin@finite.vip",
    "--admin-workos-user-id",
    "user_admin",
];

fn parse(extra: &[&str]) -> AgentCreateForOwnerExactCliArgs {
    let Some(Command::AgentCreateForOwnerExact(parsed)) =
        Args::try_parse_from(BASE.iter().chain(extra))
            .unwrap()
            .command
    else {
        panic!("expected agent-create-for-owner-exact");
    };
    parsed
}

#[test]
fn agent_create_for_owner_requires_reason_admin_and_key_and_defaults_to_preview() {
    let parsed = parse(&[]);
    assert!(!parsed.execute);
    assert!(!parsed.existing_account_confirmed);
    let input = operator_create_input(&parsed, "root".into(), None).unwrap();
    assert_eq!(
        input.surface,
        finite_saas_core::OperatorCreationSurface::Cli
    );
    assert_eq!(input.admin.workos_user_id, "user_admin");
    assert_eq!(input.cli_os_user.as_deref(), Some("root"));
    assert_eq!(input.owner_identity, None, "never taken from flags");
    assert_eq!(input.now, None);
    for required in [
        "--owner-email",
        "--display-name",
        "--reason",
        "--idempotency-key",
        "--admin-email",
        "--admin-workos-user-id",
    ] {
        let position = BASE.iter().position(|arg| *arg == required).unwrap();
        let without: Vec<_> = BASE
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != position && *index != position + 1)
            .map(|(_, arg)| *arg)
            .collect();
        assert!(Args::try_parse_from(without).is_err(), "{required}");
    }
    let tiered = parse(&[
        "--hosting-tier",
        "confidential",
        "--execute",
        "--existing-account-confirmed",
    ]);
    assert!(tiered.execute);
    assert!(
        operator_create_input(&tiered, "root".into(), None)
            .unwrap()
            .existing_account_confirmed
    );
    assert_eq!(
        operator_create_input(&tiered, "root".into(), None)
            .unwrap()
            .hosting_tier,
        Some(finite_saas_core::HostingTier::Confidential)
    );
    // No backdated audit time and no operator-supplied chat account.
    for refused in [
        &["--hosting-tier", "premium"][..],
        &["--now", "2020-01-01T00:00:00Z"],
        &[
            "--owner-chat-account-id",
            "abababababababababababababababababababababababababababababababab",
        ],
    ] {
        assert!(
            Args::try_parse_from(BASE.iter().chain(refused)).is_err(),
            "{refused:?}"
        );
    }
}

#[tokio::test]
async fn the_command_refuses_a_non_root_user_before_reading_anything() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let error = agent_create_for_owner_exact_command(parse(&["--execute"]))
        .await
        .err()
        .expect("non-root is refused");
    assert_eq!(
        error.to_string(),
        "agent-create-for-owner-exact requires OS root"
    );
}

#[derive(Default)]
struct CliBackend {
    created: AtomicUsize,
    chat_reads: AtomicUsize,
}

const OWNER_CHAT: &str = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";

impl OwnerIdentityBackend for CliBackend {
    fn workos_users_by_email<'a>(
        &'a self,
        email: &'a str,
    ) -> BoxFuture<'a, CoreResult<Vec<WorkosUserRecord>>> {
        Box::pin(async move {
            Ok((self.created.load(Ordering::SeqCst) > 0)
                .then(|| WorkosUserRecord {
                    id: "user_cli_owner".into(),
                    email: email.into(),
                    email_verified: false,
                })
                .into_iter()
                .collect())
        })
    }
    fn create_workos_user<'a>(
        &'a self,
        email: &'a str,
    ) -> BoxFuture<'a, CoreResult<WorkosUserRecord>> {
        Box::pin(async move {
            self.created.fetch_add(1, Ordering::SeqCst);
            Ok(WorkosUserRecord {
                id: "user_cli_owner".into(),
                email: email.into(),
                email_verified: false,
            })
        })
    }
    fn workos_user<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, CoreResult<Option<WorkosUserRecord>>> {
        Box::pin(async move {
            Ok((id == "user_admin").then(|| WorkosUserRecord {
                id: id.into(),
                email: "austin@finite.vip".into(),
                email_verified: true,
            }))
        })
    }
    fn operator_membership_active<'a>(&'a self, id: &'a str) -> BoxFuture<'a, CoreResult<bool>> {
        Box::pin(async move { Ok(id == "user_admin") })
    }
    fn hosted_chat_account_id<'a>(&'a self, _id: &'a str) -> BoxFuture<'a, CoreResult<String>> {
        Box::pin(async move {
            self.chat_reads.fetch_add(1, Ordering::SeqCst);
            Ok(OWNER_CHAT.to_string())
        })
    }
}

static DATABASES: AtomicU64 = AtomicU64::new(0);

/// A fresh migrated database, so the test can see exactly what committed.
async fn fresh_database(admin_url: &str) -> String {
    let name = format!(
        "cli_operator_{}_{}",
        std::process::id(),
        DATABASES.fetch_add(1, Ordering::SeqCst)
    );
    let (client, connection) = tokio_postgres::connect(admin_url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(connection);
    client
        .batch_execute(&format!("CREATE DATABASE {name}"))
        .await
        .unwrap();
    let (base, query) = admin_url
        .split_once('?')
        .map_or((admin_url, None), |(b, q)| (b, Some(q)));
    let path_start = base.find("://").map_or(0, |index| index + 3);
    let url = match base[path_start..].find('/') {
        Some(slash) => format!("{}/{name}", &base[..path_start + slash]),
        None => format!("{base}/{name}"),
    };
    let url = query.map_or(url.clone(), |query| format!("{url}?{query}"));
    let store = CoreStore::connect(&url).await.unwrap();
    store.migrate().await.unwrap();
    store
        .link_verified_user(LinkVerifiedUserInput {
            verified_email: "austin@finite.vip".into(),
            workos_user_id: "user_admin".into(),
            now: None,
        })
        .await
        .unwrap();
    url
}

async fn count(url: &str, table: &str) -> i64 {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(connection);
    client
        .query_one(&format!("SELECT count(*) FROM {table}"), &[])
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
async fn the_preview_commits_nothing_anywhere_and_execute_commits_once() {
    let Ok(admin_url) = env::var("FC_CORE_POSTGRES_TEST_URL") else {
        return;
    };
    let url = fresh_database(&admin_url).await;
    let backend = Arc::new(CliBackend::default());
    let provisioner =
        OwnerIdentityProvisioner::new(backend.clone() as Arc<dyn OwnerIdentityBackend>);

    let preview =
        run_agent_create_for_owner(&parse(&[]), &url, &provisioner, "austin".into(), None)
            .await
            .unwrap();
    assert!(preview.dry_run);
    assert!(!preview.outcome.result.owner_identity_provisioned);
    let identity_preview = preview.outcome.owner_identity_preview.clone().unwrap();
    assert!(identity_preview.would_create_workos_user);
    assert_eq!(backend.created.load(Ordering::SeqCst), 0, "no WorkOS user");
    assert_eq!(
        backend.chat_reads.load(Ordering::SeqCst),
        0,
        "no chat identity"
    );
    assert_eq!(count(&url, "operator_agent_creations").await, 0);
    assert_eq!(count(&url, "projects").await, 0);
    assert_eq!(count(&url, "users").await, 1, "only the admin");

    let executed = run_agent_create_for_owner(
        &parse(&["--execute"]),
        &url,
        &provisioner,
        "austin".into(),
        None,
    )
    .await
    .unwrap();
    assert!(!executed.dry_run);
    let record = &executed.outcome.result.record;
    assert_eq!(record.owner.workos_user_id, "user_cli_owner");
    assert_eq!(record.owner.chat_account_id, OWNER_CHAT);
    assert_eq!(record.actor.cli_os_user.as_deref(), Some("austin"));
    assert_eq!(count(&url, "operator_agent_creations").await, 1);
    assert_eq!(backend.created.load(Ordering::SeqCst), 1);

    // The admin is verified with WorkOS before Core is touched.
    let mut impostor = parse(&["--execute"]);
    impostor.admin_workos_user_id = "user_member".into();
    impostor.idempotency_key = "impostor".into();
    let error = run_agent_create_for_owner(&impostor, &url, &provisioner, "austin".into(), None)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("operator must be"), "{error}");
    assert_eq!(count(&url, "operator_agent_creations").await, 1);
}
