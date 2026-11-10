use super::*;

fn of(path: &str) -> (Class, Why) {
    let parts: Vec<&[u8]> = path.split('/').map(str::as_bytes).collect();
    let verdict = classify(&parts, EntryKind::File, BrainScope::None);
    (verdict.class, verdict.why)
}

fn depth(path: &str) -> usize {
    let parts: Vec<&[u8]> = path.split('/').map(str::as_bytes).collect();
    classify(&parts, EntryKind::File, BrainScope::None).depth
}

/// Every secret path the design lists (FIN-463 §4), and each rule's
/// neighbours.
#[test]
fn every_secret_path_is_secret() {
    let cases = [
        ("agent/identity/identity.json", Why::NostrIdentity),
        ("agent/identity/.identity.json.tmp.42", Why::NostrIdentity),
        ("agent/hermes-home/.env", Why::HermesEnv),
        ("agent/hermes-home/config.yaml", Why::HermesConfig),
        ("agent/hermes-home/config.yaml.bak", Why::HermesConfig),
        ("agent/hermes-home/.config.yaml.tmp", Why::HermesConfig),
        ("agent/agentd/agentd.sqlite3", Why::AgentdLedger),
        ("agent/agentd/agentd.sqlite3-wal", Why::AgentdLedger),
        ("agent/client.sqlite3", Why::ChatClientStore),
        ("agent/client.sqlite3-wal", Why::ChatClientStore),
        ("agent/client.sqlite3.writer-lease", Why::ChatClientStore),
        (
            "agent/hermes-home/google_token.json",
            Why::GoogleCredentials,
        ),
        (
            "agent/hermes-home/google_client_secret.json",
            Why::GoogleCredentials,
        ),
        (
            "agent/hermes-home/google_oauth_pending.json",
            Why::GoogleCredentials,
        ),
        ("workspace/old/google_token.json", Why::GoogleCredentials),
        (
            "agent/hermes-home/gws/credentials.json",
            Why::GoogleWorkspaceCli,
        ),
        ("agent/simplex/identity_chat.db", Why::Simplex),
        ("agent/simplex/address.json", Why::Simplex),
        (
            "agent/hermes-home/pairing/telegram-approved.json",
            Why::Pairing,
        ),
        (
            "agent/hermes-home/pairing/telegram-pending.json",
            Why::Pairing,
        ),
        (
            "agent/hermes-home/platforms/pairing/simplex-approved.json",
            Why::Pairing,
        ),
        ("workspace/app/.env", Why::CredentialName),
        ("workspace/app/.env.local", Why::CredentialName),
        ("workspace/app/prod.env", Why::CredentialName),
        ("workspace/app/.envrc", Why::CredentialName),
        ("agent/hermes-home/auth.json", Why::CredentialName),
        ("workspace/.git-credentials", Why::CredentialName),
        ("workspace/git-credentials", Why::CredentialName),
        ("workspace/.netrc", Why::CredentialName),
        ("workspace/.npmrc", Why::CredentialName),
        ("workspace/.pypirc", Why::CredentialName),
        ("workspace/.pgpass", Why::CredentialName),
        ("workspace/keys/id_rsa", Why::CredentialName),
        ("workspace/keys/id_ed25519", Why::CredentialName),
        ("workspace/keys/id_ecdsa", Why::CredentialName),
        ("workspace/tls/server.pem", Why::CredentialName),
        ("workspace/tls/server.KEY", Why::CredentialName),
        ("workspace/tls/cert.p12", Why::CredentialName),
        ("workspace/tls/cert.pfx", Why::CredentialName),
        ("workspace/app/release.keystore", Why::CredentialName),
        (
            "workspace/gcp/client_secret_1234.apps.json",
            Why::CredentialName,
        ),
        (
            "workspace/gcp/service-account-prod.json",
            Why::CredentialName,
        ),
        ("workspace/gcp/Service_Account.json", Why::CredentialName),
        ("workspace/gcp/serviceaccount.json", Why::CredentialName),
        ("workspace/gcp/credentials.json", Why::CredentialName),
        ("workspace/gcp/token.json", Why::CredentialName),
        ("workspace/k8s/secrets.yaml", Why::CredentialName),
        ("workspace/k8s/secrets.yml", Why::CredentialName),
        ("workspace/k8s/secrets.json", Why::CredentialName),
        ("workspace/some/identity.json", Why::CredentialName),
        ("workspace/.ssh/config", Why::CredentialName),
        ("workspace/.ssh/known_hosts", Why::CredentialName),
        ("workspace/.aws/config", Why::CredentialName),
        ("workspace/.gnupg/pubring.kbx", Why::CredentialName),
        // Secret rules come before the cache rule.
        ("workspace/app/node_modules/pkg/.env", Why::CredentialName),
        ("agent/hermes-home/skills/x/.env", Why::CredentialName),
    ];
    for (path, why) in cases {
        assert_eq!(of(path), (Class::Secret, why), "{path}");
    }
}

