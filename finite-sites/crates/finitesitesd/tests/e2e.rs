//! End-to-end test: a real finitesitesd server on an ephemeral port, driven
//! over HTTP exactly the way `fsite` and a browser would drive it —
//! NIP-98-signed API calls, Host-routed site requests, gate-vouch viewing.

// Test helpers return ureq's own error so assertions can match on exact
// HTTP statuses; its size does not matter in a test binary.
#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use finitesites_blob::BlobStore;
use finitesites_engine::{Engine, EngineConfig};
use finitesites_proto::dto::{
    ApiErrorBody, AuthRegisterResponse, EmailLoginRequest, EmailLoginResponse, EmailRedeemRequest,
    EmailRedeemResponse, GitAuthRequest, GitAuthResponse, HostedRequesterAssertionRequest,
    HostedRequesterAssertionResponse, ProjectGrantRequest, ProjectGrantResponse,
    ProjectInitRequest, ProjectInitResponse, ProjectOutputSharingResponse, ProjectOutputSummary,
    ProjectRevokeRequest, ProjectRevokeResponse, ProjectStatusResponse, SharingRequest,
    SitesAuthorizedKeyRegisterRequest, SitesAuthorizedKeyResponse, SitesAuthorizedKeyRevokeRequest,
};
use finitesites_proto::nip98;
use finitesites_proto::project_config::{
    ProjectConfig, ProjectOutputConfig, ProjectOutputKind, ProjectSection,
};
use finitesites_store::{ProjectVisibility, SiteStatus, Store};
use finitesitesd::mailer::DevMailer;
use finitesitesd::{ServeOptions, server};

const BASE_DOMAIN: &str = "sites.localhost";
const VIEWER_SESSION_SERVICE_TOKEN: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// The Auth Gate's location, as finitesitesd would learn it from
/// FINITE_SITES_AUTH_GATE_URL. finitesitesd only ever redirects browsers
/// here; it never calls the gate at runtime, so the URL does not need to
/// resolve inside the test.
const AUTH_GATE_URL: &str = "http://auth.sites.localhost:8792";
/// The gate's vouch-signing secret for tests. Only its PUBLIC key is handed
/// to finitesitesd, exactly like production.
const GATE_SIGNING_KEY: [u8; 32] = [0x2a; 32];

fn document_base_domain() -> String {
    format!("docs.{BASE_DOMAIN}")
}

fn user_secret() -> [u8; 32] {
    let mut secret = [0u8; 32];
    secret[31] = 11;
    secret
}

fn stranger_secret() -> [u8; 32] {
    let mut secret = [0u8; 32];
    secret[31] = 33;
    secret
}

fn viewer_secret() -> [u8; 32] {
    let mut secret = [0u8; 32];
    secret[31] = 22;
    secret
}

fn now_unix() -> u64 {
    time::OffsetDateTime::now_utc().unix_timestamp() as u64
}

/// ureq agent that resolves every hostname to the test server. This is what
/// wildcard DNS does in production.
fn agent_for(addr: SocketAddr) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .redirects(0)
        .resolver(move |netloc: &str| {
            let port = netloc
                .rsplit_once(':')
                .and_then(|(_, p)| p.parse::<u16>().ok())
                .unwrap_or(80);
            Ok(vec![SocketAddr::new(addr.ip(), port)])
        })
        .build()
}

struct TestServer {
    agent: ureq::Agent,
    api_url: String,
    outbox: std::path::PathBuf,
    data_dir: tempfile::TempDir,
}

impl TestServer {
    async fn start(allowed_pubkey: &str) -> TestServer {
        Self::start_with_git_auto_reconcile(allowed_pubkey, true).await
    }

    async fn start_without_publish_grant(git_auto_reconcile: bool) -> TestServer {
        Self::start_inner(
            None,
            git_auto_reconcile,
            false,
            Some(VIEWER_SESSION_SERVICE_TOKEN),
        )
        .await
    }

    async fn start_with_git_auto_reconcile(
        allowed_pubkey: &str,
        git_auto_reconcile: bool,
    ) -> TestServer {
        Self::start_inner(
            Some(allowed_pubkey),
            git_auto_reconcile,
            false,
            Some(VIEWER_SESSION_SERVICE_TOKEN),
        )
        .await
    }

    async fn start_single_origin(allowed_pubkey: &str) -> TestServer {
        Self::start_inner(
            Some(allowed_pubkey),
            true,
            true,
            Some(VIEWER_SESSION_SERVICE_TOKEN),
        )
        .await
    }

    async fn start_inner(
        allowed_pubkey: Option<&str>,
        git_auto_reconcile: bool,
        single_origin_git: bool,
        viewer_session_service_token: Option<&str>,
    ) -> TestServer {
        let data_dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&data_dir.path().join("registry.db")).unwrap();
        if let Some(allowed_pubkey) = allowed_pubkey {
            store
                .allow_pubkey(allowed_pubkey, "e2e", now_unix())
                .unwrap();
            store
                .register_sites_authorized_key("owner@example.com", allowed_pubkey, now_unix())
                .unwrap();
        }
        let blobs = BlobStore::open(&data_dir.path().join("blobs")).unwrap();
        let outbox = data_dir.path().join("outbox");
        let mailer = DevMailer::new(outbox.clone()).unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let engine = Engine::new(
            store,
            blobs,
            [9u8; 32],
            EngineConfig {
                base_domain: BASE_DOMAIN.to_string(),
                document_base_domain: document_base_domain(),
                site_url_scheme: "http".to_string(),
                site_url_port: Some(addr.port()),
            },
        );
        let api_url = format!("http://127.0.0.1:{}", addr.port());
        let git_base_url = if single_origin_git {
            api_url.clone()
        } else {
            format!("http://git.{BASE_DOMAIN}:{}", addr.port())
        };
        let options = ServeOptions {
            data_dir: data_dir.path().to_path_buf(),
            listen: addr,
            base_domain: BASE_DOMAIN.to_string(),
            document_base_domain: document_base_domain(),
            api_url,
            git_base_url,
            viewer_session_service_token: viewer_session_service_token.map(str::to_string),
            auth_gate_url: Some(AUTH_GATE_URL.to_string()),
            auth_gate_pubkey: Some(
                finite_authn::gate_pubkey_for_secret(&GATE_SIGNING_KEY).unwrap(),
            ),
            git_hook_helper_path: hook_helper_path(),
            git_auto_reconcile,
            site_url_scheme: "http".to_string(),
            site_url_port: Some(addr.port()),
            mail_provider: None,
            mail_from: None,
            app_runner_kind: finitesitesd::AppRunnerKind::Disabled,
            app_sudo_path: PathBuf::from("sudo"),
            app_nerdctl_path: PathBuf::from("nerdctl"),
            app_cni_path: PathBuf::from("/opt/cni/bin"),
            idle_timeout_seconds: 900,
        };
        let api_url = options.api_url.clone();
        tokio::spawn(async move {
            server::serve_on(
                listener,
                engine,
                Box::new(mailer),
                finitesitesd::apps::Supervisor::new(
                    Box::new(finitesitesd::apps::DisabledRunner),
                    900,
                ),
                options,
            )
            .await
            .expect("test server runs");
        });

        TestServer {
            agent: agent_for(addr),
            api_url,
            outbox,
            data_dir,
        }
    }

    fn data_dir(&self) -> &Path {
        self.data_dir.path()
    }

    fn signed(
        &self,
        secret: &[u8; 32],
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<ureq::Response, ureq::Error> {
        let url = format!("{}{path}", self.api_url);
        let header = nip98::build_auth_header(secret, &url, method, body, now_unix()).unwrap();
        let request = self
            .agent
            .request(method, &url)
            .set("Authorization", &header);
        match body {
            Some(bytes) => request.send_bytes(bytes),
            None => request.call(),
        }
    }

    fn site_get(&self, name: &str, path: &str, port: u16) -> Result<ureq::Response, ureq::Error> {
        self.agent
            .get(&format!("http://{name}.{BASE_DOMAIN}:{port}{path}"))
            .call()
    }

    /// Mint a gate vouch for one email on one output origin, the way the real
    /// gate does after a WorkOS (or dev-mode) authentication. Each call uses
    /// a fresh nonce so vouches are single-use.
    fn gate_vouch(&self, origin: &str, email: &str) -> String {
        let nonce = finitesites_proto::ids::random_32();
        finite_authn::mint_vouch(
            &GATE_SIGNING_KEY,
            origin,
            email,
            now_unix(),
            &finite_authn::AuthPolicy::default(),
            nonce,
        )
        .unwrap()
    }

    /// Redeem a gate vouch at the output origin's `/_finite/auth` consumer,
    /// the way the gate redirects a browser back.
    fn redeem_gate_code(
        &self,
        origin: &str,
        code: &str,
        return_to: &str,
    ) -> Result<ureq::Response, ureq::Error> {
        self.agent
            .get(&format!(
                "{origin}/_finite/auth?gate_code={}&return_to={}",
                urlencode(code),
                urlencode(return_to)
            ))
            .call()
    }

    fn hosted_requester_assertion(
        &self,
        token: Option<&str>,
        request: &HostedRequesterAssertionRequest,
    ) -> Result<ureq::Response, ureq::Error> {
        let body = serde_json::to_vec(request).unwrap();
        let mut call = self
            .agent
            .post(&format!(
                "{}/internal/v1/hosted-requester-assertions",
                self.api_url
            ))
            .set("Content-Type", "application/json");
        if let Some(token) = token {
            call = call.set("Authorization", &format!("Bearer {token}"));
        }
        call.send_bytes(&body)
    }

    fn document_get(
        &self,
        name: &str,
        path: &str,
        port: u16,
    ) -> Result<ureq::Response, ureq::Error> {
        self.agent
            .get(&format!(
                "http://{name}.{}:{port}{path}",
                document_base_domain()
            ))
            .call()
    }

    fn port(&self) -> u16 {
        self.api_url.rsplit_once(':').unwrap().1.parse().unwrap()
    }
}

fn hook_helper_path() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_finitesitesd") {
        return PathBuf::from(path);
    }
    let current = std::env::current_exe().unwrap();
    let debug_dir = current
        .parent()
        .and_then(Path::parent)
        .expect("test binary lives under target/debug/deps");
    let name = if cfg!(windows) {
        "finitesitesd.exe"
    } else {
        "finitesitesd"
    };
    let candidate = debug_dir.join(name);
    assert!(
        candidate.exists(),
        "finitesitesd hook helper binary missing at {}",
        candidate.display()
    );
    candidate
}

fn json_body<T: serde::de::DeserializeOwned>(response: ureq::Response) -> T {
    response.into_json().unwrap()
}

fn urlencode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn outbox_link(outbox: &Path) -> String {
    let entries: Vec<_> = std::fs::read_dir(outbox).unwrap().collect();
    assert_eq!(entries.len(), 1, "expected exactly one dev mail");
    let path = entries[0].as_ref().unwrap().path();
    let content = std::fs::read_to_string(path).unwrap();
    content
        .lines()
        .find(|line| line.starts_with("http"))
        .expect("mail contains a link")
        .to_string()
}

fn outbox_email_token(outbox: &Path) -> String {
    let entries: Vec<_> = std::fs::read_dir(outbox).unwrap().collect();
    assert_eq!(entries.len(), 1, "expected exactly one dev mail");
    let path = entries[0].as_ref().unwrap().path();
    let content = std::fs::read_to_string(path).unwrap();
    content
        .lines()
        .find_map(|line| line.trim().strip_prefix("fsite auth redeem "))
        .and_then(|rest| rest.split_whitespace().nth(1))
        .expect("mail contains an auth redeem command")
        .to_string()
}

