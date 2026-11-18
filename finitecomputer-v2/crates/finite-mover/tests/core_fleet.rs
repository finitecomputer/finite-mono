//! The fleet from Core's database (M1), on a fresh database built from
//! Core's own schema (`CORE_SCHEMA_SQL`), with every query run as a role
//! granted exactly `core-role.sql`. The host reader is a stand-in that
//! prints a `sizes` summary for one tree and fails for another.
//!
//! Needs `FC_CORE_POSTGRES_TEST_URL`, as Core's own tests do (`just test`
//! and CI supply it through devfinity).

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use finite_mover::contract::{CommandKind, Has, Sizes};
use finite_mover::fleet::{Fleet, FleetError};
use finite_mover::fleet_pg::PgFleet;
use finite_mover::sizes::SizesCache;
use serde_json::{Value, json};
use tokio_postgres::NoTls;

static DATABASES: AtomicU32 = AtomicU32::new(0);

const STOP: CommandKind = CommandKind::StopAndStage;

fn public_key(byte: u8) -> nostr::PublicKey {
    nostr::Keys::new(nostr::SecretKey::from_slice(&[byte; 32]).unwrap()).public_key()
}

/// A key's npub, as Core pins an agent's.
fn npub(byte: u8) -> String {
    use nostr::ToBech32;
    public_key(byte).to_bech32().unwrap()
}

/// A key's hex, as Core keeps a person's.
fn key_hex(byte: u8) -> String {
    public_key(byte).to_hex()
}