#[test]
fn history_is_the_allow_listed_hermes_home() {
    for path in [
        "agent/hermes-home/state.db",
        "agent/hermes-home/state.db-wal",
        "agent/hermes-home/state.db-shm",
        "agent/hermes-home/memory_store.db",
        "agent/hermes-home/memory_store.db-wal",
        "agent/hermes-home/SOUL.md",
        "agent/hermes-home/memories/MEMORY.md",
        "agent/hermes-home/memories/USER.md",
        "agent/hermes-home/skills/notes/SKILL.md",
        "agent/hermes-home/cron/jobs.json",
        "agent/hermes-home/cron/output/job/2026.md",
        "agent/hermes-home/scripts/fetch.py",
        "agent/managed-skills/finite/current/productivity/x/SKILL.md",
    ] {
        assert_eq!(of(path), (Class::History, Why::History), "{path}");
    }
}

#[test]
fn the_runtime_is_everything_else() {
    for path in [
        "agent/hermes-home/state.dbx",
        "agent/hermes-home/state.db.bak",
        "agent/hermes-home/logs/gateway.log",
        "agent/hermes-home/sessions/s.json",
        "agent/hermes-home/gateway.pid",
        "agent/hermes-home/memories",
        "agent/hermes-home/skills",
        "agent/managed-skills/finite/.sync.lock",
        "agent/managed-skills/finite/.skills-sync-x/baseline/SKILL.md",
        "agent/managed-skills/other/current/x.md",
        "agent/exports/agent.zip",
        "agent/fbrain/sync-locks/a",
        "agent/agentd/status.json",
        "agent/allowed-users",
        "migration/old.tar",
        ".finite-durable-state-root-migration.json",
        "workspace",
    ] {
        assert_eq!(of(path), (Class::Runtime, Why::Runtime), "{path}");
    }
    assert_eq!(
        of("agent/hermes-home/config.public.yaml"),
        (Class::Runtime, Why::ReplacedByCopy)
    );
}

#[test]
fn work_is_the_workspace_less_caches() {
    for path in [
        "workspace/notes.md",
        "workspace/photos/2026/a.jpg",
        "workspace/app/build/out.js",
    ] {
        assert_eq!(of(path), (Class::Work, Why::Workspace), "{path}");
    }
    for path in [
        "workspace/app/node_modules/pkg/index.js",
        "workspace/app/.git/HEAD",
        "workspace/py/.venv/bin/python3",
        "workspace/py/__pycache__/a.pyc",
        "workspace/.cache/pip/x",
        "workspace/finitebrain/b1/.finitebrain/encrypted-sync/x",
        "agent/hermes-home/skills/x/node_modules/y.js",
    ] {
        assert_eq!(of(path), (Class::Cache, Why::CacheDir), "{path}");
    }
    // A file named like a cache directory is not a directory.
    assert_eq!(of("workspace/notes/.cache"), (Class::Work, Why::Workspace));
}

#[test]
fn links_brains_and_odd_names_stay_behind() {
    let parts: Vec<&[u8]> = vec![b"workspace", b"link"];
    let link = classify(&parts, EntryKind::Other, BrainScope::None);
    assert_eq!((link.class, link.why), (Class::Runtime, Why::NotAFile));

    let parts: Vec<&[u8]> = vec![b"workspace", b"finitebrain", b"b1", b"page.md"];
    let unowned = classify(
        &parts,
        EntryKind::File,
        BrainScope::Brain {
            depth: 3,
            owned: false,
        },
    );
    assert_eq!(
        (unowned.class, unowned.why, unowned.depth),
        (Class::Runtime, Why::NotOwnedBrain, 3)
    );
    let owned = classify(
        &parts,
        EntryKind::File,
        BrainScope::Brain {
            depth: 3,
            owned: true,
        },
    );
    assert_eq!((owned.class, owned.why), (Class::Work, Why::Workspace));

    for name in [&b"bad\xffname"[..], b"new\nline", b"back\\slash"] {
        let parts: Vec<&[u8]> = vec![b"workspace", name, b"x.md"];
        let verdict = classify(&parts, EntryKind::File, BrainScope::None);
        assert_eq!(
            (verdict.class, verdict.why, verdict.depth),
            (Class::Runtime, Why::UncarriedName, 2)
        );
    }
}

#[test]
fn a_verdict_names_the_directory_that_decided_it() {
    assert_eq!(depth("workspace/app/node_modules/a/b/c.js"), 3);
    assert_eq!(depth("agent/hermes-home/logs/a/b.log"), 3);
    assert_eq!(depth("agent/simplex/a/b.db"), 2);
    assert_eq!(depth("migration/a/b"), 1);
    assert_eq!(depth("workspace/app/.env"), 3);
    assert_eq!(depth("agent/fbrain/x/y"), 2);
}