fn outbox_bodies(outbox: &Path) -> Vec<String> {
    let mut paths: Vec<_> = std::fs::read_dir(outbox)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect()
}

fn clear_outbox(outbox: &Path) {
    for entry in std::fs::read_dir(outbox).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
}

/// Build a git Command isolated from the developer's environment: no system
/// or global config and no credential helpers, so a real credential stored on
/// the host (keychain, `fsite auth git --store`) can never authenticate a
/// request the test expects to be anonymous. Tests pass identity and
/// credentials explicitly (`-c user.*`, credentials embedded in remote URLs).
fn git_command(args: &[&str], cwd: Option<&Path>) -> Command {
    let mut command = Command::new("git");
    command.args(args);
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    command.env("GIT_CONFIG_GLOBAL", "/dev/null");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command
}

fn run_git(args: &[&str], cwd: Option<&Path>) {
    let mut command = git_command(args, cwd);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_git_capture(args: &[&str], cwd: Option<&Path>) -> std::process::Output {
    let mut command = git_command(args, cwd);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn run_git_expect_failure(args: &[&str], cwd: Option<&Path>) {
    let mut command = git_command(args, cwd);
    let output = command.output().unwrap();
    assert!(
        !output.status.success(),
        "git {:?} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn try_git(args: &[&str], cwd: Option<&Path>) -> bool {
    let mut command = git_command(args, cwd);
    command.stdout(Stdio::null()).stderr(Stdio::null());
    command.status().unwrap().success()
}

fn wait_for_active_version(
    server: &TestServer,
    name: &str,
    expected: Option<u32>,
) -> ProjectOutputSummary {
    wait_for_project_active_version(server, &user_secret(), "finitechat-native", name, expected)
}

fn wait_for_project_active_version(
    server: &TestServer,
    secret: &[u8; 32],
    project_slug: &str,
    name: &str,
    expected: Option<u32>,
) -> ProjectOutputSummary {
    let mut last: Option<ProjectOutputSummary> = None;
    // Bounded wait: the receive-pack request has already completed; this only
    // waits for the out-of-band reconciler spawned after that durable event.
    for _ in 0..60 {
        let status: ProjectStatusResponse = json_body(
            server
                .signed(
                    secret,
                    "GET",
                    &format!("/api/v1/projects/{project_slug}"),
                    None,
                )
                .unwrap(),
        );
        let summary = status
            .outputs
            .into_iter()
            .find(|output| output.site_name == name)
            .expect("project status contains output");
        if summary.active_version == expected {
            return summary;
        }
        last = Some(summary);
        std::thread::sleep(Duration::from_millis(50));
    }
    let summary = last.expect("site summary was fetched at least once");
    assert_eq!(summary.active_version, expected);
    summary
}

fn wait_for_pending_git_events(server: &TestServer, expected: usize) {
    for _ in 0..60 {
        let store = Store::open(&server.data_dir().join("registry.db")).unwrap();
        if store.pending_git_ref_events(None).unwrap().len() == expected {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let store = Store::open(&server.data_dir().join("registry.db")).unwrap();
    assert_eq!(store.pending_git_ref_events(None).unwrap().len(), expected);
}

fn project_output_status(server: &TestServer, name: &str) -> ProjectOutputSummary {
    let status: ProjectStatusResponse = json_body(
        server
            .signed(
                &user_secret(),
                "GET",
                "/api/v1/projects/finitechat-native",
                None,
            )
            .unwrap(),
    );
    status
        .outputs
        .into_iter()
        .find(|output| output.site_name == name)
        .expect("project status contains output")
}

fn project_init_request(dry_run: bool) -> ProjectInitRequest {
    let mut outputs = BTreeMap::new();
    outputs.insert(
        "mockup".to_string(),
        ProjectOutputConfig {
            kind: ProjectOutputKind::Site,
            site_name: Some("finitechat-native-mockup".to_string()),
            document_name: None,
            branch: "main".to_string(),
            path: ".".to_string(),
            entry: None,
            spa: false,
            start: None,
        },
    );
    ProjectInitRequest {
        config: ProjectConfig {
            project: ProjectSection {
                slug: "finitechat-native".to_string(),
            },
            outputs,
        },
        requesting_user_npub: None,
        owner_email: None,
        hosted_requester_assertion: None,
        dry_run,
    }
}

fn bare_project_init_request(slug: &str, dry_run: bool) -> ProjectInitRequest {
    ProjectInitRequest {
        config: ProjectConfig {
            project: ProjectSection {
                slug: slug.to_string(),
            },
            outputs: BTreeMap::new(),
        },
        requesting_user_npub: None,
        owner_email: None,
        hosted_requester_assertion: None,
        dry_run,
    }
}

fn single_site_project_init_request(
    slug: &str,
    output_id: &str,
    site_name: &str,
    path: &str,
    dry_run: bool,
) -> ProjectInitRequest {
    let mut outputs = BTreeMap::new();
    outputs.insert(
        output_id.to_string(),
        ProjectOutputConfig {
            kind: ProjectOutputKind::Site,
            site_name: Some(site_name.to_string()),
            document_name: None,
            branch: "main".to_string(),
            path: path.to_string(),
            entry: None,
            spa: false,
            start: None,
        },
    );
    ProjectInitRequest {
        config: ProjectConfig {
            project: ProjectSection {
                slug: slug.to_string(),
            },
            outputs,
        },
        requesting_user_npub: None,
        owner_email: None,
        hosted_requester_assertion: None,
        dry_run,
    }
}

fn site_and_document_project_init_request(dry_run: bool) -> ProjectInitRequest {
    let mut outputs = BTreeMap::new();
    outputs.insert(
        "doc".to_string(),
        ProjectOutputConfig {
            kind: ProjectOutputKind::Document,
            site_name: None,
            document_name: Some("finitechat-native-docs".to_string()),
            branch: "main".to_string(),
            path: "docs".to_string(),
            entry: Some("index.md".to_string()),
            spa: false,
            start: None,
        },
    );
    outputs.insert(
        "mockup".to_string(),
        ProjectOutputConfig {
            kind: ProjectOutputKind::Site,
            site_name: Some("finitechat-native-mockup".to_string()),
            document_name: None,
            branch: "main".to_string(),
            path: "site".to_string(),
            entry: None,
            spa: false,
            start: None,
        },
    );
    ProjectInitRequest {
        config: ProjectConfig {
            project: ProjectSection {
                slug: "finitechat-native".to_string(),
            },
            outputs,
        },
        requesting_user_npub: None,
        owner_email: None,
        hosted_requester_assertion: None,
        dry_run,
    }
}

fn app_project_init_request(dry_run: bool) -> ProjectInitRequest {
    let mut outputs = BTreeMap::new();
    outputs.insert(
        "web".to_string(),
        ProjectOutputConfig {
            kind: ProjectOutputKind::App,
            site_name: Some("finitechat-native-app".to_string()),
            document_name: None,
            branch: "main".to_string(),
            path: "app".to_string(),
            entry: None,
            spa: false,
            start: Some("bun server.ts".to_string()),
        },
    );
    ProjectInitRequest {
        config: ProjectConfig {
            project: ProjectSection {
                slug: "finitechat-native".to_string(),
            },
            outputs,
        },
        requesting_user_npub: None,
        owner_email: None,
        hosted_requester_assertion: None,
        dry_run,
    }
}

fn mint_skyler_git_credential(server: &TestServer) -> GitAuthResponse {
    let grant_body = serde_json::to_vec(&ProjectGrantRequest {
        email: "skyler@example.com".into(),
        npub: None,
        role: "editor".into(),
    })
    .unwrap();
    let _: ProjectGrantResponse = json_body(
        server
            .signed(
                &user_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/grant",
                Some(&grant_body),
            )
            .unwrap(),
    );

    let login_body = serde_json::to_vec(&EmailLoginRequest {
        email: "skyler@example.com".into(),
    })
    .unwrap();
    server
        .agent
        .post(&format!("{}/api/v1/email-auth/request", server.api_url))
        .set("Content-Type", "application/json")
        .send_bytes(&login_body)
        .unwrap();
    let token = outbox_email_token(&server.outbox);
    clear_outbox(&server.outbox);

    let redeem_body = serde_json::to_vec(&EmailRedeemRequest {
        email: "skyler@example.com".into(),
        token,
    })
    .unwrap();
    let redeemed: EmailRedeemResponse = json_body(
        server
            .signed(
                &stranger_secret(),
                "POST",
                "/api/v1/email-auth/redeem",
                Some(&redeem_body),
            )
            .unwrap(),
    );
    assert_eq!(redeemed.email, "skyler@example.com");

    let auth_body = serde_json::to_vec(&GitAuthRequest {
        email: Some("skyler@example.com".into()),
    })
    .unwrap();
    json_body(
        server
            .signed(
                &stranger_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/git-auth",
                Some(&auth_body),
            )
            .unwrap(),
    )
}

fn push_project_files(
    server: &TestServer,
    credential: &GitAuthResponse,
    finite_toml: &str,
    branch: &str,
    files: &[(&str, &str)],
    message: &str,
) {
    let dir = tempfile::tempdir().unwrap();
    let remote = format!(
        "http://{}:{}@127.0.0.1:{}/finitechat-native.git",
        credential.username,
        credential.password,
        server.port()
    );
    let host_header = format!("Host: git.{BASE_DOMAIN}:{}", server.port());
    run_git(
        &[
            "-c",
            &format!("http.extraHeader={host_header}"),
            "clone",
            &remote,
            "repo",
        ],
        Some(dir.path()),
    );
    let repo = dir.path().join("repo");
    if !try_git(&["checkout", branch], Some(&repo)) {
        run_git(&["checkout", "-b", branch], Some(&repo));
    }
    std::fs::write(repo.join("finite.toml"), finite_toml).unwrap();
    for (path, content) in files {
        let target = repo.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(target, content).unwrap();
    }
    let mut add_args = vec!["add", "finite.toml"];
    for (path, _) in files {
        add_args.push(path);
    }
    run_git(&add_args, Some(&repo));
    run_git(
        &[
            "-c",
            "user.email=skyler@example.com",
            "-c",
            "user.name=Skyler Bot",
            "commit",
            "-m",
            message,
        ],
        Some(&repo),
    );
    run_git(
        &[
            "-c",
            &format!("http.extraHeader={host_header}"),
            "push",
            "origin",
            branch,
        ],
        Some(&repo),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn self_registration_requires_mailbox_before_project_creation() {
    let server = TestServer::start_without_publish_grant(true).await;

    let task = tokio::task::spawn_blocking(move || {
        let apply_body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let denied = server.signed(
            &stranger_secret(),
            "POST",
            "/api/v1/projects/init",
            Some(&apply_body),
        );
        assert!(matches!(denied, Err(ureq::Error::Status(403, _))));

        let registered: AuthRegisterResponse = json_body(
            server
                .signed(
                    &stranger_secret(),
                    "POST",
                    "/api/v1/auth/register",
                    Some(&[]),
                )
                .unwrap(),
        );
        assert!(registered.registered);
        assert_eq!(registered.grant_source, "self");

        let replay: AuthRegisterResponse = json_body(
            server
                .signed(
                    &stranger_secret(),
                    "POST",
                    "/api/v1/auth/register",
                    Some(&[]),
                )
                .unwrap(),
        );
        assert!(!replay.registered);

        let ureq::Error::Status(422, response) = server
            .signed(
                &stranger_secret(),
                "POST",
                "/api/v1/projects/init",
                Some(&apply_body),
            )
            .unwrap_err()
        else {
            panic!("expected requester_email_required");
        };
        let body: ApiErrorBody = serde_json::from_reader(response.into_reader()).unwrap();
        assert_eq!(body.error, "requester_email_required");
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn bare_project_repository_accepts_source_push_before_output() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let bare_body =
            serde_json::to_vec(&bare_project_init_request("finite-skills", false)).unwrap();
        let bare: ProjectInitResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/init",
                    Some(&bare_body),
                )
                .unwrap(),
        );
        assert!(bare.created);
        assert!(bare.outputs.is_empty());

        let auth_body = serde_json::to_vec(&GitAuthRequest { email: None }).unwrap();
        let credential: GitAuthResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finite-skills/git-auth",
                    Some(&auth_body),
                )
                .unwrap(),
        );
        let dir = tempfile::tempdir().unwrap();
        let remote = format!(
            "http://{}:{}@127.0.0.1:{}/finite-skills.git",
            credential.username,
            credential.password,
            server.port()
        );
        let host_header = format!("Host: git.{BASE_DOMAIN}:{}", server.port());
        let clone = run_git_capture(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "clone",
                &remote,
                "repo",
            ],
            Some(dir.path()),
        );
        assert!(
            !String::from_utf8_lossy(&clone.stderr)
                .contains("remote HEAD refers to nonexistent ref")
        );
        let repo = dir.path().join("repo");
        run_git(&["checkout", "-b", "main"], Some(&repo));
        std::fs::write(repo.join("finite.toml"), bare.finite_toml).unwrap();
        std::fs::write(repo.join("README.md"), "# Finite Skills\n").unwrap();
        run_git(&["add", "finite.toml", "README.md"], Some(&repo));
        run_git(
            &[
                "-c",
                "user.email=paul@finite.vip",
                "-c",
                "user.name=Paul",
                "commit",
                "-m",
                "Initial source",
            ],
            Some(&repo),
        );
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "push",
                "origin",
                "main",
            ],
            Some(&repo),
        );
        wait_for_pending_git_events(&server, 0);

        let status: ProjectStatusResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "GET",
                    "/api/v1/projects/finite-skills",
                    None,
                )
                .unwrap(),
        );
        assert!(status.outputs.is_empty());

        let output_body = serde_json::to_vec(&single_site_project_init_request(
            "finite-skills",
            "site",
            "finite-skills-site",
            "site",
            false,
        ))
        .unwrap();
        let with_output: ProjectInitResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/init",
                    Some(&output_body),
                )
                .unwrap(),
        );
        assert!(!with_output.created);
        assert_eq!(with_output.outputs.len(), 1);
        assert!(with_output.outputs[0].created);

        std::fs::write(repo.join("finite.toml"), with_output.finite_toml).unwrap();
        std::fs::create_dir_all(repo.join("site")).unwrap();
        std::fs::write(repo.join("site/index.html"), "<h1>skills</h1>").unwrap();
        run_git(&["add", "finite.toml", "site/index.html"], Some(&repo));
        run_git(
            &[
                "-c",
                "user.email=paul@finite.vip",
                "-c",
                "user.name=Paul",
                "commit",
                "-m",
                "Add public site output",
            ],
            Some(&repo),
        );
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "push",
                "origin",
                "main",
            ],
            Some(&repo),
        );

        let summary = wait_for_project_active_version(
            &server,
            &user_secret(),
            "finite-skills",
            "finite-skills-site",
            Some(1),
        );
        assert_eq!(summary.path, "site");
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn public_read_project_visibility_allows_anonymous_clone_but_not_push() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&bare_project_init_request("finite-skills", false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        assert_eq!(created.project_visibility, "private");

        let auth_body = serde_json::to_vec(&GitAuthRequest { email: None }).unwrap();
        let credential: GitAuthResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finite-skills/git-auth",
                    Some(&auth_body),
                )
                .unwrap(),
        );

        let host_header = format!("Host: git.{BASE_DOMAIN}:{}", server.port());
        let auth_remote = format!(
            "http://{}:{}@127.0.0.1:{}/finite-skills.git",
            credential.username,
            credential.password,
            server.port()
        );
        let public_remote = format!("http://127.0.0.1:{}/finite-skills.git", server.port());

        let denied_dir = tempfile::tempdir().unwrap();
        run_git_expect_failure(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "clone",
                &public_remote,
                "repo",
            ],
            Some(denied_dir.path()),
        );

        let seeded_dir = tempfile::tempdir().unwrap();
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "clone",
                &auth_remote,
                "repo",
            ],
            Some(seeded_dir.path()),
        );
        let seeded_repo = seeded_dir.path().join("repo");
        run_git(&["checkout", "-b", "main"], Some(&seeded_repo));
        std::fs::write(seeded_repo.join("finite.toml"), created.finite_toml).unwrap();
        std::fs::write(seeded_repo.join("README.md"), "# Finite Skills\n").unwrap();
        run_git(&["add", "finite.toml", "README.md"], Some(&seeded_repo));
        run_git(
            &[
                "-c",
                "user.email=paul@finite.vip",
                "-c",
                "user.name=Paul",
                "commit",
                "-m",
                "Seed skills",
            ],
            Some(&seeded_repo),
        );
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "push",
                "origin",
                "main",
            ],
            Some(&seeded_repo),
        );
        wait_for_pending_git_events(&server, 0);

        {
            let mut store = Store::open(&server.data_dir().join("registry.db")).unwrap();
            let update = store
                .set_project_visibility_by_slug(
                    "finite-skills",
                    ProjectVisibility::PublicRead,
                    now_unix(),
                )
                .unwrap();
            assert!(update.changed);
        }

        let status: ProjectStatusResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "GET",
                    "/api/v1/projects/finite-skills",
                    None,
                )
                .unwrap(),
        );
        assert_eq!(status.project_visibility, "public-read");

        let public_dir = tempfile::tempdir().unwrap();
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "clone",
                &public_remote,
                "repo",
            ],
            Some(public_dir.path()),
        );
        let public_repo = public_dir.path().join("repo");
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "fetch",
                "origin",
            ],
            Some(&public_repo),
        );

        let stale_credentials_dir = tempfile::tempdir().unwrap();
        let stale_credentials_remote = format!(
            "http://stale:wrong@127.0.0.1:{}/finite-skills.git",
            server.port()
        );
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "clone",
                &stale_credentials_remote,
                "repo",
            ],
            Some(stale_credentials_dir.path()),
        );

        std::fs::write(public_repo.join("README.md"), "# Public edit attempt\n").unwrap();
        run_git(&["add", "README.md"], Some(&public_repo));
        run_git(
            &[
                "-c",
                "user.email=anon@example.com",
                "-c",
                "user.name=Anonymous",
                "commit",
                "-m",
                "Anonymous edit",
            ],
            Some(&public_repo),
        );
        run_git_expect_failure(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "push",
                "origin",
                "main",
            ],
            Some(&public_repo),
        );

        run_git(
            &["remote", "set-url", "origin", &auth_remote],
            Some(&public_repo),
        );
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "push",
                "origin",
                "main",
            ],
            Some(&public_repo),
        );
        wait_for_pending_git_events(&server, 0);
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn full_publish_share_and_view_flow() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;
    let port = server.port();

    let task = tokio::task::spawn_blocking(move || {
        let health = server
            .agent
            .get(&format!("{}/api/v1/healthz", server.api_url))
            .call();
        assert!(health.is_ok());

        let apply_body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let denied = server.signed(
            &stranger_secret(),
            "POST",
            "/api/v1/projects/init",
            Some(&apply_body),
        );
        assert!(matches!(denied, Err(ureq::Error::Status(403, _))));

        let no_auth = server
            .agent
            .post(&format!("{}/api/v1/projects/init", server.api_url))
            .set("Authorization", "Nostr bm90LWFuLWV2ZW50")
            .send_bytes(&apply_body);
        assert!(matches!(no_auth, Err(ureq::Error::Status(401, _))));

        let created: ProjectInitResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/init",
                    Some(&apply_body),
                )
                .unwrap(),
        );
        let placeholder = server
            .site_get("finitechat-native-mockup", "/", port)
            .unwrap();
        assert!(placeholder.into_string().unwrap().contains("claimed"));

        let credential = mint_skyler_git_credential(&server);
        push_project_files(
            &server,
            &credential,
            &created.finite_toml,
            "main",
            &[
                ("index.html", "<h1>hello from finite</h1>"),
                ("css/style.css", "body { background: black }"),
            ],
            "Initial deploy",
        );
        let summary = wait_for_active_version(&server, "finitechat-native-mockup", Some(1));
        assert_eq!(summary.active_version, Some(1));

        // Auth-needed is now a top-level redirect to the Auth Gate carrying
        // this output's canonical origin and the return path.
        let gated = server
            .site_get("finitechat-native-mockup", "/", port)
            .unwrap();
        assert_eq!(gated.status(), 303);
        let gate_redirect = gated.header("location").unwrap().to_string();
        let expected_origin = format!("http://finitechat-native-mockup.{BASE_DOMAIN}:{port}");
        assert!(
            gate_redirect.starts_with(&format!(
                "{AUTH_GATE_URL}/authorize?output={}",
                urlencode(&expected_origin)
            )),
            "{gate_redirect}"
        );
        assert!(gated.header("cache-control").unwrap().contains("no-store"));

        let share_body = serde_json::to_vec(&SharingRequest {
            visibility: Some("shared".into()),
            confirm_public: false,
            add_emails: vec!["friend@example.com".into()],
            remove_emails: vec![],
            add_npubs: vec![],
            remove_npubs: vec![],
        })
        .unwrap();
        let shared: ProjectOutputSharingResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/outputs/mockup/sharing",
                    Some(&share_body),
                )
                .unwrap(),
        );
        assert_eq!(shared.project_slug, "finitechat-native");
        assert_eq!(shared.output_id, "mockup");
        assert_eq!(
            shared.shared_emails,
            vec!["friend@example.com", "owner@example.com"]
        );
        let old_site_route = server
            .signed(
                &user_secret(),
                "POST",
                "/api/v1/sites/finitechat-native-mockup/sharing",
                Some(&share_body),
            )
            .unwrap_err();
        assert!(matches!(old_site_route, ureq::Error::Status(404, _)));

        let site_base = format!("http://finitechat-native-mockup.{BASE_DOMAIN}:{port}");
        // A gate vouch for an unshared mailbox proves the mailbox (so the
        // request-access flow stays usable) but grants no view.
        let stranger_vouch = server.gate_vouch(&site_base, "stranger@example.com");
        let ureq::Error::Status(403, not_shared) = server
            .redeem_gate_code(&site_base, &stranger_vouch, "/")
            .unwrap_err()
        else {
            panic!("verified unshared mailbox should see the not-shared page");
        };
        let stranger_cookie = not_shared
            .header("set-cookie")
            .expect("verified mailbox receives a durable viewer cookie")
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        assert!(
            not_shared
                .into_string()
                .unwrap()
                .contains("You don&rsquo;t have access yet")
        );
        for entry in std::fs::read_dir(&server.outbox).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        let requested = server
            .agent
            .post(&format!("{site_base}/_finite/request-access"))
            .set("Cookie", &stranger_cookie)
            .send_bytes(&[])
            .unwrap();
        assert!(requested.into_string().unwrap().contains("Request sent"));
        let approval_link = outbox_link(&server.outbox);
        let confirmation = server.agent.get(&approval_link).call().unwrap();
        assert!(
            confirmation
                .into_string()
                .unwrap()
                .contains("Share this site")
        );
        let approval_token = approval_link.split("token=").nth(1).unwrap();
        let approved = server
            .agent
            .post(&format!("{site_base}/_finite/approve-access"))
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string(&format!("token={approval_token}"))
            .unwrap();
        assert!(approved.into_string().unwrap().contains("Access approved"));
        let stranger_page = server
            .agent
            .get(&format!("{site_base}/"))
            .set("Cookie", &stranger_cookie)
            .call()
            .unwrap();
        assert_eq!(
            stranger_page.into_string().unwrap(),
            "<h1>hello from finite</h1>"
        );
        for entry in std::fs::read_dir(&server.outbox).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }

        // A gate vouch for the shared mailbox mints the viewer cookie and
        // continues to the return path. No mail is involved anymore.
        let friend_vouch = server.gate_vouch(&site_base, "friend@example.com");
        let redeemed = server
            .redeem_gate_code(&site_base, &friend_vouch, "/gallery?x=1")
            .unwrap();
        assert_eq!(redeemed.status(), 303);
        assert_eq!(redeemed.header("location").unwrap(), "/gallery?x=1");
        let cookie = redeemed
            .header("set-cookie")
            .expect("vouch redemption sets a cookie")
            .split(';')
            .next()
            .unwrap()
            .to_string();
        assert!(cookie.starts_with("finite_site_auth="));
        assert_eq!(std::fs::read_dir(&server.outbox).unwrap().count(), 0);

        // The same vouch is single-use: replaying it fails closed.
        let replayed = server.redeem_gate_code(&site_base, &friend_vouch, "/");
        assert!(matches!(replayed, Err(ureq::Error::Status(400, _))));

        // A vouch minted for another output origin is not a passport here.
        let other_origin = format!("http://other.{BASE_DOMAIN}:{port}");
        let cross_site_vouch = server.gate_vouch(&other_origin, "friend@example.com");
        let cross_site = server.redeem_gate_code(&site_base, &cross_site_vouch, "/");
        assert!(matches!(cross_site, Err(ureq::Error::Status(400, _))));

        // A fresh vouch for the same viewer mints a fresh working session.
        let second_vouch = server.gate_vouch(&site_base, "friend@example.com");
        let second = server
            .redeem_gate_code(&site_base, &second_vouch, "/")
            .unwrap();
        assert_eq!(second.status(), 303);
        assert!(second.header("set-cookie").is_some());

        let page = server
            .agent
            .get(&format!("{site_base}/"))
            .set("Cookie", &cookie)
            .call()
            .unwrap();
        assert_eq!(
            page.header("content-type").unwrap(),
            "text/html; charset=utf-8"
        );
        assert_eq!(page.header("cache-control"), Some("private, no-store"));
        let etag = page.header("etag").unwrap().to_string();
        assert_eq!(page.into_string().unwrap(), "<h1>hello from finite</h1>");

        let revalidated = server
            .agent
            .get(&format!("{site_base}/"))
            .set("Cookie", &cookie)
            .set("If-None-Match", &etag)
            .call()
            .unwrap();
        assert_eq!(revalidated.status(), 304);
        assert_eq!(
            revalidated.header("cache-control"),
            Some("private, no-store")
        );

        let css = server
            .agent
            .get(&format!("{site_base}/css/style.css"))
            .set("Cookie", &cookie)
            .call()
            .unwrap();
        assert_eq!(
            css.header("content-type").unwrap(),
            "text/css; charset=utf-8"
        );

        let missing = server
            .agent
            .get(&format!("{site_base}/nope.html"))
            .set("Cookie", &cookie)
            .call();
        assert!(matches!(missing, Err(ureq::Error::Status(404, _))));

        let public_body = serde_json::to_vec(&SharingRequest {
            visibility: Some("public".into()),
            confirm_public: true,
            add_emails: vec![],
            remove_emails: vec![],
            add_npubs: vec![],
            remove_npubs: vec![],
        })
        .unwrap();
        server
            .signed(
                &user_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/outputs/mockup/sharing",
                Some(&public_body),
            )
            .unwrap();
        let open = server
            .site_get("finitechat-native-mockup", "/", port)
            .unwrap();
        assert_eq!(open.header("cache-control"), Some("no-store"));
        assert_eq!(open.into_string().unwrap(), "<h1>hello from finite</h1>");

        {
            let mut store = Store::open(&server.data_dir.path().join("registry.db")).unwrap();
            let deleted = store
                .set_site_status_by_name(
                    "finitechat-native-mockup",
                    SiteStatus::Deleted,
                    "site_deleted",
                    now_unix(),
                )
                .unwrap();
            assert!(deleted.changed);
        }
        let deleted = server.site_get("finitechat-native-mockup", "/", port);
        assert!(matches!(deleted, Err(ureq::Error::Status(404, _))));

        let unknown = server.site_get("ghost", "/", port);
        let Err(ureq::Error::Status(404, response)) = unknown else {
            panic!("expected 404 for unknown site");
        };
        assert!(
            response
                .into_string()
                .unwrap()
                .contains("No site lives here")
        );
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn gate_vouch_viewing_replaces_the_deleted_session_exchanges() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;
    let port = server.port();

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        let credential = mint_skyler_git_credential(&server);
        push_project_files(
            &server,
            &credential,
            &created.finite_toml,
            "main",
            &[("index.html", "<h1>account preview</h1>")],
            "Viewer session deploy",
        );
        wait_for_active_version(&server, "finitechat-native-mockup", Some(1));

        let site_base = format!("http://finitechat-native-mockup.{BASE_DOMAIN}:{port}");

        // The internal account-to-session exchanges are gone entirely.
        for dead in [
            "/internal/v1/viewer-sessions",
            "/internal/v1/native-viewer-sessions",
        ] {
            assert!(matches!(
                server
                    .agent
                    .post(&format!("{}{dead}", server.api_url))
                    .set(
                        "Authorization",
                        &format!("Bearer {VIEWER_SESSION_SERVICE_TOKEN}")
                    )
                    .send_bytes(b"{}"),
                Err(ureq::Error::Status(404, _))
            ));
        }

        // An unshared verified mailbox reaches the not-shared page (and can
        // request access), but its cookie grants no view.
        let unshared_vouch = server.gate_vouch(&site_base, "Friend@Example.com");
        let ureq::Error::Status(403, not_shared) = server
            .redeem_gate_code(&site_base, &unshared_vouch, "/")
            .unwrap_err()
        else {
            panic!("verified but unshared mailbox must reach the not-shared page");
        };
        assert!(not_shared.into_string().unwrap().contains("Request access"));

        // Sharing the mailbox makes the same ceremony succeed.
        let share_body = serde_json::to_vec(&SharingRequest {
            visibility: Some("shared".into()),
            confirm_public: false,
            add_emails: vec!["friend@example.com".into()],
            remove_emails: vec![],
            add_npubs: vec![],
            remove_npubs: vec![],
        })
        .unwrap();
        server
            .signed(
                &user_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/outputs/mockup/sharing",
                Some(&share_body),
            )
            .unwrap();

        let vouch = server.gate_vouch(&site_base, "friend@example.com");
        let redeemed = server
            .redeem_gate_code(&site_base, &vouch, "/gallery?view=one#photo")
            .unwrap();
        assert_eq!(redeemed.status(), 303);
        assert_eq!(redeemed.header("location"), Some("/gallery?view=one#photo"));
        let set_cookies = redeemed.all("set-cookie");
        assert_eq!(set_cookies.len(), 2);
        let ordinary_set_cookie = set_cookies
            .iter()
            .find(|cookie| cookie.starts_with("finite_site_auth="))
            .unwrap();
        assert!(
            ordinary_set_cookie.ends_with("Path=/; Max-Age=604800; HttpOnly; SameSite=Lax; Secure")
        );
        let partitioned_set_cookie = set_cookies
            .iter()
            .find(|cookie| cookie.starts_with("__Host-finite_site_auth_partitioned="))
            .unwrap();
        assert!(
            partitioned_set_cookie
                .ends_with("Path=/; Max-Age=604800; HttpOnly; SameSite=None; Secure; Partitioned")
        );
        let ordinary_cookie = ordinary_set_cookie.split(';').next().unwrap().to_string();
        let partitioned_cookie = partitioned_set_cookie
            .split(';')
            .next()
            .unwrap()
            .to_string();

        // Single use: the redeemed vouch cannot be replayed.
        assert!(matches!(
            server.redeem_gate_code(&site_base, &vouch, "/"),
            Err(ureq::Error::Status(400, _))
        ));
        // Garbage and wrong-key vouches fail closed without side effects.
        assert!(matches!(
            server.redeem_gate_code(&site_base, "garbage", "/"),
            Err(ureq::Error::Status(400, _))
        ));
        let mut other_key = [0x77u8; 32];
        other_key[0] = GATE_SIGNING_KEY[0].wrapping_add(1);
        let wrong_key_vouch = {
            let nonce = finitesites_proto::ids::random_32();
            finite_authn::mint_vouch(
                &other_key,
                &site_base,
                "friend@example.com",
                now_unix(),
                &finite_authn::AuthPolicy::default(),
                nonce,
            )
            .unwrap()
        };
        assert!(matches!(
            server.redeem_gate_code(&site_base, &wrong_key_vouch, "/"),
            Err(ureq::Error::Status(400, _))
        ));
        // Bad return paths are rejected before anything is trusted.
        let bounded_vouch = server.gate_vouch(&site_base, "friend@example.com");
        assert!(matches!(
            server.redeem_gate_code(&site_base, &bounded_vouch, "//evil.example"),
            Err(ureq::Error::Status(400, _))
        ));
        // The origin binding is exact: a vouch for a different output is
        // not a passport to this one.
        let second_origin = format!("http://second-preview-site.{BASE_DOMAIN}:{port}");
        let cross_vouch = server.gate_vouch(&second_origin, "friend@example.com");
        assert!(matches!(
            server.redeem_gate_code(&site_base, &cross_vouch, "/"),
            Err(ureq::Error::Status(400, _))
        ));

        // Existing cookies keep working; both cookie kinds view content.
        let clean_agent = agent_for(SocketAddr::from(([127, 0, 0, 1], port)));
        let page = clean_agent
            .get(&format!("{site_base}/"))
            .set("Cookie", &ordinary_cookie)
            .call()
            .unwrap();
        assert_eq!(page.into_string().unwrap(), "<h1>account preview</h1>");
        let iframe_page = clean_agent
            .get(&format!("{site_base}/"))
            .set("Cookie", &partitioned_cookie)
            .call()
            .unwrap();
        assert_eq!(
            iframe_page.into_string().unwrap(),
            "<h1>account preview</h1>"
        );

        // Logout clears both cookies.
        let logout = clean_agent
            .get(&format!("{site_base}/_finite/logout"))
            .call()
            .unwrap();
        assert_eq!(logout.status(), 303);
        assert_eq!(logout.header("location"), Some("/"));
        let cleared = logout.all("set-cookie");
        assert_eq!(
            cleared,
            vec![
                "finite_site_auth=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax; Secure",
                "__Host-finite_site_auth_partitioned=; Path=/; Max-Age=0; HttpOnly; SameSite=None; Secure; Partitioned",
            ]
        );

        // Revoking the share closes live sessions: view access flips to the
        // gate redirect, and a fresh vouch lands on the not-shared page.
        let revoke_body = serde_json::to_vec(&SharingRequest {
            visibility: Some("shared".into()),
            confirm_public: false,
            add_emails: vec![],
            remove_emails: vec!["friend@example.com".into()],
            add_npubs: vec![],
            remove_npubs: vec![],
        })
        .unwrap();
        server
            .signed(
                &user_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/outputs/mockup/sharing",
                Some(&revoke_body),
            )
            .unwrap();
        let revoked_view = clean_agent
            .get(&format!("{site_base}/"))
            .set("Cookie", &ordinary_cookie)
            .call()
            .unwrap();
        assert_eq!(revoked_view.status(), 303);
        assert!(
            revoked_view
                .header("location")
                .unwrap()
                .starts_with(&format!("{AUTH_GATE_URL}/authorize?output="))
        );
        let after_revoke_vouch = server.gate_vouch(&site_base, "friend@example.com");
        assert!(matches!(
            server.redeem_gate_code(&site_base, &after_revoke_vouch, "/"),
            Err(ureq::Error::Status(403, _))
        ));
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn hosted_requester_assertion_binds_verified_mailbox_to_exact_agent_project_init() {
    let owner_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let requester_npub = finitesites_proto::npub::encode_npub(&owner_pubkey).unwrap();
    let agent_pubkey = finitesites_proto::event::pubkey_for_secret(&stranger_secret()).unwrap();
    let agent_npub = finitesites_proto::npub::encode_npub(&agent_pubkey).unwrap();
    let other_agent_pubkey = finitesites_proto::event::pubkey_for_secret(&viewer_secret()).unwrap();
    let server = TestServer::start(&owner_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let mut store = Store::open(&server.data_dir().join("registry.db")).unwrap();
        store
            .allow_pubkey(&agent_pubkey, "hosted agent", now_unix())
            .unwrap();
        store
            .allow_pubkey(&other_agent_pubkey, "other hosted agent", now_unix())
            .unwrap();
        drop(store);

        let assertion_request = HostedRequesterAssertionRequest {
            email: "owner@example.com".to_owned(),
            requester_npub: requester_npub.clone(),
            agent_npub: agent_npub.clone(),
        };
        assert!(matches!(
            server.hosted_requester_assertion(None, &assertion_request),
            Err(ureq::Error::Status(401, _))
        ));
        let assertion: HostedRequesterAssertionResponse = json_body(
            server
                .hosted_requester_assertion(Some(VIEWER_SESSION_SERVICE_TOKEN), &assertion_request)
                .unwrap(),
        );
        assert_eq!(assertion.email, "owner@example.com");
        assert_eq!(assertion.requester_npub, requester_npub);
        assert_eq!(assertion.agent_npub, agent_npub);

        let mut init = project_init_request(false);
        init.requesting_user_npub = Some(assertion.requester_npub);
        init.owner_email = Some(assertion.email.clone());
        init.hosted_requester_assertion = Some(assertion.assertion);
        let body = serde_json::to_vec(&init).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(
                    &stranger_secret(),
                    "POST",
                    "/api/v1/projects/init",
                    Some(&body),
                )
                .unwrap(),
        );
        assert_eq!(created.owner_email.as_deref(), Some("owner@example.com"));
        assert!(created.outputs[0].requesting_user_shared);

        let mut wrong_agent = init;
        wrong_agent.config.project.slug = "wrong-agent".to_owned();
        let wrong_body = serde_json::to_vec(&wrong_agent).unwrap();
        assert!(matches!(
            server.signed(
                &viewer_secret(),
                "POST",
                "/api/v1/projects/init",
                Some(&wrong_body)
            ),
            Err(ureq::Error::Status(403, _))
        ));
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn share_rows_are_the_invitation_and_send_no_viewer_mail() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;
    let port = server.port();

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        let credential = mint_skyler_git_credential(&server);
        push_project_files(
            &server,
            &credential,
            &created.finite_toml,
            "main",
            &[("index.html", "<h1>invite</h1>")],
            "Invite test deploy",
        );
        wait_for_active_version(&server, "finitechat-native-mockup", Some(1));

        // Sharing adds access and sends nothing: the share row is the
        // invitation, and the added viewer signs in through the Auth Gate
        // at the site URL.
        let share_body = serde_json::to_vec(&SharingRequest {
            visibility: Some("shared".into()),
            confirm_public: false,
            add_emails: vec!["Friend@Example.com".into()],
            remove_emails: vec![],
            add_npubs: vec![],
            remove_npubs: vec![],
        })
        .unwrap();
        let shared: ProjectOutputSharingResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/outputs/mockup/sharing",
                    Some(&share_body),
                )
                .unwrap(),
        );
        assert_eq!(shared.project_slug, "finitechat-native");
        assert_eq!(shared.output_id, "mockup");
        assert_eq!(
            shared.shared_emails,
            vec!["friend@example.com", "owner@example.com"]
        );
        assert!(shared.invited_emails.is_empty());
        assert!(outbox_bodies(&server.outbox).is_empty());

        // The shared viewer can still view immediately, through the gate.
        let site_base = format!("http://finitechat-native-mockup.{BASE_DOMAIN}:{port}");
        let vouch = server.gate_vouch(&site_base, "friend@example.com");
        let redeemed = server.redeem_gate_code(&site_base, &vouch, "/").unwrap();
        assert_eq!(redeemed.status(), 303);
        let cookie = redeemed
            .all("set-cookie")
            .into_iter()
            .find(|cookie| cookie.starts_with("finite_site_auth="))
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let page = server
            .agent
            .get(&format!("{site_base}/"))
            .set("Cookie", &cookie)
            .call()
            .unwrap();
        assert_eq!(page.into_string().unwrap(), "<h1>invite</h1>");

        // Replays are idempotent and still send nothing.
        let replay: ProjectOutputSharingResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/outputs/mockup/sharing",
                    Some(&share_body),
                )
                .unwrap(),
        );
        assert_eq!(
            replay.shared_emails,
            vec!["friend@example.com", "owner@example.com"]
        );
        assert!(replay.invited_emails.is_empty());
        assert!(outbox_bodies(&server.outbox).is_empty());
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn project_init_repository_failure_is_repaired_by_one_replay() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let git_parent = server.data_dir().join("git");
        std::fs::write(&git_parent, "blocks the project repository directory").unwrap();
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();

        let failed = server
            .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
            .unwrap_err();
        let ureq::Error::Status(503, response) = failed else {
            panic!("expected repository setup to return 503");
        };
        let error: finitesites_proto::dto::ApiErrorBody = json_body(response);
        assert_eq!(
            error.error,
            finitesites_proto::dto::ERROR_GIT_REPOSITORY_SETUP_FAILED
        );
        assert!(error.message.contains("registry state was saved"));
        assert!(
            error
                .message
                .contains("replay this exact Project Init request once")
        );

        let store = Store::open(&server.data_dir().join("registry.db")).unwrap();
        let partial = store
            .project_by_slug("finitechat-native")
            .unwrap()
            .expect("registry transaction remains durable");
        assert_eq!(store.project_outputs(&partial.id).unwrap().len(), 1);
        drop(store);

        std::fs::remove_file(&git_parent).unwrap();
        let repaired: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        assert!(!repaired.created);
        assert!(!repaired.outputs[0].created);
        assert_eq!(repaired.project_id.as_deref(), Some(partial.id.as_str()));
        let repo =
            finitesitesd::git::project_root(server.data_dir()).join(format!("{}.git", partial.id));
        assert!(repo.join("HEAD").is_file());
        assert!(repo.join("hooks/post-receive").is_file());
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn project_init_and_git_auth_flow() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let dry_body = serde_json::to_vec(&project_init_request(true)).unwrap();
        let dry_run: ProjectInitResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/init",
                    Some(&dry_body),
                )
                .unwrap(),
        );
        assert!(dry_run.dry_run);
        assert!(dry_run.created);
        assert_eq!(dry_run.project_id, None);
        assert_eq!(
            dry_run.git_remote_url,
            format!(
                "http://git.{BASE_DOMAIN}:{}/finitechat-native.git",
                server.port()
            )
        );
        assert!(dry_run.finite_toml.contains("[outputs.mockup]"));

        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        assert!(!created.dry_run);
        assert!(created.created);
        assert!(created.project_id.is_some());
        assert!(created.outputs[0].created);
        assert_eq!(created.outputs[0].site_name, "finitechat-native-mockup");

        let replay: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        assert!(!replay.created);
        assert!(!replay.outputs[0].created);
        assert_eq!(replay.project_id, created.project_id);

        let owner_auth_body = serde_json::to_vec(&GitAuthRequest { email: None }).unwrap();
        let owner_credential: GitAuthResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/git-auth",
                    Some(&owner_auth_body),
                )
                .unwrap(),
        );
        assert_eq!(owner_credential.project_slug, "finitechat-native");
        assert_eq!(owner_credential.username, owner_credential.credential_id);
        assert_eq!(owner_credential.password.len(), 64);

        let unauthorized_native = server
            .signed(
                &stranger_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/git-auth",
                Some(&owner_auth_body),
            )
            .unwrap_err();
        assert!(matches!(unauthorized_native, ureq::Error::Status(403, _)));

        let bad_auth = serde_json::to_vec(&GitAuthRequest {
            email: Some("skyler@example.com".into()),
        })
        .unwrap();
        let unverified = server
            .signed(
                &stranger_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/git-auth",
                Some(&bad_auth),
            )
            .unwrap_err();
        assert!(matches!(unverified, ureq::Error::Status(403, _)));

        let grant_body = serde_json::to_vec(&ProjectGrantRequest {
            email: "skyler@example.com".into(),
            npub: None,
            role: "editor".into(),
        })
        .unwrap();
        let grant: ProjectGrantResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/grant",
                    Some(&grant_body),
                )
                .unwrap(),
        );
        assert!(grant.collaborator.created);

        let login_body = serde_json::to_vec(&EmailLoginRequest {
            email: "skyler@example.com".into(),
        })
        .unwrap();
        let login: EmailLoginResponse = server
            .agent
            .post(&format!("{}/api/v1/email-auth/request", server.api_url))
            .set("Content-Type", "application/json")
            .send_bytes(&login_body)
            .unwrap()
            .into_json()
            .unwrap();
        assert_eq!(login.email, "skyler@example.com");
        let token = outbox_email_token(&server.outbox);
        clear_outbox(&server.outbox);

        let redeem_body = serde_json::to_vec(&EmailRedeemRequest {
            email: "skyler@example.com".into(),
            token,
        })
        .unwrap();
        let redeemed: EmailRedeemResponse = json_body(
            server
                .signed(
                    &stranger_secret(),
                    "POST",
                    "/api/v1/email-auth/redeem",
                    Some(&redeem_body),
                )
                .unwrap(),
        );
        assert_eq!(redeemed.email, "skyler@example.com");

        let credential: GitAuthResponse = json_body(
            server
                .signed(
                    &stranger_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/git-auth",
                    Some(&bad_auth),
                )
                .unwrap(),
        );
        assert_eq!(credential.project_slug, "finitechat-native");
        assert_eq!(credential.username, credential.credential_id);
        assert_eq!(credential.password.len(), 64);
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn linked_email_satisfies_git_auth_without_sites_email_key() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let stranger_pubkey = finitesites_proto::event::pubkey_for_secret(&stranger_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        assert!(created.created);

        let grant_body = serde_json::to_vec(&ProjectGrantRequest {
            email: "skyler@example.com".into(),
            npub: None,
            role: "editor".into(),
        })
        .unwrap();
        let grant: ProjectGrantResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/grant",
                    Some(&grant_body),
                )
                .unwrap(),
        );
        assert!(grant.collaborator.created);

        // A daemon-local Email Link, with no Sites email key and no Identity
        // Authority anywhere, satisfies the email grant for the linked key.
        {
            let mut store = Store::open(&server.data_dir().join("registry.db")).unwrap();
            store
                .link_email_to_native_principal("skyler@example.com", &stranger_pubkey, now_unix())
                .unwrap();
        }

        let auth_body = serde_json::to_vec(&GitAuthRequest {
            email: Some("skyler@example.com".into()),
        })
        .unwrap();
        let credential: GitAuthResponse = json_body(
            server
                .signed(
                    &stranger_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/git-auth",
                    Some(&auth_body),
                )
                .unwrap(),
        );
        assert_eq!(credential.project_slug, "finitechat-native");
        assert_eq!(credential.username, credential.credential_id);
        assert_eq!(credential.password.len(), 64);

        // An unlinked key with the same email claim fails closed.
        let error = server
            .signed(
                &viewer_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/git-auth",
                Some(&auth_body),
            )
            .unwrap_err();
        assert!(matches!(error, ureq::Error::Status(403, _)));
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn email_proof_registers_and_revokes_sites_key_without_identity_authority() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let stranger_npub = finitesites_proto::npub::encode_npub(
        &finitesites_proto::event::pubkey_for_secret(&stranger_secret()).unwrap(),
    )
    .unwrap();
    let server = TestServer::start(&user_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        assert!(created.created);
        let grant_body = serde_json::to_vec(&ProjectGrantRequest {
            email: "paul@finite.vip".into(),
            npub: None,
            role: "editor".into(),
        })
        .unwrap();
        server
            .signed(
                &user_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/grant",
                Some(&grant_body),
            )
            .unwrap();

        // Daemon-local challenge: the token arrives via the local mailer.
        let login_body = serde_json::to_vec(&EmailLoginRequest {
            email: "paul@finite.vip".into(),
        })
        .unwrap();
        server
            .agent
            .post(&format!("{}/api/v1/email-auth/request", server.api_url))
            .set("Content-Type", "application/json")
            .send_bytes(&login_body)
            .unwrap();
        let token = outbox_email_token(&server.outbox);
        clear_outbox(&server.outbox);

        let register_body = serde_json::to_vec(&SitesAuthorizedKeyRegisterRequest {
            email: "paul@finite.vip".into(),
            token: token.clone(),
        })
        .unwrap();
        let registered: SitesAuthorizedKeyResponse = json_body(
            server
                .signed(
                    &stranger_secret(),
                    "POST",
                    "/api/v1/sites-authorized-keys/register",
                    Some(&register_body),
                )
                .unwrap(),
        );
        assert!(registered.active);
        assert_eq!(registered.email, "paul@finite.vip");
        assert_eq!(registered.npub, stranger_npub);

        // The proof is single-use: replaying it fails closed.
        let replay = server
            .signed(
                &stranger_secret(),
                "POST",
                "/api/v1/sites-authorized-keys/register",
                Some(&register_body),
            )
            .unwrap_err();
        assert!(matches!(replay, ureq::Error::Status(401, _)));

        let auth_body = serde_json::to_vec(&GitAuthRequest {
            email: Some("paul@finite.vip".into()),
        })
        .unwrap();
        server
            .signed(
                &stranger_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/git-auth",
                Some(&auth_body),
            )
            .unwrap();

        // Revocation also requires a fresh daemon-local proof.
        server
            .agent
            .post(&format!("{}/api/v1/email-auth/request", server.api_url))
            .set("Content-Type", "application/json")
            .send_bytes(&login_body)
            .unwrap();
        let fresh_token = outbox_email_token(&server.outbox);
        clear_outbox(&server.outbox);
        let revoke_body = serde_json::to_vec(&SitesAuthorizedKeyRevokeRequest {
            email: "paul@finite.vip".into(),
            token: fresh_token,
            target_npub: stranger_npub.clone(),
        })
        .unwrap();
        let revoked: SitesAuthorizedKeyResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/sites-authorized-keys/revoke",
                    Some(&revoke_body),
                )
                .unwrap(),
        );
        assert!(!revoked.active);

        let error = server
            .signed(
                &stranger_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/git-auth",
                Some(&auth_body),
            )
            .unwrap_err();
        assert!(matches!(error, ureq::Error::Status(403, _)));
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn project_grant_send_invite_emails_collaborator_and_replays() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        assert!(!created.dry_run);
        assert!(created.created);
        assert!(created.outputs[0].created);

        let grant_body = serde_json::to_vec(&ProjectGrantRequest {
            email: "skyler@example.com".to_string(),
            npub: None,
            role: "editor".to_string(),
        })
        .unwrap();
        let grant: ProjectGrantResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/grant?send_invites=true",
                    Some(&grant_body),
                )
                .unwrap(),
        );
        assert_eq!(grant.collaborator.email, "skyler@example.com");
        assert!(grant.collaborator.created);
        assert_eq!(grant.invited_emails, vec!["skyler@example.com"]);
        let bodies = outbox_bodies(&server.outbox);
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].contains("You've been invited to collaborate on finitechat-native"));
        assert!(bodies[0].contains("fsite auth redeem skyler@example.com"));
        assert!(bodies[0].contains(
            "fsite auth git finitechat-native --email skyler@example.com --store --output json"
        ));
        assert!(bodies[0].contains(&format!(
            "git clone http://git.{BASE_DOMAIN}:{}/finitechat-native.git",
            server.port()
        )));

        clear_outbox(&server.outbox);
        let replay: ProjectGrantResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/grant?send_invites=true",
                    Some(&grant_body),
                )
                .unwrap(),
        );
        assert!(!replay.collaborator.created);
        assert_eq!(replay.invited_emails, vec!["skyler@example.com"]);
        assert_eq!(outbox_bodies(&server.outbox).len(), 1);
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn project_collaborator_remove_revokes_git_credentials() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        json_body::<ProjectInitResponse>(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        let grant_body = serde_json::to_vec(&ProjectGrantRequest {
            email: "skyler@example.com".into(),
            npub: None,
            role: "editor".into(),
        })
        .unwrap();
        json_body::<ProjectGrantResponse>(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/grant",
                    Some(&grant_body),
                )
                .unwrap(),
        );

        let credential = mint_skyler_git_credential(&server);
        let remote = format!(
            "http://{}:{}@127.0.0.1:{}/finitechat-native.git",
            credential.username,
            credential.password,
            server.port()
        );
        let host_header = format!("Host: git.{BASE_DOMAIN}:{}", server.port());
        let dir = tempfile::tempdir().unwrap();
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "ls-remote",
                &remote,
            ],
            Some(dir.path()),
        );

        let remove_body = serde_json::to_vec(&ProjectRevokeRequest {
            email: "skyler@example.com".into(),
            npub: None,
        })
        .unwrap();
        let stranger_remove = server
            .signed(
                &stranger_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/revoke",
                Some(&remove_body),
            )
            .unwrap_err();
        assert!(matches!(stranger_remove, ureq::Error::Status(403, _)));

        let removed: ProjectRevokeResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/revoke",
                    Some(&remove_body),
                )
                .unwrap(),
        );
        assert_eq!(removed.project_slug, "finitechat-native");
        assert_eq!(removed.email, "skyler@example.com");
        assert!(removed.removed);
        assert_eq!(removed.revoked_git_credentials, 1);

        run_git_expect_failure(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "ls-remote",
                &remote,
            ],
            Some(dir.path()),
        );
        let auth_after_remove = server
            .signed(
                &stranger_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/git-auth",
                Some(
                    &serde_json::to_vec(&GitAuthRequest {
                        email: Some("skyler@example.com".into()),
                    })
                    .unwrap(),
                ),
            )
            .unwrap_err();
        assert!(matches!(auth_after_remove, ureq::Error::Status(403, _)));

        let replay: ProjectRevokeResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/revoke",
                    Some(&remove_body),
                )
                .unwrap(),
        );
        assert!(!replay.removed);
        assert_eq!(replay.revoked_git_credentials, 0);
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn native_project_collaborator_grant_and_revoke_use_own_identity() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let stranger_pubkey = finitesites_proto::event::pubkey_for_secret(&stranger_secret()).unwrap();
    let stranger_npub = finitesites_proto::npub::encode_npub(&stranger_pubkey).unwrap();
    let server = TestServer::start(&user_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        json_body::<ProjectInitResponse>(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        let grant_body = serde_json::to_vec(&ProjectGrantRequest {
            email: String::new(),
            npub: Some(stranger_npub.clone()),
            role: "editor".into(),
        })
        .unwrap();

        let invite_attempt = server
            .signed(
                &user_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/grant?send_invites=true",
                Some(&grant_body),
            )
            .unwrap_err();
        assert!(matches!(invite_attempt, ureq::Error::Status(400, _)));
        assert!(outbox_bodies(&server.outbox).is_empty());

        let grant: ProjectGrantResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/grant",
                    Some(&grant_body),
                )
                .unwrap(),
        );
        assert!(grant.collaborator.created);
        assert_eq!(grant.collaborator.email, "");
        assert_eq!(
            grant.collaborator.npub.as_deref(),
            Some(stranger_npub.as_str())
        );
        assert!(grant.invited_emails.is_empty());

        let replay: ProjectGrantResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/grant",
                    Some(&grant_body),
                )
                .unwrap(),
        );
        assert!(!replay.collaborator.created);

        let status: ProjectStatusResponse = json_body(
            server
                .signed(
                    &stranger_secret(),
                    "GET",
                    "/api/v1/projects/finitechat-native",
                    None,
                )
                .unwrap(),
        );
        assert_eq!(status.role, "editor");
        assert!(
            status
                .collaborators
                .iter()
                .any(|collaborator| collaborator.npub.as_deref() == Some(stranger_npub.as_str()))
        );

        let auth_body = serde_json::to_vec(&GitAuthRequest { email: None }).unwrap();
        let credential: GitAuthResponse = json_body(
            server
                .signed(
                    &stranger_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/git-auth",
                    Some(&auth_body),
                )
                .unwrap(),
        );
        let remote = format!(
            "http://{}:{}@127.0.0.1:{}/finitechat-native.git",
            credential.username,
            credential.password,
            server.port()
        );
        let host_header = format!("Host: git.{BASE_DOMAIN}:{}", server.port());
        let dir = tempfile::tempdir().unwrap();
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "ls-remote",
                &remote,
            ],
            Some(dir.path()),
        );

        let revoke_body = serde_json::to_vec(&ProjectRevokeRequest {
            email: String::new(),
            npub: Some(stranger_npub.clone()),
        })
        .unwrap();
        let removed: ProjectRevokeResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/revoke",
                    Some(&revoke_body),
                )
                .unwrap(),
        );
        assert!(removed.removed);
        assert_eq!(removed.email, "");
        assert_eq!(removed.npub.as_deref(), Some(stranger_npub.as_str()));
        assert_eq!(removed.revoked_git_credentials, 1);
        run_git_expect_failure(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "ls-remote",
                &remote,
            ],
            Some(dir.path()),
        );

        let auth_after_remove = server
            .signed(
                &stranger_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/git-auth",
                Some(&auth_body),
            )
            .unwrap_err();
        assert!(matches!(auth_after_remove, ureq::Error::Status(403, _)));

        let replay: ProjectRevokeResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/revoke",
                    Some(&revoke_body),
                )
                .unwrap(),
        );
        assert!(!replay.removed);
        assert_eq!(replay.revoked_git_credentials, 0);
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_api_and_git_origin_supports_clone_push_and_publish() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start_single_origin(&user_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let project: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        assert_eq!(
            project.git_remote_url,
            format!("{}/finitechat-native.git", server.api_url)
        );

        let auth_body = serde_json::to_vec(&GitAuthRequest { email: None }).unwrap();
        let credential: GitAuthResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/git-auth",
                    Some(&auth_body),
                )
                .unwrap(),
        );
        let authenticated_remote = format!(
            "http://{}:{}@127.0.0.1:{}/finitechat-native.git",
            credential.username,
            credential.password,
            server.port()
        );
        let checkout = tempfile::tempdir().unwrap();
        let repo = checkout.path().join("finitechat-native");
        run_git(
            &[
                "clone",
                &authenticated_remote,
                repo.to_str().expect("temp path is UTF-8"),
            ],
            Some(checkout.path()),
        );
        run_git(&["checkout", "-b", "main"], Some(&repo));
        std::fs::write(repo.join("finite.toml"), project.finite_toml).unwrap();
        std::fs::write(repo.join("index.html"), "<h1>single origin publish</h1>").unwrap();
        run_git(&["add", "finite.toml", "index.html"], Some(&repo));
        run_git(
            &[
                "-c",
                "user.email=agent@example.com",
                "-c",
                "user.name=Agent",
                "commit",
                "-m",
                "Publish through one origin",
            ],
            Some(&repo),
        );
        run_git(&["push", "origin", "main"], Some(&repo));

        let output = wait_for_active_version(&server, "finitechat-native-mockup", Some(1));
        assert_eq!(output.active_version, Some(1));
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn git_http_clone_and_push_with_minted_credential() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );

        let credential = mint_skyler_git_credential(&server);

        let dir = tempfile::tempdir().unwrap();
        let remote = format!(
            "http://{}:{}@127.0.0.1:{}/finitechat-native.git",
            credential.username,
            credential.password,
            server.port()
        );
        let host_header = format!("Host: git.{BASE_DOMAIN}:{}", server.port());
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "clone",
                &remote,
                "repo",
            ],
            Some(dir.path()),
        );
        let repo = dir.path().join("repo");
        run_git(&["checkout", "-b", "main"], Some(&repo));
        std::fs::write(repo.join("finite.toml"), created.finite_toml).unwrap();
        std::fs::write(repo.join("index.html"), "<h1>from git</h1>").unwrap();
        run_git(&["add", "finite.toml", "index.html"], Some(&repo));
        run_git(
            &[
                "-c",
                "user.email=skyler@example.com",
                "-c",
                "user.name=Skyler Bot",
                "commit",
                "-m",
                "Initial project output",
            ],
            Some(&repo),
        );
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "push",
                "origin",
                "main",
            ],
            Some(&repo),
        );

        let summary = wait_for_active_version(&server, "finitechat-native-mockup", Some(1));
        assert_eq!(summary.active_version, Some(1));

        let llms = server
            .site_get("finitechat-native-mockup", "/llms.txt", server.port())
            .unwrap()
            .into_string()
            .unwrap();
        assert!(llms.contains("Project: finitechat-native"));
        assert!(llms.contains(
            "fsite auth git finitechat-native --email YOUR_EDITOR_EMAIL --store --output json"
        ));
        assert!(llms.contains("fsite auth git finitechat-native --store --output json"));
        assert!(llms.contains("git push origin main"));
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn git_push_can_change_spa_fallback_per_version() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;
    let port = server.port();

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        let public_body = serde_json::to_vec(&SharingRequest {
            visibility: Some("public".into()),
            confirm_public: true,
            add_emails: vec![],
            remove_emails: vec![],
            add_npubs: vec![],
            remove_npubs: vec![],
        })
        .unwrap();
        server
            .signed(
                &user_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/outputs/mockup/sharing",
                Some(&public_body),
            )
            .unwrap();

        let credential = mint_skyler_git_credential(&server);
        push_project_files(
            &server,
            &credential,
            &created.finite_toml,
            "main",
            &[("index.html", "<h1>not an spa</h1>")],
            "Publish static version",
        );
        wait_for_active_version(&server, "finitechat-native-mockup", Some(1));
        let missing = server.site_get("finitechat-native-mockup", "/app/route", port);
        assert!(matches!(missing, Err(ureq::Error::Status(404, _))));

        let spa_config = created.finite_toml.replace("spa = false", "spa = true");
        push_project_files(
            &server,
            &credential,
            &spa_config,
            "main",
            &[("index.html", "<h1>spa shell</h1>")],
            "Enable SPA fallback",
        );
        wait_for_active_version(&server, "finitechat-native-mockup", Some(2));
        let fallback = server
            .site_get("finitechat-native-mockup", "/app/route", port)
            .unwrap();
        assert_eq!(fallback.into_string().unwrap(), "<h1>spa shell</h1>");

        push_project_files(
            &server,
            &credential,
            &created.finite_toml,
            "main",
            &[("index.html", "<h1>static again</h1>")],
            "Disable SPA fallback",
        );
        wait_for_active_version(&server, "finitechat-native-mockup", Some(3));
        let missing_again = server.site_get("finitechat-native-mockup", "/app/route", port);
        assert!(matches!(missing_again, Err(ureq::Error::Status(404, _))));
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn app_output_publishes_from_project_git_and_documents_runtime_contract() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;
    let port = server.port();

    let task = tokio::task::spawn_blocking(move || {
        let dry_body = serde_json::to_vec(&app_project_init_request(true)).unwrap();
        let dry_run: ProjectInitResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/init",
                    Some(&dry_body),
                )
                .unwrap(),
        );
        let dry_app = dry_run
            .outputs
            .iter()
            .find(|output| output.output_id == "web")
            .expect("dry run includes app output");
        assert_eq!(dry_app.kind, "app");
        assert_eq!(dry_app.site_name, "finitechat-native-app");
        assert_eq!(dry_app.start.as_deref(), Some("bun server.ts"));

        let body = serde_json::to_vec(&app_project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        assert!(created.finite_toml.contains("kind = \"app\""));
        assert!(created.finite_toml.contains("start = \"bun server.ts\""));

        let credential = mint_skyler_git_credential(&server);
        push_project_files(
            &server,
            &credential,
            &created.finite_toml,
            "main",
            &[
                (
                    "app/server.ts",
                    "Bun.serve({ hostname: '0.0.0.0', port: Number(process.env.PORT), fetch() { return new Response(process.env.DATA_DIR || 'missing'); } });\n",
                ),
                (
                    "app/.next/server.js",
                    "console.log('committed runtime payload');\n",
                ),
                ("app/.env.local", "SECRET=not-bundled\n"),
            ],
            "Publish app output",
        );

        let summary = wait_for_active_version(&server, "finitechat-native-app", Some(1));
        assert_eq!(summary.kind, "app");
        assert_eq!(summary.start.as_deref(), Some("bun server.ts"));
        assert_eq!(summary.path, "app");

        let llms = server
            .site_get("finitechat-native-app", "/llms.txt", port)
            .unwrap()
            .into_string()
            .unwrap();
        assert!(llms.contains("Output kind: app"));
        assert!(llms.contains("This is a stateful app output."));
        assert!(llms.contains("Finite Sites sets PORT and DATA_DIR"));
        assert!(llms.contains("listen on 0.0.0.0:$PORT"));
        assert!(llms.contains("write live mutable state only under DATA_DIR"));
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn document_output_renders_markdown_and_agent_companion_paths() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;
    let port = server.port();

    let task = tokio::task::spawn_blocking(move || {
        let dry_body = serde_json::to_vec(&site_and_document_project_init_request(true)).unwrap();
        let dry_run: ProjectInitResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/init",
                    Some(&dry_body),
                )
                .unwrap(),
        );
        let dry_doc = dry_run
            .outputs
            .iter()
            .find(|output| output.output_id == "doc")
            .expect("dry run includes document output");
        assert_eq!(dry_doc.kind, "document");
        assert_eq!(
            dry_doc.document_name.as_deref(),
            Some("finitechat-native-docs")
        );
        assert!(dry_doc.output_url.contains(".docs.sites.localhost:"));

        let body = serde_json::to_vec(&site_and_document_project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        assert!(created.finite_toml.contains("kind = \"document\""));
        assert!(
            created
                .finite_toml
                .contains("document_name = \"finitechat-native-docs\"")
        );

        let credential = mint_skyler_git_credential(&server);
        push_project_files(
            &server,
            &credential,
            &created.finite_toml,
            "main",
            &[
                ("site/index.html", "<h1>site bytes</h1>"),
                (
                    "docs/index.md",
                    "---\ntitle: Hermes Notes\n---\n# Hermes Notes\n\nSee [[guide|the guide]].\n\n<div>raw</div>\n",
                ),
                (
                    "docs/guide.md",
                    "# Guide\n\nA second authored Markdown page.\n",
                ),
            ],
            "Publish site and document outputs",
        );

        let site_summary = project_output_status(&server, "finitechat-native-mockup");
        assert_eq!(site_summary.kind, "site");
        assert_eq!(
            site_summary.active_version,
            Some(1),
            "a successful git push must return after its version is active"
        );
        let doc_summary = project_output_status(&server, "finitechat-native-docs");
        assert_eq!(doc_summary.kind, "document");
        assert_eq!(
            doc_summary.active_version,
            Some(1),
            "a successful git push must activate every matching output"
        );
        assert_eq!(doc_summary.output_name, "finitechat-native-docs");
        assert_eq!(doc_summary.entry.as_deref(), Some("index.md"));

        // Private documents redirect to the Auth Gate like every output.
        let private = server
            .document_get("finitechat-native-docs", "/", port)
            .unwrap();
        assert_eq!(private.status(), 303);
        assert!(
            private
                .header("location")
                .unwrap()
                .starts_with(&format!("{AUTH_GATE_URL}/authorize?output="))
        );

        let share_body = serde_json::to_vec(&SharingRequest {
            visibility: Some("public".into()),
            confirm_public: true,
            add_emails: vec![],
            remove_emails: vec![],
            add_npubs: vec![],
            remove_npubs: vec![],
        })
        .unwrap();
        let shared: ProjectOutputSharingResponse = json_body(
            server
                .signed(
                    &user_secret(),
                    "POST",
                    "/api/v1/projects/finitechat-native/outputs/doc/sharing",
                    Some(&share_body),
                )
                .unwrap(),
        );
        assert_eq!(shared.output_id, "doc");
        assert_eq!(shared.visibility, "public");

        let rendered = server
            .document_get("finitechat-native-docs", "/", port)
            .unwrap();
        assert_eq!(
            rendered.header("content-type").unwrap(),
            "text/html; charset=utf-8"
        );
        assert_eq!(rendered.header("cache-control"), Some("no-store"));
        let first_etag = rendered.header("etag").unwrap().to_string();
        let rendered = rendered.into_string().unwrap();
        assert!(rendered.contains("<h1>Hermes Notes</h1>"));
        assert!(rendered.contains("href=\"/guide\""));
        assert!(rendered.contains("&lt;div&gt;raw&lt;/div&gt;"));
        assert!(rendered.contains("href=\"/llms.txt\""));
        assert!(rendered.contains("href=\"/llms-full.txt\""));
        assert!(rendered.contains("href=\"/index.md\""));

        let guide = server
            .document_get("finitechat-native-docs", "/guide", port)
            .unwrap()
            .into_string()
            .unwrap();
        assert!(guide.contains("<h1>Guide</h1>"));

        let raw = server
            .document_get("finitechat-native-docs", "/guide.md", port)
            .unwrap();
        assert_eq!(
            raw.header("content-type").unwrap(),
            "text/markdown; charset=utf-8"
        );
        assert_eq!(
            raw.into_string().unwrap(),
            "# Guide\n\nA second authored Markdown page.\n"
        );

        let llms = server
            .document_get("finitechat-native-docs", "/llms.txt", port)
            .unwrap()
            .into_string()
            .unwrap();
        assert!(llms.contains("Output name: finitechat-native-docs"));
        assert!(llms.contains("Output URL: http://finitechat-native-docs.docs.sites.localhost:"));
        assert!(llms.contains("For document outputs, commit authored Markdown"));

        let full = server
            .document_get("finitechat-native-docs", "/llms-full.txt", port)
            .unwrap()
            .into_string()
            .unwrap();
        assert!(full.contains("## /index.md"));
        assert!(full.contains("## /guide.md"));
        assert!(full.contains("# Hermes Notes"));
        assert!(full.contains("# Guide"));

        push_project_files(
            &server,
            &credential,
            &created.finite_toml,
            "main",
            &[(
                "docs/guide.md",
                "# Better Guide\n\nA second authored Markdown page.\n",
            )],
            "Rename sibling document page",
        );
        assert_eq!(
            project_output_status(&server, "finitechat-native-docs").active_version,
            Some(2),
            "push completion must include activation of the new document version"
        );

        let revalidated = server
            .agent
            .get(&format!(
                "http://finitechat-native-docs.{}:{port}/",
                document_base_domain()
            ))
            .set("If-None-Match", &first_etag)
            .call()
            .unwrap();
        assert_eq!(revalidated.status(), 200);
        let second_etag = revalidated.header("etag").unwrap().to_string();
        assert_ne!(second_etag, first_etag);
        let revalidated = revalidated.into_string().unwrap();
        assert!(revalidated.contains(">Better Guide</a>"));
        assert!(!revalidated.contains(">Guide</a>"));
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn git_ref_event_reconciles_after_restart_boundary() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start_with_git_auto_reconcile(&user_pubkey, false).await;

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );

        let credential = mint_skyler_git_credential(&server);

        let dir = tempfile::tempdir().unwrap();
        let remote = format!(
            "http://{}:{}@127.0.0.1:{}/finitechat-native.git",
            credential.username,
            credential.password,
            server.port()
        );
        let host_header = format!("Host: git.{BASE_DOMAIN}:{}", server.port());
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "clone",
                &remote,
                "repo",
            ],
            Some(dir.path()),
        );
        let repo = dir.path().join("repo");
        run_git(&["checkout", "-b", "main"], Some(&repo));
        std::fs::write(repo.join("finite.toml"), created.finite_toml).unwrap();
        std::fs::write(repo.join("index.html"), "<h1>after restart</h1>").unwrap();
        run_git(&["add", "finite.toml", "index.html"], Some(&repo));
        run_git(
            &[
                "-c",
                "user.email=skyler@example.com",
                "-c",
                "user.name=Skyler Bot",
                "commit",
                "-m",
                "Durable hook event",
            ],
            Some(&repo),
        );
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "push",
                "origin",
                "main",
            ],
            Some(&repo),
        );

        let summary = project_output_status(&server, "finitechat-native-mockup");
        assert_eq!(summary.active_version, None);

        let data_dir = server.data_dir().to_path_buf();
        {
            let store = Store::open(&data_dir.join("registry.db")).unwrap();
            let pending = store.pending_git_ref_events(None).unwrap();
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].ref_name, "refs/heads/main");
        }

        let store = Store::open(&data_dir.join("registry.db")).unwrap();
        let blobs = BlobStore::open(&data_dir.join("blobs")).unwrap();
        let mut engine = Engine::new(
            store,
            blobs,
            [9u8; 32],
            EngineConfig {
                base_domain: BASE_DOMAIN.to_string(),
                document_base_domain: document_base_domain(),
                site_url_scheme: "http".to_string(),
                site_url_port: Some(server.port()),
            },
        );
        let processed =
            finitesitesd::git::reconcile_pending_events(&mut engine, &data_dir, None, now_unix())
                .unwrap();
        assert_eq!(processed, 1);
        let replay =
            finitesitesd::git::reconcile_pending_events(&mut engine, &data_dir, None, now_unix())
                .unwrap();
        assert_eq!(replay, 0);

        let summary = wait_for_active_version(&server, "finitechat-native-mockup", Some(1));
        assert_eq!(summary.active_version, Some(1));
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn git_push_to_non_deploy_branch_does_not_publish() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );

        let credential = mint_skyler_git_credential(&server);

        let dir = tempfile::tempdir().unwrap();
        let remote = format!(
            "http://{}:{}@127.0.0.1:{}/finitechat-native.git",
            credential.username,
            credential.password,
            server.port()
        );
        let host_header = format!("Host: git.{BASE_DOMAIN}:{}", server.port());
        let bad_remote = format!(
            "http://{}:{}@127.0.0.1:{}/finitechat-native.git",
            credential.username,
            "badpassword",
            server.port()
        );
        run_git_expect_failure(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "ls-remote",
                &bad_remote,
            ],
            Some(dir.path()),
        );
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "clone",
                &remote,
                "repo",
            ],
            Some(dir.path()),
        );
        let repo = dir.path().join("repo");
        run_git(&["checkout", "-b", "notes"], Some(&repo));
        std::fs::write(repo.join("finite.toml"), created.finite_toml).unwrap();
        std::fs::write(repo.join("index.html"), "<h1>not deployed</h1>").unwrap();
        run_git(&["add", "finite.toml", "index.html"], Some(&repo));
        run_git(
            &[
                "-c",
                "user.email=skyler@example.com",
                "-c",
                "user.name=Skyler Bot",
                "commit",
                "-m",
                "Push non deploy branch",
            ],
            Some(&repo),
        );
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "push",
                "origin",
                "notes",
            ],
            Some(&repo),
        );

        let summary = project_output_status(&server, "finitechat-native-mockup");
        assert_eq!(summary.active_version, None);
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn static_publish_revalidates_to_new_active_bytes_when_push_returns() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;
    let port = server.port();

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        let public_body = serde_json::to_vec(&SharingRequest {
            visibility: Some("public".into()),
            confirm_public: true,
            add_emails: vec![],
            remove_emails: vec![],
            add_npubs: vec![],
            remove_npubs: vec![],
        })
        .unwrap();
        server
            .signed(
                &user_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/outputs/mockup/sharing",
                Some(&public_body),
            )
            .unwrap();

        let credential = mint_skyler_git_credential(&server);
        push_project_files(
            &server,
            &credential,
            &created.finite_toml,
            "main",
            &[
                ("index.html", "<h1>version one</h1>"),
                ("app.js", "console.log('version one')"),
            ],
            "Publish version one",
        );
        assert_eq!(
            project_output_status(&server, "finitechat-native-mockup").active_version,
            Some(1)
        );
        let first = server
            .site_get("finitechat-native-mockup", "/", port)
            .unwrap();
        let first_etag = first.header("etag").unwrap().to_string();
        assert_eq!(first.header("cache-control"), Some("no-store"));
        assert_eq!(first.into_string().unwrap(), "<h1>version one</h1>");
        let first_asset = server
            .site_get("finitechat-native-mockup", "/app.js", port)
            .unwrap();
        assert_eq!(first_asset.header("cache-control"), Some("no-store"));
        let first_asset_etag = first_asset.header("etag").unwrap().to_string();
        assert_eq!(
            first_asset.into_string().unwrap(),
            "console.log('version one')"
        );

        push_project_files(
            &server,
            &credential,
            &created.finite_toml,
            "main",
            &[
                ("index.html", "<h1>version two</h1>"),
                ("app.js", "console.log('version two')"),
            ],
            "Publish version two",
        );
        assert_eq!(
            project_output_status(&server, "finitechat-native-mockup").active_version,
            Some(2)
        );
        let revalidated = server
            .agent
            .get(&format!(
                "http://finitechat-native-mockup.{BASE_DOMAIN}:{port}/"
            ))
            .set("If-None-Match", &first_etag)
            .call()
            .unwrap();
        assert_eq!(revalidated.status(), 200);
        assert_ne!(revalidated.header("etag"), Some(first_etag.as_str()));
        assert_eq!(revalidated.into_string().unwrap(), "<h1>version two</h1>");
        let revalidated_asset = server
            .agent
            .get(&format!(
                "http://finitechat-native-mockup.{BASE_DOMAIN}:{port}/app.js"
            ))
            .set("If-None-Match", &first_asset_etag)
            .call()
            .unwrap();
        assert_eq!(revalidated_asset.status(), 200);
        assert_eq!(revalidated_asset.header("cache-control"), Some("no-store"));
        assert_ne!(
            revalidated_asset.header("etag"),
            Some(first_asset_etag.as_str())
        );
        assert_eq!(
            revalidated_asset.into_string().unwrap(),
            "console.log('version two')"
        );
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_deploy_reports_failure_after_accepting_the_git_ref() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );

        let credential = mint_skyler_git_credential(&server);

        let dir = tempfile::tempdir().unwrap();
        let remote = format!(
            "http://{}:{}@127.0.0.1:{}/finitechat-native.git",
            credential.username,
            credential.password,
            server.port()
        );
        let host_header = format!("Host: git.{BASE_DOMAIN}:{}", server.port());
        run_git(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "clone",
                &remote,
                "repo",
            ],
            Some(dir.path()),
        );
        let repo = dir.path().join("repo");
        run_git(&["checkout", "-b", "main"], Some(&repo));
        let bad_config = created
            .finite_toml
            .replace("path = \".\"", "path = \"dist\"");
        std::fs::write(repo.join("finite.toml"), bad_config).unwrap();
        std::fs::write(repo.join("index.html"), "<h1>not deployed</h1>").unwrap();
        run_git(&["add", "finite.toml", "index.html"], Some(&repo));
        run_git(
            &[
                "-c",
                "user.email=skyler@example.com",
                "-c",
                "user.name=Skyler Bot",
                "commit",
                "-m",
                "Missing output path",
            ],
            Some(&repo),
        );
        let local_head =
            String::from_utf8(run_git_capture(&["rev-parse", "HEAD"], Some(&repo)).stdout).unwrap();
        run_git_expect_failure(
            &[
                "-c",
                &format!("http.extraHeader={host_header}"),
                "push",
                "origin",
                "main",
            ],
            Some(&repo),
        );
        let remote_ref = String::from_utf8(
            run_git_capture(
                &[
                    "-c",
                    &format!("http.extraHeader={host_header}"),
                    "ls-remote",
                    "origin",
                    "refs/heads/main",
                ],
                Some(&repo),
            )
            .stdout,
        )
        .unwrap();
        assert!(
            remote_ref.starts_with(local_head.trim()),
            "post-receive deploy failure is reported after Git durably accepts the ref"
        );

        let summary = project_output_status(&server, "finitechat-native-mockup");
        assert_eq!(summary.active_version, None);
    });
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn generated_llms_txt_requires_project_output_and_respects_user_file() {
    let user_pubkey = finitesites_proto::event::pubkey_for_secret(&user_secret()).unwrap();
    let server = TestServer::start(&user_pubkey).await;
    let port = server.port();

    let task = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&project_init_request(false)).unwrap();
        let created: ProjectInitResponse = json_body(
            server
                .signed(&user_secret(), "POST", "/api/v1/projects/init", Some(&body))
                .unwrap(),
        );
        let credential = mint_skyler_git_credential(&server);
        push_project_files(
            &server,
            &credential,
            &created.finite_toml,
            "main",
            &[("index.html", "<h1>v1</h1>")],
            "Initial agent docs deploy",
        );
        wait_for_active_version(&server, "finitechat-native-mockup", Some(1));

        let generated = server
            .site_get("finitechat-native-mockup", "/llms.txt", port)
            .unwrap()
            .into_string()
            .unwrap();
        assert!(generated.contains("Project: finitechat-native"));
        assert!(generated.contains("fsite auth git finitechat-native"));

        let public_body = serde_json::to_vec(&SharingRequest {
            visibility: Some("public".into()),
            confirm_public: true,
            add_emails: vec![],
            remove_emails: vec![],
            add_npubs: vec![],
            remove_npubs: vec![],
        })
        .unwrap();
        server
            .signed(
                &user_secret(),
                "POST",
                "/api/v1/projects/finitechat-native/outputs/mockup/sharing",
                Some(&public_body),
            )
            .unwrap();

        push_project_files(
            &server,
            &credential,
            &created.finite_toml,
            "main",
            &[
                ("index.html", "<h1>v2</h1>"),
                ("llms.txt", "custom project instructions"),
            ],
            "Author llms instructions",
        );
        wait_for_active_version(&server, "finitechat-native-mockup", Some(2));
        let custom = server
            .site_get("finitechat-native-mockup", "/llms.txt", port)
            .unwrap();
        assert_eq!(custom.into_string().unwrap(), "custom project instructions");
    });
    task.await.unwrap();
}