/// A fresh database with Core's schema, a role granted `core-role.sql`
/// there, and the URL that connects as that role. Both are dropped after.
struct CoreDb {
    admin_url: String,
    name: String,
    role: String,
    url: String,
    as_role: String,
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

async fn execute(url: &str, sql: &str) {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await.unwrap();
    let driver = tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .batch_execute(sql)
        .await
        .unwrap_or_else(|error| panic!("{error:?}\n{sql}"));
    drop(client);
    let _ = driver.await;
}

/// `url` with its database named `name`, and `query` added.
fn with_database(url: &str, name: &str, query: &str) -> String {
    let (base, existing) = match url.split_once('?') {
        Some((base, existing)) => (base, Some(existing)),
        None => (url, None),
    };
    let authority = base.find("://").map_or(0, |at| at + 3);
    let base = match base[authority..].find('/') {
        Some(slash) => format!("{}/{name}", &base[..authority + slash]),
        None => format!("{base}/{name}"),
    };
    let query = [existing, Some(query)]
        .into_iter()
        .flatten()
        .filter(|q| !q.is_empty())
        .collect::<Vec<_>>()
        .join("&");
    if query.is_empty() {
        base
    } else {
        format!("{base}?{query}")
    }
}

impl CoreDb {
    fn new() -> Self {
        let admin_url = std::env::var("FC_CORE_POSTGRES_TEST_URL").expect(
            "FC_CORE_POSTGRES_TEST_URL is required for Core Postgres tests; run `just test`",
        );
        let unique = format!(
            "{}_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            DATABASES.fetch_add(1, Ordering::Relaxed)
        );
        let name = format!("fc_mover_test_{unique}");
        let role = format!("fc_mover_role_{unique}");
        let url = with_database(&admin_url, &name, "");
        block_on(async {
            execute(&admin_url, &format!("CREATE DATABASE \"{name}\"")).await;
            execute(&admin_url, &format!("CREATE ROLE \"{role}\" NOLOGIN")).await;
            execute(&url, finite_saas_core::CORE_SCHEMA_SQL).await;
            let grants =
                include_str!("../core-role.sql").replace("finite_mover", &format!("\"{role}\""));
            execute(&url, &grants).await;
        });
        // Every query the mover makes runs as the role, as on lat-2.
        let as_role = with_database(&admin_url, &name, &format!("options=-c%20role%3D{role}"));
        Self {
            admin_url,
            name,
            role,
            url,
            as_role,
        }
    }
}

impl Drop for CoreDb {
    fn drop(&mut self) {
        let (admin, name, role) = (self.admin_url.clone(), self.name.clone(), self.role.clone());
        // Dropped on its own thread: a panicking test may hold a runtime.
        let _ = std::thread::spawn(move || {
            block_on(async {
                execute(
                    &admin,
                    &format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"),
                )
                .await;
                execute(&admin, &format!("DROP ROLE IF EXISTS \"{role}\"")).await;
            })
        })
        .join();
    }
}

fn spec(runtime: &str, durable: &str) -> String {
    json!({ "schema": "runtime_spec.v1", "spec": { "projectId": "-", "agentRuntimeId": runtime, "durableStateId": durable } }).to_string()
}

/// One old agent on a Kata host: its project, runtime, link and creation
/// request, `running` for that runtime.
fn agent(
    org: &str,
    owner: &str,
    project: &str,
    runtime: &str,
    active: bool,
    spec: Option<&str>,
) -> String {
    let spec = spec.map_or("NULL".to_string(), |spec| format!("'{spec}'::jsonb"));
    format!(
        "INSERT INTO projects (id, customer_org_id, owner_user_id, display_name, created_at, updated_at)
           VALUES ('{project}', '{org}', '{owner}', 'Agent {project}', now(), now());
         INSERT INTO agent_runtimes (id, project_id, source_host_id, source_machine_id, source_import_key, placement_runner_class, runtime_resource_class, host_facts, created_at, updated_at)
           VALUES ('{runtime}', '{project}', 'lat-a', 'machine_{runtime}', 'key_{runtime}', 'kata', 'vcpu2_memory4_gib', '{{}}', now(), now());
         INSERT INTO project_runtime_links (id, project_id, agent_runtime_id, active, created_at)
           VALUES ('link_{runtime}', '{project}', '{runtime}', {active}, now());
         INSERT INTO agent_creation_requests (id, customer_org_id, owner_user_id, project_id, idempotency_key, display_name, runner_class, status, agent_runtime_id, profile_picture_url, runtime_spec, created_at, updated_at)
           VALUES ('request_{project}', '{org}', '{owner}', '{project}', 'idem_{project}', 'Agent {project}', 'kata', 'running', '{runtime}', 'https://pictures.example/{project}.png', {spec}, now() - interval '1 hour', now());"
    )
}

fn fleet_fixture() -> String {
    let mut sql = String::from(
        "INSERT INTO users (id, normalized_email, link_status, workos_user_id, created_at, updated_at) VALUES
           ('u_one', 'one@example.test', 'linked', 'user_w1', now(), now()),
           ('u_two', 'two@example.test', 'linked', 'user_w2', now(), now()),
           ('u_pending', 'pending@example.test', 'pending', NULL, now(), now());
         INSERT INTO customer_orgs (id, owner_user_id, name, billing_class, created_at, updated_at) VALUES
           ('org_one', 'u_one', 'One', 'standard', now(), now()),
           ('org_two', 'u_two', 'Two', 'standard', now(), now()),
           ('org_pending', 'u_pending', 'Pending', 'standard', now(), now());",
    );
    // Listed: live, its spec its own.
    sql += &agent(
        "org_one",
        "u_one",
        "proj_live",
        "rt_live",
        true,
        Some(&spec("rt_live", "tree_live")),
    );
    sql += &agent(
        "org_two",
        "u_two",
        "proj_two",
        "rt_two",
        true,
        Some(&spec("rt_two", "tree_two")),
    );
    // Each signs with its own key, which Core pinned from its reports.
    sql += &format!(
        "UPDATE agent_runtimes SET health_reporting_npub = '{}' WHERE id = 'rt_live';
         UPDATE agent_runtimes SET health_reporting_npub = '{}' WHERE id = 'rt_two';",
        npub(1),
        npub(2)
    );
    // `u_one`'s keys: two hosted keys Core observed, one since retired,
    // and the chat account `proj_live` was created for (FIN-499).
    sql += &format!(
        "INSERT INTO account_brain_principals (id, user_id, public_key_hex, source, issuer, status, first_observed_at, last_observed_at, revision) VALUES
           ('principal_one', 'u_one', '{}', 'hosted_device_observation', 'finite-dashboard', 'active', now(), now(), 1),
           ('principal_old', 'u_one', '{}', 'hosted_device_observation', 'finite-dashboard', 'retired', now(), now(), 2);
         UPDATE agent_creation_requests SET owner_chat_account_id = '{}' WHERE project_id = 'proj_live';",
        key_hex(3),
        key_hex(5),
        key_hex(4)
    );
    // Listed: launched before runtime specs; Core synthesizes its spec with
    // the runtime's own id.
    sql += &agent("org_one", "u_one", "proj_legacy", "rt_legacy", true, None);
    // Listed: its spec names the machine, which Core repairs to the runtime.
    sql += &agent(
        "org_one",
        "u_one",
        "proj_machine",
        "rt_machine",
        true,
        Some(&spec("rt_machine", "machine_rt_machine")),
    );
    // Listed once: cold-relocated, so it has two running creation rows;
    // the newer one's spec is current, the first one's picture is its own.
    sql += &agent(
        "org_one",
        "u_one",
        "proj_relocated",
        "rt_relocated",
        true,
        Some(&spec("rt_relocated", "tree_relocated_before")),
    );
    sql += &format!(
        "INSERT INTO agent_creation_requests (id, customer_org_id, owner_user_id, project_id, idempotency_key, display_name, runner_class, status, agent_runtime_id, relocation_spec, runtime_spec, created_at, updated_at)
           VALUES ('request_relocation', 'org_one', 'u_one', 'proj_relocated', 'idem_relocation', 'Agent proj_relocated', 'kata', 'running', 'rt_relocated', '{{\"relocation\":{{}}}}', '{}'::jsonb, now(), now());",
        spec("rt_relocated", "tree_relocated")
    );
    // Not listed: marked moved.
    sql += &agent(
        "org_one",
        "u_one",
        "proj_moved",
        "rt_moved",
        true,
        Some(&spec("rt_moved", "tree_moved")),
    );
    sql += "INSERT INTO agent_runtime_moves (agent_runtime_id, project_id, moved, moved_at, agent_npub)
              VALUES ('rt_moved', 'proj_moved', 'archived', now(), NULL);";
    // Not listed, no binding: its live runtime has no running creation.
    sql += &agent(
        "org_one",
        "u_one",
        "proj_respun",
        "rt_respun_old",
        false,
        Some(&spec("rt_respun_old", "tree_respun")),
    );
    sql += "INSERT INTO agent_runtimes (id, project_id, source_host_id, source_machine_id, source_import_key, placement_runner_class, runtime_resource_class, host_facts, created_at, updated_at)
              VALUES ('rt_respun_new', 'proj_respun', 'lat-a', 'machine_new', 'key_respun_new', 'kata', 'vcpu2_memory4_gib', '{}', now() + interval '1 minute', now());
            INSERT INTO project_runtime_links (id, project_id, agent_runtime_id, active, created_at)
              VALUES ('link_respun_new', 'proj_respun', 'rt_respun_new', true, now());";
    // Not listed, no binding: its spec names another runtime.
    sql += &agent(
        "org_one",
        "u_one",
        "proj_foreign",
        "rt_foreign",
        true,
        Some(&spec("rt_elsewhere", "tree_elsewhere")),
    );
    // Not listed, no binding: not on a Kata host.
    sql += &agent(
        "org_one",
        "u_one",
        "proj_phala",
        "rt_phala",
        true,
        Some(&spec("rt_phala", "tree_phala")),
    );
    sql += "UPDATE agent_runtimes SET placement_runner_class = 'phala' WHERE id = 'rt_phala';";
    // Not listed: its owner never signed in.
    sql += &agent(
        "org_pending",
        "u_pending",
        "proj_pending",
        "rt_pending",
        true,
        Some(&spec("rt_pending", "tree_pending")),
    );
    // Not listed: an imported candidate's project.
    sql += &agent(
        "org_one",
        "u_one",
        "proj_imported",
        "rt_imported",
        true,
        Some(&spec("rt_imported", "tree_imported")),
    );
    sql += "INSERT INTO project_import_candidates (id, source_host_id, source_machine_id, source_import_key, owner_email, pending_user_id, customer_org_id, status, host_facts, created_at, updated_at)
              VALUES ('cand_1', 'lat-a', 'machine_cand', 'key_cand', 'pending@example.test', 'u_pending', 'org_one', 'admin_review', '{}', now(), now());
            UPDATE projects SET import_candidate_id = 'cand_1' WHERE id = 'proj_imported';";
    // Not listed, but bound with its receipt: retired (its link gone) with
    // its Recovery Snapshot taken.
    sql += &agent(
        "org_one",
        "u_one",
        "proj_retired",
        "rt_retired",
        false,
        Some(&spec("rt_retired", "tree_retired")),
    );
    sql += "INSERT INTO runtime_artifacts (id, kind, reference, version_label, state_schema_version, created_at)
              VALUES ('artifact_1', 'oci_image', 'registry.example/hermes@sha256:00', 'v1', 's1', now());
            INSERT INTO runtime_control_requests (id, project_id, agent_runtime_id, source_host_id, source_machine_id, requested_by_user_id, kind, status, created_at, updated_at)
              VALUES ('rcr_retired', 'proj_retired', 'rt_retired', 'lat-a', 'machine_rt_retired', 'u_one', 'stop', 'succeeded', now(), now());
            INSERT INTO runtime_retirement_snapshots (request_id, project_id, agent_runtime_id, durable_state_id, runtime_artifact_id, schema_version, backend, locator, zip_bytes, zip_sha256, manifest_sha256, created_at, verified_at, recovery_authority_id, retention_policy, stored_at)
              VALUES ('rcr_retired', 'proj_retired', 'rt_retired', 'tree_retired', 'artifact_1', 'runtime_retirement_snapshot.v1', 'borg', 'borg::retired', 1, repeat('a', 64), repeat('b', 64), '2026-11-10T00:00:00Z', '2026-11-10T00:00:00Z', 'authority', 'indefinite_until_purge', now());";
    sql
}

/// The host reader: `sizes tree_live` prints its summary; anything else
/// fails.
const HOST: &str = r#"
case "$*" in
  "sizes tree_live") printf '{"format":"x","id":"tree_live","classes":{"secret":{"files":1,"bytes":7},"config":{"files":0,"bytes":0},"cache":{"files":1,"bytes":100},"runtime":{"files":0,"bytes":0},"history":{"files":2,"bytes":20},"work":{"files":3,"bytes":3}},"staged":{"files":5,"bytes":23},"left":[],"left_more":0}' ;;
  *) echo "no such tree $2" >&2; exit 3 ;;
