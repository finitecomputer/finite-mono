//! Local, read-only identity projection for Brain display metadata.
//! No secrets, messages, encrypted state, invite tokens, or grants are queried.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use finite_brain_server::principal_labels::{
    LabelBinding, LabelProjection, LabelSource, MAX_LABELS, MAX_PROJECTION_BYTES, PrincipalKind,
    PrincipalLabel,
};
use finite_nostr::NostrPublicKey;
use rusqlite::{Connection, OpenFlags};
#[cfg(test)]
use sha2::{Digest, Sha256};
use tokio_postgres::NoTls;

#[derive(Debug)]
enum RefreshError {
    Configuration,
    SourceUnavailable,
    InvalidSource,
    Limit,
    Output,
}

fn readonly_sqlite(path: &Path) -> Result<Connection, RefreshError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| RefreshError::SourceUnavailable)?;
    conn.busy_timeout(Duration::from_millis(250))
        .map_err(|_| RefreshError::SourceUnavailable)?;
    conn.execute_batch("PRAGMA query_only=ON; BEGIN;")
        .map_err(|_| RefreshError::SourceUnavailable)?;
    Ok(conn)
}

fn brain_principals(path: &Path) -> Result<BTreeSet<String>, RefreshError> {
    let conn = readonly_sqlite(path)?;
    // These are existing Brain principals, not an account-wide directory dump.
    let mut statement = conn
        .prepare(
            "SELECT user_id FROM brain_members
        UNION SELECT user_id FROM brain_admins
        UNION SELECT user_id FROM folder_access
        UNION SELECT owner_user_id FROM brains WHERE owner_user_id IS NOT NULL
        UNION SELECT agent_npub FROM personal_agents LIMIT ?1",
        )
        .map_err(|_| RefreshError::InvalidSource)?;
    let rows = statement
        .query_map([MAX_LABELS as u64 + 1], |row| row.get::<_, String>(0))
        .map_err(|_| RefreshError::InvalidSource)?;
    let principals = rows
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|_| RefreshError::InvalidSource)?;
    if principals.len() > MAX_LABELS {
        return Err(RefreshError::Limit);
    }
    Ok(principals)
}

fn hosted_public_key(root: &Path, storage_id: &str) -> Result<Option<String>, RefreshError> {
    // The hosted device owns this WorkOS-subject namespace (user_storage_id).
    // A bare creation-request owner key is caller input and is never a source.
    let database = root
        .join("users")
        .join(storage_id)
        .join("chat/client.sqlite3");
    if !database
        .try_exists()
        .map_err(|_| RefreshError::SourceUnavailable)?
    {
        return Ok(None);
    }
    let conn = readonly_sqlite(&database)?;
    let mut statement = conn.prepare("SELECT DISTINCT account_id FROM client_device_states WHERE device_id='hosted-web' LIMIT 2")
        .map_err(|_| RefreshError::InvalidSource)?;
    let keys = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| RefreshError::InvalidSource)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| RefreshError::InvalidSource)?;
    if keys.len() != 1 {
        return Ok(None);
    }
    Ok(NostrPublicKey::parse(&keys[0])
        .ok()
        .and_then(|key| key.to_npub().ok()))
}

fn hosted_candidates(
    root: &Path,
    principals: &BTreeSet<String>,
    started: Instant,
) -> Result<BTreeMap<String, String>, RefreshError> {
    let users = match fs::read_dir(root.join("users")) {
        Ok(users) => users,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(_) => return Err(RefreshError::SourceUnavailable),
    };
    let mut candidates = BTreeMap::new();
    // Stream existing hosted namespaces, not every enrolled Core account.
    // Time bounds this scan; the output limit counts only matching principals.
    for user in users {
        if started.elapsed() > Duration::from_secs(25) {
            return Err(RefreshError::Limit);
        }
        let user = user.map_err(|_| RefreshError::SourceUnavailable)?;
        let storage_id = user.file_name().to_string_lossy().into_owned();
        if storage_id.len() != 64
            || !storage_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            continue;
        }
        if let Some(npub) = hosted_public_key(root, &storage_id)? {
            if principals.contains(&npub) {
                candidates.insert(storage_id, npub);
            }
        }
        if candidates.len() > MAX_LABELS {
            return Err(RefreshError::Limit);
        }
    }
    Ok(candidates)
}

