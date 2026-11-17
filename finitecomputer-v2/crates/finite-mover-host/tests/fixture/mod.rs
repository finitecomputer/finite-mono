//! A durable tree with every secret path of FIN-463 §4 in it, each
//! holding a marker no staged file holds, beside the history, work, cache
//! and runtime a real agent has.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

pub const ID: &str = "runtime_fixture";
/// The old agent's own key and its person's, as the mover sends them, and
/// a key that is neither.
pub const AGENT_KEY: [u8; 32] = [0x0a; 32];
pub const PERSON_KEY: [u8; 32] = [0x0b; 32];
pub const ELSE_KEY: [u8; 32] = [0x0c; 32];
/// A manifest's kind and owner; `None` is a manifest that isn't JSON.
pub type Manifest = Option<(&'static str, Option<[u8; 32]>)>;
/// The brains, each in `workspace/finitebrain/<folder>/`: its folder, its
/// manifest, and the `owner` its name file says when the mover sends both
/// keys.
pub const BRAINS: &[(&str, Manifest, &str)] = &[
    ("brain-agent", Some(("personal", Some(AGENT_KEY))), "agent"),
    (
        "brain-person",
        Some(("personal", Some(PERSON_KEY))),
        "person",
    ),
    ("brain-org", Some(("organization", None)), "other"),
    ("brain-else", Some(("personal", Some(ELSE_KEY))), "other"),
    ("broken", None, "unknown"),
];
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
    "agent/agentd/status.json",
    "workspace/bots/pairing/telegram-approved.json",
];

/// Work text whose contents hold a credential shape: secret by content,
/// left behind.
pub const SECRET_TEXT: &[(&str, &str)] = &[
    (
        "workspace/notes/keys.md",
        "my github token ghp_abcdefghij0123456789",
    ),
    ("workspace/bot.txt", TELEGRAM_TOKEN),
];

/// History text holding a credential shape: staged with it redacted, as
/// (path, what the file holds, what the stage holds).
pub const REDACTED_TEXT: &[(&str, &str, &str)] = &[
    (
        "agent/hermes-home/memories/MEMORY.md",
        "likes tea\nkey sk-proj4abcdefghijklmnop, keep it safe\n",
        "likes tea\nkey [REDACTED], keep it safe\n",
    ),
    // A whole private key: the block goes, BEGIN through END, and the
    // text after it stays.
    (
        "agent/hermes-home/skills/deploy/SKILL.md",
        "# Deploy\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAAKEYPAYLOAD1\nAAAAKEYPAYLOAD2==\n-----END OPENSSH PRIVATE KEY-----\nThen run deploy.sh\n",
        "# Deploy\n[REDACTED]\nThen run deploy.sh\n",
    ),
    // A key cut short, with no END: everything from BEGIN on goes.
    (
        "agent/hermes-home/memories/half-a-key.md",
        "pasted half a key:\n-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nMIIEKEYPAYLOAD3\nKEYPAYLOAD4\n",
        "pasted half a key:\n[REDACTED]",
    ),
];

/// What every private key's body in `REDACTED_TEXT` holds: never staged.
pub const KEY_PAYLOAD: &str = "KEYPAYLOAD";

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
    ("workspace/finitebrain/brain-agent/page.md", "work"),
    ("workspace/finitebrain/brain-person/page.md", "work"),
    ("workspace/finitebrain/brain-org/page.md", "work"),
    ("workspace/finitebrain/brain-else/page.md", "work"),
    ("workspace/finitebrain/broken/page.md", "work"),
    ("workspace/app/package.json", "work"),
    ("workspace/notes/build/outline.md", "work"),
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
    for (path, text, _) in REDACTED_TEXT {
        write(path, text.as_bytes());
    }
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
    // Brains: the agent's, its person's, a team's, someone else's, and
    // one whose manifest can't be read. Each one's page is in STAGED.
    for (folder, manifest, _) in BRAINS {
        write(
            &format!("workspace/finitebrain/{folder}/.finitebrain/brain-directory.json"),
            manifest_json(folder, *manifest).as_bytes(),
        );
    }
    // Cache and runtime.
    write(
        "workspace/app/node_modules/pkg/index.js",
        b"module.exports = 1;\n",
    );
    write("workspace/app/.git/HEAD", b"ref: refs/heads/main\n");
    // Build output beside its project file; a "build" folder of notes is
    // not (workspace/notes has no project file).
    write("workspace/app/dist/bundle.js", b"var built = 1;\n");
    // Tool state in hidden folders, and earlier exports.
    write(
        "workspace/.config/tool/settings.json",
        b"{\"theme\": \"dark\"}\n",
    );
    write(
        "workspace/finite-export/finite-export-agent.zip",
        b"PK earlier export",
    );
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

/// The name file of the brain in `workspace/finitebrain/<folder>/`.
pub fn name_file(folder: &str) -> String {
    format!("workspace/finitebrain/{folder}.json")
}

/// A key's `npub1…`.
pub fn npub(key: [u8; 32]) -> String {
    let hrp = bech32::Hrp::parse("npub").unwrap();
    bech32::encode::<bech32::Bech32>(hrp, &key).unwrap()
}

/// `.finitebrain/brain-directory.json` as `fbrain open` writes it
/// (finite-brain-core's `BrainDirectoryManifest`), named after its folder.
pub fn manifest_json(folder: &str, manifest: Manifest) -> String {
    let Some((kind, owner)) = manifest else {
        return "not json".to_owned();
    };
    serde_json::json!({
        "version": "1",
        "brain": { "id": folder, "kind": kind, "name": format!("{folder} notes"), "ownerNpub": owner.map(npub) },
        "workingTree": { "path": "." },
        "encryptedSync": { "path": ".finitebrain/encrypted-sync" },
        "portability": { "ownedByAgentRuntime": true },
        "createdAt": "2026-11-01T00:00:00Z",
        "updatedAt": "2026-11-01T00:00:00Z"
    })
    .to_string()
}

fn write_raw_name(directory: &Path, name: &[u8]) {
    use std::os::unix::ffi::OsStrExt;
    fs::write(
        directory.join(std::ffi::OsStr::from_bytes(name)),
        b"odd name\n",
    )
    .unwrap();
}