esac
"#;

fn write_config(dir: &Path, conninfo: &str) -> std::path::PathBuf {
    let key = dir.join("mover.key");
    std::fs::write(&key, "21".repeat(32)).unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::create_dir_all(dir.join("locks")).unwrap();
    let config = json!({
        "finite_chat": "http://127.0.0.1:9",
        "key_file": key,
        "locks": dir.join("locks"),
        "fleet": { "postgres": { "conninfo": conninfo, "sizes_file": dir.join("sizes.json") } },
        "core": { "program": ["false"], "admin_email": "mover@example.test", "admin_workos_user_id": "user_mover", "wait_seconds": 1 },
        "hosts": { "lat-a": ["sh", "-c", HOST, "host"] },
    });
    let path = dir.join("config.json");
    std::fs::write(&path, config.to_string()).unwrap();
    path
}

/// An agent as listed: old, name, picture, sizes.
type Listed = (String, String, Option<String>, Sizes);

#[test]
fn the_roster_lists_each_live_agent_of_a_signed_in_owner_once_with_its_daily_sizes() {
    let db = CoreDb::new();
    block_on(execute(&db.url, &fleet_fixture()));
    let dir = tempfile::tempdir().unwrap();
    let sizes_file = dir.path().join("sizes.json");
    // Yesterday's run measured proj_two, and an agent since gone.
    let yesterday = Sizes {
        cache: 1,
        history: 2,
        work: 3,
        staged: 5,
        total: 6,
    };
    SizesCache {
        measured_ms: 1,
        agents: [
            ("proj_two".to_string(), yesterday),
            ("proj_gone".to_string(), yesterday),
        ]
        .into(),
    }
    .write(&sizes_file)
    .unwrap();

    // Today's run, as the timer runs it: proj_two's host fails, so it keeps
    // yesterday's; proj_gone is no longer live, so it's dropped.
    let config = write_config(dir.path(), &db.as_role);
    let run = Command::new(env!("CARGO_BIN_EXE_finite-mover"))
        .arg("--config")
        .arg(&config)
        .arg("sizes")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let lines: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    // Each tree the host was asked for is the one Core's rule names.
    let unmeasured = |old: &str, tree: &str| json!({ "old": old, "unmeasured": format!("the host reader could not be started: exit status: 3: no such tree {tree}") });
    assert_eq!(
        lines,
        vec![
            unmeasured("proj_legacy", "rt_legacy"),
            unmeasured("proj_machine", "rt_machine"),
            unmeasured("proj_relocated", "tree_relocated"),
            unmeasured("proj_two", "tree_two"),
            json!({ "measured": 1, "failed": 4 }),
        ]
    );
    let cache = SizesCache::read(&sizes_file);
    let today = Sizes {
        cache: 100,
        history: 20,
        work: 3,
        staged: 23,
        total: 130,
    };
    assert_eq!(
        cache.agents,
        [
            ("proj_live".to_string(), today),
            ("proj_two".to_string(), yesterday)
        ]
        .into()
    );

    let fleet = PgFleet::new(&db.as_role, &sizes_file, None);
    let roster = fleet.roster().unwrap();
    let listed: Vec<(String, String, Vec<Listed>)> = roster
        .people
        .iter()
        .map(|person| {
            let agents = person
                .agents
                .iter()
                .map(|agent| {
                    (
                        agent.old.clone(),
                        agent.name.clone(),
                        agent.picture_url.clone(),
                        agent.sizes,
                    )
                })
                .collect();
            (person.workos_id.clone(), person.email.clone(), agents)
        })
        .collect();
    assert_eq!(
        listed,
        vec![
            (
                "user_w1".into(),
                "one@example.test".into(),
                ["proj_legacy", "proj_live", "proj_machine", "proj_relocated"]
                    .map(|old| (
                        old.to_string(),
                        format!("Agent {old}"),
                        Some(format!("https://pictures.example/{old}.png")),
                        if old == "proj_live" {
                            today
                        } else {
                            Sizes::default()
                        }
                    ))
                    .to_vec()
            ),
            (
                "user_w2".into(),
                "two@example.test".into(),
                vec![(
                    "proj_two".into(),
                    "Agent proj_two".into(),
                    Some("https://pictures.example/proj_two.png".into()),
                    yesterday
                )]
            ),
        ]
    );
    // What Core doesn't hold is listed empty, never guessed: brains come
    // whole in each agent's stage (FIN-499).
    for agent in roster.people.iter().flat_map(|person| &person.agents) {
        assert!(
            agent.brains.is_empty()
                && agent.sites.is_empty()
                && agent.last_active_ms.is_none()
                && agent.has == Has::default(),
            "{}",
            agent.old
        );
    }
    assert!(roster.snapshot.starts_with("core-") && roster.observed_ms > 0);
}