fn binding(
    npub: String,
    source_id: String,
    name: String,
    kind: PrincipalKind,
    now: u64,
) -> Option<LabelBinding> {
    // Preserve even unusable names until conflict resolution: dropping one
    // owner here could make a different owner of the same key look unique.
    let canonical = NostrPublicKey::parse(&npub).ok()?.to_npub().ok()?;
    if canonical != npub {
        return None;
    }
    Some(LabelBinding {
        npub,
        source_id,
        label: PrincipalLabel {
            name,
            kind,
            observed_at: now,
            source: match kind {
                PrincipalKind::Human => LabelSource::HostedAccount,
                PrincipalKind::Agent => LabelSource::ManagedAgent,
            },
        },
    })
}

async fn collect(
    database: &tokio_postgres::Config,
    brain_db: &Path,
    hosted_root: &Path,
    now: u64,
) -> Result<LabelProjection, RefreshError> {
    let started = Instant::now();
    let principals = brain_principals(brain_db)?;
    let candidates = hosted_candidates(hosted_root, &principals, started)?;
    let (mut client, connection) = database
        .connect(NoTls)
        .await
        .map_err(|_| RefreshError::SourceUnavailable)?;
    let connection_task = tokio::spawn(async move {
        let _ = connection.await;
    });
    let transaction = client
        .build_transaction()
        .read_only(true)
        .start()
        .await
        .map_err(|_| RefreshError::SourceUnavailable)?;
    transaction
        .batch_execute("SET LOCAL statement_timeout = '5s'; SET LOCAL lock_timeout = '250ms';")
        .await
        .map_err(|_| RefreshError::SourceUnavailable)?;
    let targets: Vec<_> = principals.iter().cloned().collect();
    let limit = MAX_LABELS as i64 + 1;
    let agents = transaction
        .query(
            "SELECT DISTINCT ar.health_reporting_npub, p.id, p.display_name
        FROM agent_runtimes ar JOIN projects p ON p.id=ar.project_id
        WHERE ar.health_reporting_npub = ANY($1) LIMIT $2",
            &[&targets, &limit],
        )
        .await
        .map_err(|_| RefreshError::SourceUnavailable)?;
    let subject_hashes: Vec<_> = candidates.keys().cloned().collect();
    let users = transaction
        .query(
            "SELECT workos_user_id, normalized_email,
            encode(sha256(convert_to(workos_user_id, 'UTF8')), 'hex') AS subject_hash
        FROM users WHERE link_status='linked' AND workos_user_id IS NOT NULL
            AND encode(sha256(convert_to(workos_user_id, 'UTF8')), 'hex') = ANY($1) LIMIT $2",
            &[&subject_hashes, &limit],
        )
        .await
        .map_err(|_| RefreshError::SourceUnavailable)?;
    transaction
        .rollback()
        .await
        .map_err(|_| RefreshError::SourceUnavailable)?;
    drop(client);
    connection_task.abort();
    if agents.len() > MAX_LABELS || users.len() > MAX_LABELS {
        return Err(RefreshError::Limit);
    }
    let mut bindings = Vec::new();
    for row in agents {
        if let Some(label) = binding(
            row.get(0),
            row.get(1),
            row.get(2),
            PrincipalKind::Agent,
            now,
        ) {
            bindings.push(label);
        }
    }
    for row in users {
        if started.elapsed() > Duration::from_secs(25) {
            return Err(RefreshError::Limit);
        }
        let subject: String = row.get(0);
        let subject_hash: String = row.get(2);
        if let Some(npub) = candidates.get(&subject_hash) {
            if let Some(label) =
                binding(npub.clone(), subject, row.get(1), PrincipalKind::Human, now)
            {
                bindings.push(label);
            }
        }
    }
    let projection = LabelProjection {
        version: 1,
        generated_at: now,
        bindings,
    };
    if projection.labels(now).is_none() {
        return Err(RefreshError::InvalidSource);
    }
    Ok(projection)
}

