//! A durable tree with every secret path of FIN-463 §4 in it, each
//! holding a marker no staged file holds, beside the history, work, cache
//! and runtime a real agent has.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

pub const ID: &str = "runtime_fixture";
pub const OWNED_BRAIN: &str = "brain-owned";
/// Every secret file's contents hold this, and nothing staged does.
pub const MARKER: &str = "SECRET-MARKER";
pub const TELEGRAM_TOKEN: &str = "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw1";
pub const OPENROUTER_KEY: &str = "sk-or-v1-0123456789abcdef0123456789abcdef";
pub const SECRET_HEX: &str = "0101010101010101010101010101010101010101010101010101010101010101";

pub struct Fixture {
    _dir: tempfile::TempDir,
    pub work_root: PathBuf,
    pub tree: PathBuf,
}

/// The secret paths, by the rule that keeps each out.
pub const SECRET_FILES: &[&str] = &[
    "agent/identity/identity.json",
    "agent/hermes-home/.env",
    "agent/hermes-home/config.yaml",
    "agent/agentd/agentd.sqlite3",
    "agent/agentd/agentd.sqlite3-wal",
    "agent/client.sqlite3",
    "agent/client.sqlite3-wal",
    "agent/client.sqlite3.writer-lease",
    "agent/hermes-home/google_token.json",
    "agent/hermes-home/google_client_secret.json",
    "agent/hermes-home/google_oauth_pending.json",
    "agent/hermes-home/gws/credentials.json",
    "agent/simplex/identity_chat.db",
    "agent/simplex/identity_agent.db",
    "agent/hermes-home/pairing/telegram-approved.json",
    "agent/hermes-home/platforms/pairing/simplex-approved.json",
    "agent/hermes-home/auth.json",
    "workspace/app/.env",
    "workspace/app/.env.production",
    "workspace/app/.envrc",
    "workspace/app/prod.env",
    "workspace/.git-credentials",
    "workspace/.netrc",
    "workspace/.npmrc",
    "workspace/.pypirc",
    "workspace/.pgpass",
    "workspace/keys/id_rsa",
    "workspace/keys/id_ed25519",
    "workspace/tls/server.pem",
    "workspace/tls/server.key",
    "workspace/tls/cert.p12",
    "workspace/tls/cert.pfx",
    "workspace/app/release.keystore",
    "workspace/gcp/client_secret_1234.apps.json",
    "workspace/gcp/service-account-prod.json",
    "workspace/gcp/credentials.json",
    "workspace/k8s/secrets.yaml",
    "workspace/.ssh/config",
    "workspace/.aws/credentials",
];

/// Text whose contents hold a credential shape: secret by content.
pub const SECRET_TEXT: &[(&str, &str)] = &[
    (
        "workspace/notes/keys.md",
        "my github token ghp_abcdefghij0123456789",
    ),
    (
        "agent/hermes-home/memories/MEMORY.md",
        "key sk-proj4abcdefghijklmnop",
    ),
    (
        "agent/hermes-home/skills/deploy/SKILL.md",
        "-----BEGIN OPENSSH PRIVATE KEY-----",
    ),
    ("workspace/bot.txt", TELEGRAM_TOKEN),
];

/// What is staged, with its class.
pub const STAGED: &[(&str, &str)] = &[
    ("agent/hermes-home/state.db", "history"),
    ("agent/hermes-home/state.db-wal", "history"),
    ("agent/hermes-home/memory_store.db", "history"),
    ("agent/hermes-home/SOUL.md", "history"),
    ("agent/hermes-home/memories/USER.md", "history"),
    ("agent/hermes-home/skills/notes/SKILL.md", "history"),
    ("agent/hermes-home/cron/jobs.json", "history"),
    ("agent/hermes-home/scripts/fetch.py", "history"),
    (
        "agent/managed-skills/finite/current/productivity/x/SKILL.md",
        "history",
    ),
    ("workspace/notes/today.md", "work"),
    ("workspace/notes/placeholder.md", "work"),
    ("workspace/photos/cat.jpg", "work"),
    ("workspace/finitebrain/brain-owned/page.md", "work"),
];