#[test]
fn a_binding_is_the_runtime_its_own_spec_names_with_any_receipt_already_taken() {
    let db = CoreDb::new();
    block_on(execute(&db.url, &fleet_fixture()));
    let dir = tempfile::tempdir().unwrap();
    let fleet = PgFleet::new(&db.as_role, &dir.path().join("sizes.json"), None);

    let live = fleet.binding("proj_live", STOP).unwrap().unwrap();
    assert_eq!(
        (
            live.old.as_str(),
            live.agent_runtime_id.as_str(),
            live.source_host_id.as_str(),
            live.source_machine_id.as_str(),
            live.durable_state_id.as_str()
        ),
        (
            "proj_live",
            "rt_live",
            "lat-a",
            "machine_rt_live",
            "tree_live"
        )
    );
    assert_eq!(live.retired_receipt, None);
    // The keys that place its brains' owners (FIN-499): its own, as Core
    // pinned it, and its person's: each active hosted key Core observed
    // for its owner, and the chat account it was created for.
    assert_eq!(live.agent_npub, Some(npub(1)), "the key Core pinned");
    let mut person = vec![key_hex(3), key_hex(4)];
    person.sort();
    assert_eq!(live.person_keys, person);
    // The same person's other agent, created without a chat account: the
    // person's observed key alone. Another person's: none, never a guess.
    let legacy = fleet.binding("proj_legacy", STOP).unwrap().unwrap();
    assert_eq!(
        (legacy.agent_npub, legacy.person_keys),
        (None, vec![key_hex(3)])
    );
    let two = fleet.binding("proj_two", STOP).unwrap().unwrap();
    assert_eq!(
        (two.agent_npub, two.person_keys),
        (Some(npub(2)), Vec::<String>::new())
    );

    // Retired: its link is gone, its runtime and receipt remain, so a
    // retry of the retire step marks it moved without retiring it again.
    let retired = fleet.binding("proj_retired", STOP).unwrap().unwrap();
    assert_eq!(
        (
            retired.agent_runtime_id.as_str(),
            retired.retired_receipt.as_deref()
        ),
        ("rt_retired", Some("rcr_retired"))
    );

    // Each tree as Core names it for a control.
    let tree = |old: &str| {
        fleet
            .binding(old, STOP)
            .unwrap()
            .map(|binding| (binding.agent_runtime_id, binding.durable_state_id))
    };
    assert_eq!(
        tree("proj_legacy"),
        Some(("rt_legacy".into(), "rt_legacy".into())),
        "no spec: the runtime's id"
    );
    assert_eq!(
        tree("proj_machine"),
        Some(("rt_machine".into(), "rt_machine".into())),
        "Core's repair"
    );
    assert_eq!(
        tree("proj_relocated"),
        Some(("rt_relocated".into(), "tree_relocated".into())),
        "the latest running spec"
    );

    // Nothing Core's rule gives this mover to act on.
    for old in ["proj_respun", "proj_foreign", "proj_phala", "proj_unknown"] {
        assert_eq!(fleet.binding(old, STOP).unwrap(), None, "{old}");
    }

    // The live list the sizes run measures is the roster's agents.
    let mut measured: Vec<String> = fleet
        .live()
        .unwrap()
        .into_iter()
        .map(|binding| binding.old)
        .collect();
    measured.sort();
    assert_eq!(
        measured,
        [
            "proj_legacy",
            "proj_live",
            "proj_machine",
            "proj_relocated",
            "proj_two"
        ]
    );
}