fn publish(path: &Path, projection: &LabelProjection) -> Result<(), RefreshError> {
    let bytes = serde_json::to_vec(projection).map_err(|_| RefreshError::Output)?;
    if bytes.len() as u64 > MAX_PROJECTION_BYTES {
        return Err(RefreshError::Limit);
    }
    let parent = path.parent().ok_or(RefreshError::Configuration)?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| RefreshError::Output)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o640))
            .map_err(|_| RefreshError::Output)?;
    }
    temporary
        .write_all(&bytes)
        .map_err(|_| RefreshError::Output)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| RefreshError::Output)?;
    temporary.persist(path).map_err(|_| RefreshError::Output)?;
    Ok(())
}

async fn refresh() -> Result<(), RefreshError> {
    let required = |name| std::env::var(name).map_err(|_| RefreshError::Configuration);
    let url = required("FINITE_BRAIN_LABEL_DATABASE_URL")?;
    let brain_db = PathBuf::from(required("FINITE_BRAIN_DB")?);
    let hosted_root = PathBuf::from(required("FINITECHAT_HOSTED_DATA_ROOT")?);
    let output = PathBuf::from(required("FINITE_BRAIN_PRINCIPAL_LABELS")?);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RefreshError::Configuration)?
        .as_secs();
    let database = url
        .parse::<tokio_postgres::Config>()
        .map_err(|_| RefreshError::Configuration)?;
    let projection = collect(&database, &brain_db, &hosted_root, now).await?;
    publish(&output, &projection)?;
    println!(
        "Brain label projection refreshed: {} unambiguous identities",
        projection
            .labels(now)
            .ok_or(RefreshError::InvalidSource)?
            .len()
    );
    Ok(())
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match tokio::time::timeout(Duration::from_secs(30), refresh()).await {
        Ok(Ok(())) => std::process::ExitCode::SUCCESS,
        result => {
            // Keep connection URLs, account names, subjects and keys out of logs.
            eprintln!(
                "Brain label refresh failed: {:?}",
                result.map(|result| result.err())
            );
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject_hash(subject: &str) -> String {
        format!("{:x}", Sha256::digest(subject.as_bytes()))
    }

    fn principal(byte: u8) -> (String, String) {
        let hex = format!("{byte:02x}").repeat(32);
        let npub = NostrPublicKey::parse(&hex).unwrap().to_npub().unwrap();
        (hex, npub)
    }

    fn hosted_fixture(root: &Path, subject: &str, hex: &str) -> PathBuf {
        let path = root
            .join("users")
            .join(format!("{:x}", Sha256::digest(subject.as_bytes())))
            .join("chat/client.sqlite3");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(&path).unwrap();
        // Retained hosted Chat public columns; no encrypted state is needed or read.
        conn.execute_batch("CREATE TABLE client_device_states (account_id TEXT, device_id TEXT, nonce BLOB, ciphertext BLOB);").unwrap();
        conn.execute(
            "INSERT INTO client_device_states VALUES (?1, 'hosted-web', X'00', X'01')",
            [hex],
        )
        .unwrap();
        path
    }

    #[test]
    fn hosted_identity_reads_fail_closed_without_creating_or_modifying_stores() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            hosted_public_key(dir.path(), &subject_hash("absent"))
                .unwrap()
                .is_none()
        );
        assert!(!dir.path().join("users").exists());
        let (hex, npub) = principal(1);
        let path = hosted_fixture(dir.path(), "verified-subject", &hex);
        let before = fs::read(&path).unwrap();
        assert_eq!(
            hosted_public_key(dir.path(), &subject_hash("verified-subject")).unwrap(),
            Some(npub)
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        let readonly = readonly_sqlite(&path).unwrap();
        assert!(
            readonly
                .execute("DELETE FROM client_device_states", [])
                .is_err()
        );
        drop(readonly);
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO client_device_states VALUES (?1, 'hosted-web', X'00', X'01')",
            [principal(2).0],
        )
        .unwrap();
        assert!(
            hosted_public_key(dir.path(), &subject_hash("verified-subject"))
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn projection_joins_verified_sources_and_refreshes_new_members_without_source_writes() {
        let url = std::env::var("FC_CORE_POSTGRES_TEST_URL")
            .expect("run with devfinity: FC_CORE_POSTGRES_TEST_URL is required");
        let (admin, connection) = tokio_postgres::connect(&url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let schema = format!("brain_labels_{suffix}");
        admin
            .batch_execute(&format!(
                "CREATE SCHEMA {schema}; SET search_path TO {schema};
            CREATE TABLE users (workos_user_id TEXT, normalized_email TEXT, link_status TEXT);
            CREATE TABLE projects (id TEXT, display_name TEXT);
            CREATE TABLE agent_runtimes (project_id TEXT, health_reporting_npub TEXT);"
            ))
            .await
            .unwrap();
        let (human_hex, human) = principal(1);
        let (_, agent) = principal(2);
        let (future_hex, future) = principal(3);
        admin.execute("INSERT INTO users VALUES ('verified-human', 'alex@example.com', 'linked'), ('future-human', 'future@example.com', 'linked'), ('unverified', 'wrong@example.com', 'pending')", &[]).await.unwrap();
        admin
            .execute(
                "INSERT INTO projects VALUES ('agent-project', 'alex@example.com')",
                &[],
            )
            .await
            .unwrap();
        admin
            .execute(
                "INSERT INTO agent_runtimes VALUES ('agent-project', $1)",
                &[&agent],
            )
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let brain = dir.path().join("brain.sqlite3");
        let conn = Connection::open(&brain).unwrap();
        conn.execute_batch(
            "CREATE TABLE brain_members (user_id TEXT); CREATE TABLE brain_admins (user_id TEXT);
            CREATE TABLE folder_access (user_id TEXT); CREATE TABLE brains (owner_user_id TEXT);
            CREATE TABLE personal_agents (agent_npub TEXT);",
        )
        .unwrap();
        for npub in [&human, &agent] {
            conn.execute("INSERT INTO brain_members VALUES (?1)", [npub])
                .unwrap();
        }
        let hosted = dir.path().join("hosted");
        hosted_fixture(&hosted, "verified-human", &human_hex);
        hosted_fixture(&hosted, "future-human", &future_hex);
        hosted_fixture(&hosted, "unverified", &human_hex);
        admin.batch_execute("INSERT INTO users SELECT 'unrelated-' || id, 'unrelated@example.com', 'linked' FROM generate_series(1, 5000) id").await.unwrap();
        let before = fs::read(&brain).unwrap();
        let mut config: tokio_postgres::Config = url.parse().unwrap();
        config.options(&format!("-c search_path={schema}"));
        let projection = collect(&config, &brain, &hosted, 100).await.unwrap();
        let labels = projection.labels(100).unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[&human].display(), "alex@example.com (human)");
        assert_eq!(labels[&agent].display(), "alex@example.com (agent)");
        assert_eq!(fs::read(&brain).unwrap(), before);
        let output = dir.path().join("labels.json");
        publish(&output, &projection).unwrap();
        let first = fs::read(&output).unwrap();
        conn.execute("INSERT INTO brain_members VALUES (?1)", [&future])
            .unwrap();
        let refreshed = collect(&config, &brain, &hosted, 160).await.unwrap();
        assert_eq!(
            refreshed.labels(160).unwrap()[&future].name,
            "future@example.com"
        );
        publish(&output, &refreshed).unwrap();
        assert_ne!(fs::read(&output).unwrap(), first);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&output).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
        // Two verified accounts claiming the same key remain ambiguous even if
        // their display names agree. No first/last row wins.
        admin
            .execute(
                "INSERT INTO users VALUES ('other-human', 'alex@example.com', 'linked')",
                &[],
            )
            .await
            .unwrap();
        hosted_fixture(&hosted, "other-human", &human_hex);
        let ambiguous = collect(&config, &brain, &hosted, 220).await.unwrap();
        assert!(!ambiguous.labels(220).unwrap().contains_key(&human));
        admin.execute("UPDATE users SET normalized_email=E'bad\\nname' WHERE workos_user_id='other-human'", &[]).await.unwrap();
        let invalid_owner = collect(&config, &brain, &hosted, 240).await.unwrap();
        let labels = invalid_owner.labels(240).unwrap();
        assert!(!labels.contains_key(&human));
        assert!(labels.contains_key(&agent));
        assert_eq!(
            admin
                .query_one("SELECT count(*) FROM agent_runtimes", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            1
        );
        admin
            .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
            .await
            .unwrap();
        connection.abort();
    }
}