pub fn build() -> Fixture {
    let dir = tempfile::Builder::new()
        .prefix("mover-host-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let work_root = dir.path().join("work");
    let tree = work_root.join("kata").join(ID);
    let write = |path: &str, contents: &[u8]| {
        let path = tree.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    };
    for (index, path) in SECRET_FILES.iter().enumerate() {
        write(path, format!("{MARKER}-{index}\n").as_bytes());
    }
    write(
        "agent/identity/identity.json",
        format!(
            r#"{{"version":1,"kind":"nostr-secp256k1","secret_hex":"{SECRET_HEX}","public_key_hex":"x","created_at":"t","created_by":"{MARKER}"}}"#
        )
        .as_bytes(),
    );
    write(
        "agent/hermes-home/.env",
        format!("# {MARKER}\nOPENROUTER_API_KEY='{OPENROUTER_KEY}'\n").as_bytes(),
    );
    write(
        "agent/hermes-home/config.yaml",
        format!(
            "timezone: America/Chicago\n\
             agent:\n  personality: kind and brief\n\
             model:\n  default: glm-5-3-flash\n  provider: custom\n  api_key: {MARKER}-model\n\
             gateway:\n  platforms:\n    telegram:\n      enabled: true\n      token: '{TELEGRAM_TOKEN}'\n\
             mcp_servers:\n  x:\n    headers:\n      Authorization: Bearer {MARKER}-header\n"
        )
        .as_bytes(),
    );
    for (index, (path, text)) in SECRET_TEXT.iter().enumerate() {
        write(path, format!("{text}\n{MARKER}-text-{index}\n").as_bytes());
    }
    let mut database = b"SQLite format 3\0".to_vec();
    database.extend(std::iter::repeat_n(7u8, 40_000));
    for (path, _) in STAGED {
        let contents = if path.ends_with(".db") || path.ends_with(".jpg") {
            database.clone()
        } else {
            format!("{path}\nsk-xxxxxxxxxxxxxxxxxxxxxxxx is a placeholder\n").into_bytes()
        };
        write(path, &contents);
    }
    // Brains: owned, someone else's, and one whose manifest can't be read.
    for (brain, id) in [
        ("brain-owned", Some(OWNED_BRAIN)),
        ("brain-org", Some("brain-org")),
        ("broken", None),
    ] {
        let manifest = id.map_or_else(
            || "not json".to_owned(),
            |id| format!(r#"{{"version":"1","brain":{{"id":"{id}","kind":"personal","name":"n","owner_npub":null}}}}"#),
        );
        write(
            &format!("workspace/finitebrain/{brain}/.finitebrain/brain-directory.json"),
            manifest.as_bytes(),
        );
        if brain != "brain-owned" {
            write(
                &format!("workspace/finitebrain/{brain}/page.md"),
                b"not this person's brain\n",
            );
        }
    }
    // Cache and runtime.
    write(
        "workspace/app/node_modules/pkg/index.js",
        b"module.exports = 1;\n",
    );
    write("workspace/app/.git/HEAD", b"ref: refs/heads/main\n");
    write("agent/hermes-home/logs/gateway.log", b"log line\n");
    write(
        "agent/hermes-home/config.public.yaml",
        b"timezone: Not/This\n",
    );
    write("agent/exports/agent.zip", b"PK old export");
    write("migration/legacy.txt", b"earlier migration\n");
    // Links are never followed: one to a host file, one into another
    // agent's tree.
    let other = work_root.join("kata").join("other_agent");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("theirs.md"), format!("{MARKER}-other-agent\n")).unwrap();
    symlink("/etc/passwd", tree.join("workspace/passwd")).unwrap();
    symlink(&other, tree.join("workspace/other")).unwrap();
    symlink(
        other.join("theirs.md"),
        tree.join("agent/hermes-home/SOUL-link.md"),
    )
    .unwrap();
    rustix::fs::mknodat(
        rustix::fs::CWD,
        tree.join("workspace/pipe"),
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )
    .unwrap();
    write_raw_name(&tree.join("workspace"), b"bad\xffname.txt");
    // A hard link is not a symbolic link: O_NOFOLLOW opens it. It names a
    // file outside the tree, so it must never be read.
    std::fs::write(
        work_root.join("outside.txt"),
        format!("{MARKER}-hard-link\n"),
    )
    .unwrap();
    std::fs::hard_link(
        work_root.join("outside.txt"),
        tree.join("workspace/report.txt"),
    )
    .unwrap();
    Fixture {
        _dir: dir,
        work_root,
        tree,
    }
}

fn write_raw_name(directory: &Path, name: &[u8]) {
    use std::os::unix::ffi::OsStrExt;
    fs::write(
        directory.join(std::ffi::OsStr::from_bytes(name)),
        b"odd name\n",
    )
    .unwrap();
}