/// The roster's agents by person: (WorkOS id, [old]).
fn roster_of(fleet: &PgFleet) -> Vec<(String, Vec<String>)> {
    fleet
        .roster()
        .unwrap()
        .people
        .into_iter()
        .map(|person| {
            let agents = person.agents.into_iter().map(|agent| agent.old).collect();
            (person.workos_id, agents)
        })
        .collect()
}

fn people(listed: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    listed
        .iter()
        .map(|(person, agents)| {
            (
                person.to_string(),
                agents.iter().map(|old| old.to_string()).collect(),
            )
        })
        .collect()
}

#[test]
fn a_cohort_lists_and_binds_its_agents_alone_read_again_on_each_call() {
    let db = CoreDb::new();
    block_on(execute(&db.url, &fleet_fixture()));
    let dir = tempfile::tempdir().unwrap();
    let sizes = dir.path().join("sizes.json");
    let cohort = dir.path().join("cohort.json");
    let fleet = PgFleet::new(&db.as_role, &sizes, Some(&cohort));
    // One agent of each person, one moved already, and one Core lacks:
    // those two are left out, and said by id alone.
    std::fs::write(
        &cohort,
        r#"{"old": ["proj_relocated", "proj_two", "proj_moved", "proj_unknown"]}"#,
    )
    .unwrap();
    assert_eq!(
        roster_of(&fleet),
        people(&[("user_w1", &["proj_relocated"]), ("user_w2", &["proj_two"])])
    );
    // Stopped, staged or retired only if listed; restarted whatever it lists.
    for kind in [STOP, CommandKind::Retire] {
        assert!(fleet.binding("proj_two", kind).unwrap().is_some());
        assert!(matches!(
            fleet.binding("proj_live", kind),
            Err(FleetError::Unlisted(old)) if old == "proj_live"
        ));
    }
    assert!(
        fleet
            .binding("proj_live", CommandKind::Restart)
            .unwrap()
            .is_some()
    );

    // Root adds someone: the next call has them, with no release.
    std::fs::write(
        &cohort,
        r#"{"old": ["proj_relocated", "proj_two", "proj_live"]}"#,
    )
    .unwrap();
    assert_eq!(
        roster_of(&fleet),
        people(&[
            ("user_w1", &["proj_live", "proj_relocated"]),
            ("user_w2", &["proj_two"])
        ])
    );
    assert!(fleet.binding("proj_live", STOP).unwrap().is_some());
}

#[test]
fn a_cohort_file_not_there_lists_no_one_and_one_unreadable_is_an_error() {
    let db = CoreDb::new();
    block_on(execute(&db.url, &fleet_fixture()));
    let dir = tempfile::tempdir().unwrap();
    let sizes = dir.path().join("sizes.json");
    let cohort = dir.path().join("cohort.json");
    let fleet = PgFleet::new(&db.as_role, &sizes, Some(&cohort));

    // Named but not there: no one, never everyone.
    assert_eq!(roster_of(&fleet), people(&[]));
    assert!(matches!(
        fleet.binding("proj_live", STOP),
        Err(FleetError::Unlisted(_))
    ));

    // Not its shape, or not readable: the call fails, and the next one
    // reads it again. A restart never reads it, so it still starts.
    for broken in [
        r#"{"old": ["proj_live"], "all": true}"#,
        r#"{"old": ["#,
        "{}",
    ] {
        std::fs::write(&cohort, broken).unwrap();
        assert!(
            matches!(fleet.roster(), Err(FleetError::Cohort(_))),
            "{broken}"
        );
        assert!(
            matches!(fleet.binding("proj_live", STOP), Err(FleetError::Cohort(_))),
            "{broken}"
        );
        assert!(
            fleet
                .binding("proj_live", CommandKind::Restart)
                .unwrap()
                .is_some()
        );
    }
    std::fs::remove_file(&cohort).unwrap();
    std::fs::create_dir(&cohort).unwrap();
    assert!(matches!(fleet.roster(), Err(FleetError::Cohort(_))));

    // No cohort named: everyone, as before there were cohorts.
    let everyone = PgFleet::new(&db.as_role, &sizes, None);
    assert_eq!(
        roster_of(&everyone),
        people(&[
            (
                "user_w1",
                &["proj_legacy", "proj_live", "proj_machine", "proj_relocated"]
            ),
            ("user_w2", &["proj_two"])
        ])
    );
    assert!(everyone.binding("proj_live", STOP).unwrap().is_some());
}

#[test]
fn the_mover_role_reads_only_what_core_role_sql_grants() {
    let db = CoreDb::new();
    let denied = |sql: &str| {
        let as_role = db.as_role.clone();
        let sql = sql.to_string();
        std::thread::spawn(move || {
            block_on(async {
                let (client, connection) = tokio_postgres::connect(&as_role, NoTls).await.unwrap();
                tokio::spawn(async move {
                    let _ = connection.await;
                });
                client
                    .batch_execute(&sql)
                    .await
                    .err()
                    .map(|error| format!("{error:?}"))
            })
        })
        .join()
        .unwrap()
    };
    for sql in [
        "SELECT * FROM users",
        "SELECT runtime_spec, idempotency_key FROM agent_creation_requests",
        "SELECT token_hash FROM runtime_relay_credentials",
        "SELECT * FROM account_brain_principals",
        "UPDATE projects SET display_name = 'x'",
        "INSERT INTO agent_runtime_moves (agent_runtime_id, project_id, moved, moved_at) VALUES ('a', 'b', 'archived', now())",
    ] {
        let error = denied(sql).unwrap_or_else(|| panic!("the role may run {sql}"));
        assert!(error.contains("permission denied"), "{sql}: {error}");
    }
    assert_eq!(
        denied("SELECT id, workos_user_id, normalized_email FROM users"),
        None
    );
    for sql in [
        "SELECT id, health_reporting_npub FROM agent_runtimes",
        "SELECT owner_chat_account_id FROM agent_creation_requests",
        "SELECT user_id, public_key_hex, status FROM account_brain_principals",
    ] {
        assert_eq!(denied(sql), None, "{sql}");
    }
}

/// An Agent an operator created for a never-signed-in person, through
/// Core's own store and lease path: the roster leaves it off until the
/// person signs in with that email as the provisioned WorkOS user, then
/// lists it under their email.
#[test]
fn an_operator_created_agent_is_listed_under_its_owner_once_they_sign_in() {
    use finite_saas_core::store::CoreStore;
    use finite_saas_core::*;

    let db = CoreDb::new();
    let project_id = block_on(async {
        let store = CoreStore::connect(&db.url).await.unwrap();
        store
            .link_verified_user(LinkVerifiedUserInput {
                verified_email: "admin@finite.vip".into(),
                workos_user_id: "user_admin".into(),
                now: None,
            })
            .await
            .unwrap();
        store
            .upsert_runtime_artifact(UpsertRuntimeArtifactInput {
                id: "artifact-mover".into(),
                kind: RuntimeArtifactKind::OciImage,
                reference: format!(
                    "ghcr.io/finitecomputer/agent-runtime:m@sha256:{}",
                    "e".repeat(64)
                ),
                version_label: "m".into(),
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
        let created = store
            .admin_create_agent_for_owner(OperatorCreateAgentInput {
                admin: OperatorIdentity {
                    email: "admin@finite.vip".into(),
                    workos_user_id: "user_admin".into(),
                },
                surface: OperatorCreationSurface::Cli,
                owner_email: "frankie@example.test".into(),
                display_name: "Frankie Bot".into(),
                reason: "box1 migration".into(),
                idempotency_key: "mover-roster".into(),
                hosting_tier: None,
                runtime_resource_class: None,
                target_source_host_id: None,
                // The operator path provisions this WorkOS user and its
                // hosted-chat account before creating the Agent.
                owner_identity: Some(OwnerSignInIdentity {
                    workos_user_id: "user_frankie".into(),
                    chat_account_id: "f".repeat(64),
                }),
                existing_account_confirmed: false,
                cli_os_user: Some("root".into()),
                profile_picture_url: None,
                placement_override: None,
                now: None,
            })
            .await
            .unwrap();
        let capabilities = RuntimeCapabilitiesEnvelope::V1(RuntimeCapabilitiesV1 {
            restart: true,
            recover_known_good_chat: false,
            runtime_upgrade: true,
            stop: true,
            runtime_retirement: false,
            trial_archive: false,
        });
        let lease = store
            .lease_agent_creation_request(LeaseAgentCreationRequestInput {
                runner_id: "runner-lat-a".into(),
                lease_token: "lease-mover".into(),
                lease_seconds: Some(300),
                runner_capacity: Some(RunnerLeaseCapacity {
                    runner_classes: vec![RunnerClass::Kata],
                    runtime_capabilities: Some(capabilities.clone()),
                    ..RunnerLeaseCapacity::default()
                }),
                source_host_id: Some("lat-a".into()),
                now: None,
            })
            .await
            .unwrap()
            .expect("the operator creation leases");
        assert_eq!(lease.request.id, created.request.id);
        store
            .complete_agent_creation_request(CompleteAgentCreationRequestInput {
                request_id: lease.request.id.clone(),
                runner_id: "runner-lat-a".into(),
                lease_token: "lease-mover".into(),
                source_host_id: "lat-a".into(),
                source_machine_id: "machine-frankie".into(),
                runtime_artifact_id: Some("artifact-mover".into()),
                state_schema_version: Some("state-v1".into()),
                provider_runtime_handle: None,
                contact_endpoint: None,
                runtime_capabilities: Some(capabilities),
                display_name: None,
                hostname: None,
                runtime_host: Some("lat-a".into()),
                runtime_status: Some(RuntimeSummaryStatus::Online),
                active_inference_profile: None,
                hermes_available: Some(true),
                published_app_urls: Vec::new(),
                agent_npub: None,
                now: None,
            })
            .await
            .unwrap();
        created.project.id
    });
    let dir = tempfile::tempdir().unwrap();
    let fleet = PgFleet::new(&db.as_role, &dir.path().join("sizes.json"), None);
    let roster = fleet.roster().unwrap();
    assert!(
        roster.people.is_empty(),
        "a pending owner has no WorkOS user"
    );

    block_on(async {
        CoreStore::connect(&db.url)
            .await
            .unwrap()
            .link_verified_user(LinkVerifiedUserInput {
                verified_email: "Frankie@example.test".into(),
                workos_user_id: "user_frankie".into(),
                now: None,
            })
            .await
            .unwrap();
    });
    let roster = fleet.roster().unwrap();
    assert_eq!(roster.people.len(), 1);
    let person = &roster.people[0];
    assert_eq!(person.workos_id, "user_frankie");
    assert_eq!(person.email, "frankie@example.test");
    assert_eq!(person.agents.len(), 1);
    assert_eq!(person.agents[0].old, project_id);
    assert_eq!(person.agents[0].name, "Frankie Bot");
}
