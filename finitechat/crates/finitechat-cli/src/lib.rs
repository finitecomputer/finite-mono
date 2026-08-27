use std::io::Write;

mod app;
mod auth;
mod capture;
mod cli;
mod diagnose;
mod hermes;
mod repair;

use clap::Parser;
use finitechat_delivery::{HttpKeyPackageId, HttpKeyPackagePublication};
use finitechat_http::{
    AckWelcomeRequest, ApplicationEffectRequest, BootstrapAccountRoomRequest,
    ClaimKeyPackageRequest, ClaimKeyPackagesRequest, ClaimWelcomesRequest,
    ExpireKeyPackageLeaseRequest, GetDeviceLivenessRequest, GroupSyncRequest, InboxSyncRequest,
    KeyPackageInventoryRequest, LeaveRoomRequest, ListAccountRoomDirectoryRequest,
    ObserveDeviceLivenessRequest, ReportInvalidCommitRequest, RevokeDeviceRequest,
    SaveAccountRoomRequest, UpdateRoomAdminsRequest,
};
use finitechat_mls::{NOSTR_SECRET_KEY_BYTES, NostrSecretKey};
use finitechat_proto::{DeviceRef, RoomProtocol};
use finitechat_transport::engine::KeyPackage;
use finitechat_transport::{GroupId, MemberId, MessageId};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

pub(crate) const DEFAULT_SERVER_URL: &str = "https://chat.finite.computer";
pub(crate) const DEFAULT_SYNC_LIMIT: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedHttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub json: Option<Value>,
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error("failed to serialize request: {0}")]
    Serialize(serde_json::Error),
    #[error("failed to parse JSON: {0}")]
    Json(serde_json::Error),
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("server returned {status}: {body}")]
    Server {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("failed to write output: {0}")]
    Output(std::io::Error),
    #[error("hermes: {0}")]
    Hermes(String),
    #[error("identity: {0}")]
    Identity(String),
    #[error("runtime: {0}")]
    Runtime(String),
}

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => 2,
            Self::Serialize(_)
            | Self::Json(_)
            | Self::Http(_)
            | Self::Server { .. }
            | Self::Output(_)
            | Self::Hermes(_)
            | Self::Identity(_)
            | Self::Runtime(_) => 1,
        }
    }

    /// Stable machine-readable class of this error. Shared by the Hermes
    /// service error body and the `--json` CLI stderr line so both transports
    /// report the same classification.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Usage(_) => "usage",
            Self::Serialize(_) => "serialize",
            Self::Json(_) => "json",
            Self::Http(_) => "http",
            Self::Server { .. } => "server",
            Self::Output(_) => "output",
            Self::Hermes(_) => "hermes",
            Self::Identity(_) => "identity",
            Self::Runtime(_) => "runtime",
        }
    }

    /// Whether a caller may retry the same request unchanged. Decided by the
    /// error class, never by the human-readable message.
    pub fn retryable(&self) -> bool {
        match self {
            Self::Http(_) => true,
            Self::Server { status, .. } => {
                status.is_server_error()
                    || *status == reqwest::StatusCode::REQUEST_TIMEOUT
                    || *status == reqwest::StatusCode::TOO_MANY_REQUESTS
            }
            Self::Usage(_)
            | Self::Serialize(_)
            | Self::Json(_)
            | Self::Output(_)
            | Self::Hermes(_)
            | Self::Identity(_)
            | Self::Runtime(_) => false,
        }
    }

    /// One-line JSON form printed to stderr when `--json` was requested, so a
    /// machine caller of the CLI sees the same `error_kind` / `retryable`
    /// fields the resident service returns in its HTTP error body.
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "ok": false,
            "status": "error",
            "error_kind": self.kind(),
            "retryable": self.retryable(),
            "error": self.to_string(),
        })
    }
}

/// `--json` is a global flag on the subcommands that support it; an error
/// raised before or during parsing still honours it, so the check is on the
/// raw arguments rather than the parsed command.
pub fn json_errors_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().any(|arg| arg.as_ref() == "--json")
}

pub fn run<I, S, W>(args: I, output: &mut W) -> Result<(), CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
    W: Write,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let cli = match cli::Cli::try_parse_from(std::iter::once("finitechat".to_owned()).chain(args)) {
        Ok(cli) => cli,
        Err(error) => return report_clap_error(&error, output),
    };
    match cli.command {
        // Success paths so `finitechat --version` works as an install check
        // and `finitechat --help` self-describes for agents (exit 0, stdout);
        // `finitechat version` matches the historical subcommand form.
        cli::Command::Version => {
            writeln!(output, "finitechat {}", env!("CARGO_PKG_VERSION")).map_err(CliError::Output)
        }
        cli::Command::HttpSmoke => {
            let ids = finitechat_delivery::prove_http_delivery_core_orders_commit_then_message()
                .expect("HTTP delivery core smoke passes");
            writeln!(
                output,
                "ordered {} messages through the Finite Chat HTTP delivery core",
                ids.len()
            )
            .map_err(CliError::Output)
        }
        cli::Command::Http(args) => {
            let request = build_http_request(&args.server, &args.command)?;
            execute_http_request(&request, output)
        }
        cli::Command::Auth(args) => auth::run(args.command, output),
        cli::Command::App(args) => app::run(args, output),
        cli::Command::Capture(args) => capture::run(args, output),
        cli::Command::Diagnose(args) => diagnose::run(args, output),
        cli::Command::Repair(args) => repair::run(args, output),
        cli::Command::Hermes(args) => hermes::run(args, output),
    }
}

/// clap renders `--help`/`--version` as "errors" that must succeed through
/// the writer (exit 0, stdout); every other parse failure is a usage error
/// (exit 2, stderr via `CliError::Usage`).
fn report_clap_error<W: Write>(error: &clap::Error, output: &mut W) -> Result<(), CliError> {
    match error.kind() {
        clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
            write!(output, "{}", error.render()).map_err(CliError::Output)
        }
        _ => Err(CliError::Usage(error.render().to_string())),
    }
}

pub fn prepare_http_request<I, S>(args: I) -> Result<PreparedHttpRequest, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let parsed =
        cli::HttpArgs::try_parse_from(std::iter::once("http".to_owned()).chain(args.into_iter()))
            .map_err(|error| CliError::Usage(error.render().to_string()))?;
    build_http_request(&parsed.server, &parsed.command)
}

fn raw_delivery_owner_from_cli(owner: String) -> Result<MemberId, CliError> {
    if serde_json::from_str::<DeviceRef>(&owner).is_ok() {
        return Err(CliError::Usage(
            "--owner is a raw delivery MemberId, not DeviceRef JSON; use finitechat-client runtime delivery for Finite device KeyPackages".to_owned(),
        ));
    }
    Ok(MemberId::new(owner.into_bytes()))
}

fn build_http_request(
    server: &str,
    command: &cli::HttpCommand,
) -> Result<PreparedHttpRequest, CliError> {
    use cli::HttpCommand as Http;

    match command {
        Http::Health => Ok(PreparedHttpRequest {
            method: HttpMethod::Get,
            url: route_url(server, "/health"),
            json: None,
        }),
        Http::SubmitCommit { request_json } => {
            request_json_passthrough(server, "/commits", request_json)
        }
        Http::AppendEvent { request_json } => {
            request_json_passthrough(server, "/events", request_json)
        }
        Http::ApplicationEffectGet { message_id } => post_json_request(
            server,
            "/application-effects/get",
            &ApplicationEffectRequest {
                message_id: message_id.clone(),
            },
        ),
        Http::ApplicationEffectCounts => post_json_request(
            server,
            "/application-effects/counts",
            &serde_json::json!({}),
        ),
        Http::AppendActivity { request_json } => {
            request_json_passthrough(server, "/activities", request_json)
        }
        Http::SyncGroup {
            group_id,
            after_seq,
            limit,
            requester,
        } => post_json_request(
            server,
            "/sync/group",
            &GroupSyncRequest {
                group_id: GroupId::new(group_id.clone().into_bytes()),
                after_seq: *after_seq,
                limit: *limit,
                requester: requester
                    .clone()
                    .map(|requester| MemberId::new(requester.into_bytes())),
            },
        ),
        Http::SyncInbox {
            recipient,
            after_seq,
            limit,
        } => post_json_request(
            server,
            "/sync/inbox",
            &InboxSyncRequest {
                recipient: MemberId::new(recipient.clone().into_bytes()),
                after_seq: *after_seq,
                limit: *limit,
            },
        ),
        Http::RevokeDevice {
            account_id,
            device_id,
        } => post_json_request(
            server,
            "/devices/revoke",
            &RevokeDeviceRequest {
                device: DeviceRef {
                    account_id: account_id.clone(),
                    device_id: device_id.clone(),
                },
            },
        ),
        Http::ObserveDeviceLiveness {
            account_id,
            device_id,
            observed_at_ms,
            expires_at_ms,
        } => post_json_request(
            server,
            "/devices/liveness",
            &ObserveDeviceLivenessRequest {
                device: DeviceRef {
                    account_id: account_id.clone(),
                    device_id: device_id.clone(),
                },
                observed_at_ms: *observed_at_ms,
                expires_at_ms: *expires_at_ms,
            },
        ),
        Http::GetDeviceLiveness {
            account_id,
            device_id,
            now_ms,
        } => post_json_request(
            server,
            "/devices/liveness/get",
            &GetDeviceLivenessRequest {
                device: DeviceRef {
                    account_id: account_id.clone(),
                    device_id: device_id.clone(),
                },
                now_ms: *now_ms,
            },
        ),
        Http::PublishKeyPackage {
            owner,
            key_package_id,
            bytes,
        } => post_json_request(
            server,
            "/key-packages",
            &HttpKeyPackagePublication {
                key_package_id: HttpKeyPackageId::new(key_package_id.clone().into_bytes()),
                owner: raw_delivery_owner_from_cli(owner.clone())?,
                key_package: KeyPackage::new(bytes.clone().into_bytes()),
            },
        ),
        Http::KeyPackageInventory { owner } => post_json_request(
            server,
            "/key-packages/inventory",
            &KeyPackageInventoryRequest {
                owner: raw_delivery_owner_from_cli(owner.clone())?,
            },
        ),
        Http::ClaimKeyPackage { owner } => post_json_request(
            server,
            "/key-packages/claim",
            &ClaimKeyPackageRequest {
                owner: raw_delivery_owner_from_cli(owner.clone())?,
            },
        ),
        Http::ClaimKeyPackages {
            owners,
            idempotency_key,
        } => post_json_request(
            server,
            "/key-packages/claims",
            &ClaimKeyPackagesRequest {
                owners: owners
                    .iter()
                    .map(|owner| raw_delivery_owner_from_cli(owner.clone()))
                    .collect::<Result<Vec<_>, _>>()?,
                idempotency_key: idempotency_key.clone(),
            },
        ),
        Http::ExpireKeyPackageLease { key_package_id } => post_json_request(
            server,
            "/key-packages/leases/expire",
            &ExpireKeyPackageLeaseRequest {
                key_package_id: HttpKeyPackageId::new(key_package_id.clone().into_bytes()),
            },
        ),
        Http::AccountRoomSave {
            account_id,
            room_id,
            record_json,
        } => post_json_request(
            server,
            "/account-rooms",
            &SaveAccountRoomRequest {
                account_id: account_id.clone(),
                room_id: room_id.clone(),
                record: serde_json::from_str(record_json).map_err(CliError::Json)?,
            },
        ),
        Http::AccountRoomBootstrap {
            room_id,
            mls_group_id,
            account_id,
            device_id,
        } => post_json_request(
            server,
            "/account-rooms/bootstrap",
            &BootstrapAccountRoomRequest {
                room_id: room_id.clone(),
                mls_group_id: mls_group_id.clone(),
                creator: DeviceRef {
                    account_id: account_id.clone(),
                    device_id: device_id.clone(),
                },
                protocol: RoomProtocol::default(),
            },
        ),
        Http::AccountRoomsList {
            account_id,
            after_room_id,
            limit,
        } => post_json_request(
            server,
            "/account-rooms/list",
            &ListAccountRoomDirectoryRequest {
                account_id: account_id.clone(),
                after_room_id: after_room_id.clone(),
                limit: *limit,
            },
        ),
        Http::RoomLeave {
            room_id,
            account_id,
            device_id,
        } => post_json_request(
            server,
            "/rooms/leave",
            &LeaveRoomRequest {
                room_id: room_id.clone(),
                sender: DeviceRef {
                    account_id: account_id.clone(),
                    device_id: device_id.clone(),
                },
            },
        ),
        Http::RoomAdmins {
            room_id,
            account_id,
            device_id,
            grant,
            revoke,
        } => post_json_request(
            server,
            "/rooms/admins",
            &UpdateRoomAdminsRequest {
                room_id: room_id.clone(),
                sender: DeviceRef {
                    account_id: account_id.clone(),
                    device_id: device_id.clone(),
                },
                grant: grant.clone(),
                revoke: revoke.clone(),
            },
        ),
        Http::ReportInvalidCommit {
            room_id,
            account_id,
            device_id,
            offending_seq,
        } => post_json_request(
            server,
            "/rooms/report-invalid-commit",
            &ReportInvalidCommitRequest {
                room_id: room_id.clone(),
                reporter: DeviceRef {
                    account_id: account_id.clone(),
                    device_id: device_id.clone(),
                },
                offending_seq: *offending_seq,
            },
        ),
        Http::ClaimWelcomes { recipient, limit } => post_json_request(
            server,
            "/welcomes/claim",
            &ClaimWelcomesRequest {
                recipient: MemberId::new(recipient.clone().into_bytes()),
                limit: *limit,
            },
        ),
        Http::AckWelcome { message_id } => post_json_request(
            server,
            "/welcomes/ack",
            &AckWelcomeRequest {
                message_id: MessageId::new(message_id.clone().into_bytes()),
            },
        ),
    }
}

fn request_json_passthrough(
    server: &str,
    path: &str,
    request_json: &str,
) -> Result<PreparedHttpRequest, CliError> {
    let request: Value = serde_json::from_str(request_json).map_err(CliError::Json)?;
    post_json_request(server, path, &request)
}

fn post_json_request<T: Serialize>(
    server: &str,
    path: &str,
    body: &T,
) -> Result<PreparedHttpRequest, CliError> {
    Ok(PreparedHttpRequest {
        method: HttpMethod::Post,
        url: route_url(server, path),
        json: Some(serde_json::to_value(body).map_err(CliError::Serialize)?),
    })
}

fn execute_http_request<W: Write>(
    request: &PreparedHttpRequest,
    output: &mut W,
) -> Result<(), CliError> {
    let client = reqwest::blocking::Client::new();
    let builder = match request.method {
        HttpMethod::Get => client.get(&request.url),
        HttpMethod::Post => client
            .post(&request.url)
            .json(request.json.as_ref().expect("POST request has JSON body")),
    };
    let response = builder.send()?;
    let status = response.status();
    let body = response.text()?;
    if !status.is_success() {
        return Err(CliError::Server { status, body });
    }
    writeln!(output, "{body}").map_err(CliError::Output)
}

pub(crate) fn write_pretty_json<T: Serialize, W: Write>(
    output: &mut W,
    value: &T,
) -> Result<(), CliError> {
    serde_json::to_writer_pretty(&mut *output, value).map_err(CliError::Serialize)?;
    writeln!(output).map_err(CliError::Output)
}

fn route_url(server: &str, path: &str) -> String {
    format!(
        "{}/{}",
        server.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Parse a 64-character lowercase hex account secret into a Nostr secret
/// key. Callers read the hex from a file, never from a CLI argument.
pub(crate) fn parse_account_secret(hex: &str) -> Result<NostrSecretKey, CliError> {
    let invalid = || {
        CliError::Usage(
            "the account secret must be 64 lowercase hex characters (32 bytes)".to_owned(),
        )
    };
    if hex.len() != NOSTR_SECRET_KEY_BYTES * 2 {
        return Err(invalid());
    }
    let mut bytes = [0u8; NOSTR_SECRET_KEY_BYTES];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).map_err(|_| invalid())?;
    }
    NostrSecretKey::from_bytes(bytes).map_err(|_| invalid())
}

/// Point `FINITE_HOME` at a process-wide throwaway directory so tests never
/// mint or read the developer's real shared identity. Set once per process;
/// every in-process test that can reach identity resolution calls this first.
#[cfg(test)]
pub(crate) fn ensure_test_finite_home() -> std::path::PathBuf {
    use std::sync::OnceLock;
    static HOME: OnceLock<std::path::PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let dir = tempfile::tempdir().expect("test FINITE_HOME tempdir");
        let path = dir.path().to_path_buf();
        // Keep the directory alive for the whole test process.
        std::mem::forget(dir);
        // SAFETY: set exactly once, before any identity resolution in this
        // process; tests that resolve identity call this helper first.
        unsafe { std::env::set_var("FINITE_HOME", &path) };
        path
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use finitechat_client::{
        FiniteChatDevice, FiniteChatDeviceConfig, HttpRuntimeDelivery, ReqwestHttpRuntimeTransport,
        RuntimeDelivery,
    };
    use finitechat_delivery::HttpSyncPage;
    use finitechat_http::{
        AckWelcomeRequest, ApplicationEffectRequest, BootstrapAccountRoomRequest,
        ClaimKeyPackagesRequest, ClaimWelcomesRequest, ExpireKeyPackageLeaseRequest,
        GetDeviceLivenessRequest, GroupSyncRequest, HttpKeyPackageClaim,
        KeyPackageInventoryRequest, ListAccountRoomDirectoryRequest, ObserveDeviceLivenessRequest,
        PublishKeyPackageResponse, ReportInvalidCommitRequest, RevokeDeviceRequest,
        SaveAccountRoomRequest,
    };
    use finitechat_mls::{NOSTR_SECRET_KEY_BYTES, NostrSecretKey};
    use finitechat_proto::{CommitAccepted, WelcomeState};

    const CLI_LIVE_ALICE_SECRET: [u8; NOSTR_SECRET_KEY_BYTES] = [71; NOSTR_SECRET_KEY_BYTES];

    #[test]
    fn sync_group_command_defaults_cursor_and_limit() {
        let request =
            prepare_http_request(["sync-group", "--group-id", "room-a"]).expect("prepared request");

        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, "https://chat.finite.computer/sync/group");
        let body: GroupSyncRequest =
            serde_json::from_value(request.json.expect("json")).expect("sync request");
        assert_eq!(body.group_id.as_slice(), b"room-a");
        assert_eq!(body.after_seq, 0);
        assert_eq!(body.limit, DEFAULT_SYNC_LIMIT);
        assert!(body.requester.is_none());
    }

    #[test]
    fn sync_group_command_accepts_requester() {
        let request = prepare_http_request([
            "sync-group",
            "--group-id",
            "room-a",
            "--after-seq",
            "7",
            "--limit",
            "3",
            "--requester",
            "alice-phone",
        ])
        .expect("prepared request");

        let body: GroupSyncRequest =
            serde_json::from_value(request.json.expect("json")).expect("sync request");
        assert_eq!(body.group_id.as_slice(), b"room-a");
        assert_eq!(body.after_seq, 7);
        assert_eq!(body.limit, 3);
        assert_eq!(
            body.requester.expect("requester").as_slice(),
            b"alice-phone"
        );
    }

    #[test]
    fn submit_commit_command_posts_request_json() {
        let request = prepare_http_request([
            "--server",
            "http://localhost:9000",
            "submit-commit",
            "--request-json",
            r#"{"room_id":"room-a","idempotency_key":"idem-a"}"#,
        ])
        .expect("prepared request");

        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, "http://localhost:9000/commits");
        let body = request.json.expect("json");
        assert_eq!(body["room_id"], "room-a");
        assert_eq!(body["idempotency_key"], "idem-a");
    }

    #[test]
    fn event_and_activity_commands_post_request_json() {
        let event = prepare_http_request([
            "append-event",
            "--request-json",
            r#"{"event":{"room_id":"room-a","sender":"alice-phone"},"delivery_policy":{"push":"default","unread":"never","command_inbox":"create"}}"#,
        ])
        .expect("event request");

        assert_eq!(event.method, HttpMethod::Post);
        assert_eq!(event.url, "https://chat.finite.computer/events");
        let body = event.json.expect("json");
        assert_eq!(body["event"]["room_id"], "room-a");
        assert_eq!(body["event"]["sender"], "alice-phone");
        assert_eq!(body["delivery_policy"]["command_inbox"], "create");

        let effect = prepare_http_request([
            "application-effect-get",
            "--message-id",
            "application-message-a",
        ])
        .expect("effect request");

        assert_eq!(effect.method, HttpMethod::Post);
        assert_eq!(
            effect.url,
            "https://chat.finite.computer/application-effects/get"
        );
        let body: ApplicationEffectRequest =
            serde_json::from_value(effect.json.expect("json")).expect("effect request body");
        assert_eq!(body.message_id, "application-message-a");

        let counts = prepare_http_request(["application-effect-counts"]).expect("counts request");

        assert_eq!(counts.method, HttpMethod::Post);
        assert_eq!(
            counts.url,
            "https://chat.finite.computer/application-effects/counts"
        );
        assert_eq!(counts.json.expect("json"), serde_json::json!({}));

        let activity = prepare_http_request([
            "append-activity",
            "--request-json",
            r#"{"room_id":"room-a","sender":"alice-phone","activity_id":"typing-a"}"#,
        ])
        .expect("activity request");

        assert_eq!(activity.method, HttpMethod::Post);
        assert_eq!(activity.url, "https://chat.finite.computer/activities");
        let body = activity.json.expect("json");
        assert_eq!(body["room_id"], "room-a");
        assert_eq!(body["sender"], "alice-phone");
        assert_eq!(body["activity_id"], "typing-a");
    }

    #[test]
    fn revoke_device_command_builds_revoke_request() {
        let request = prepare_http_request([
            "revoke-device",
            "--account-id",
            "alice",
            "--device-id",
            "alice-phone",
        ])
        .expect("request");

        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, "https://chat.finite.computer/devices/revoke");
        let body: RevokeDeviceRequest =
            serde_json::from_value(request.json.expect("json")).expect("revoke request");
        assert_eq!(body.device, DeviceRef::new("alice", "alice-phone"));
    }

    #[test]
    fn device_liveness_commands_build_route_dtos() {
        let observe = prepare_http_request([
            "observe-device-liveness",
            "--account-id",
            "alice",
            "--device-id",
            "alice-phone",
            "--observed-at-ms",
            "1000",
            "--expires-at-ms",
            "61000",
        ])
        .expect("observe request");

        assert_eq!(observe.method, HttpMethod::Post);
        assert_eq!(observe.url, "https://chat.finite.computer/devices/liveness");
        let body: ObserveDeviceLivenessRequest =
            serde_json::from_value(observe.json.expect("json")).expect("liveness observe request");
        assert_eq!(body.device, DeviceRef::new("alice", "alice-phone"));
        assert_eq!(body.observed_at_ms, 1000);
        assert_eq!(body.expires_at_ms, 61000);

        let get = prepare_http_request([
            "get-device-liveness",
            "--account-id",
            "alice",
            "--device-id",
            "alice-phone",
            "--now-ms",
            "60000",
        ])
        .expect("get request");

        assert_eq!(get.method, HttpMethod::Post);
        assert_eq!(get.url, "https://chat.finite.computer/devices/liveness/get");
        let body: GetDeviceLivenessRequest =
            serde_json::from_value(get.json.expect("json")).expect("liveness get request");
        assert_eq!(body.device, DeviceRef::new("alice", "alice-phone"));
        assert_eq!(body.now_ms, 60000);
    }

    #[test]
    fn claim_key_package_command_builds_claim_request() {
        let request =
            prepare_http_request(["claim-key-package", "--owner", "alice"]).expect("request");

        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(
            request.url,
            "https://chat.finite.computer/key-packages/claim"
        );
        let body: ClaimKeyPackageRequest =
            serde_json::from_value(request.json.expect("json")).expect("claim request");
        assert_eq!(body.owner.as_slice(), b"alice");
    }

    #[test]
    fn key_package_inventory_command_builds_inventory_request() {
        let request =
            prepare_http_request(["key-package-inventory", "--owner", "alice"]).expect("request");

        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(
            request.url,
            "https://chat.finite.computer/key-packages/inventory"
        );
        let body: KeyPackageInventoryRequest =
            serde_json::from_value(request.json.expect("json")).expect("inventory request");
        assert_eq!(body.owner.as_slice(), b"alice");
    }

    #[test]
    fn claim_key_packages_command_builds_batch_claim_request() {
        let request = prepare_http_request([
            "claim-key-packages",
            "--owner",
            "alice-phone",
            "--owner",
            "alice-laptop",
            "--idempotency-key",
            "fanout-claim-1",
        ])
        .expect("request");

        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(
            request.url,
            "https://chat.finite.computer/key-packages/claims"
        );
        let body: ClaimKeyPackagesRequest =
            serde_json::from_value(request.json.expect("json")).expect("batch claim request");
        assert_eq!(body.owners.len(), 2);
        assert_eq!(body.owners[0].as_slice(), b"alice-phone");
        assert_eq!(body.owners[1].as_slice(), b"alice-laptop");
        assert_eq!(body.idempotency_key.as_deref(), Some("fanout-claim-1"));
    }

    #[test]
    fn expire_key_package_lease_command_builds_expiry_request() {
        let request = prepare_http_request([
            "expire-key-package-lease",
            "--key-package-id",
            "kp-lease-expired",
        ])
        .expect("request");

        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(
            request.url,
            "https://chat.finite.computer/key-packages/leases/expire"
        );
        let body: ExpireKeyPackageLeaseRequest =
            serde_json::from_value(request.json.expect("json")).expect("expiry request");
        assert_eq!(body.key_package_id.as_slice(), b"kp-lease-expired");
    }

    #[test]
    fn account_room_commands_build_route_dtos() {
        let bootstrap = prepare_http_request([
            "account-room-bootstrap",
            "--room-id",
            "room-a",
            "--mls-group-id",
            "mls-a",
            "--account-id",
            "alice",
            "--device-id",
            "alice-phone",
        ])
        .expect("bootstrap request");

        assert_eq!(bootstrap.method, HttpMethod::Post);
        assert_eq!(
            bootstrap.url,
            "https://chat.finite.computer/account-rooms/bootstrap"
        );
        let body: BootstrapAccountRoomRequest =
            serde_json::from_value(bootstrap.json.expect("json"))
                .expect("account-room bootstrap request");
        assert_eq!(body.room_id, "room-a");
        assert_eq!(body.mls_group_id, "mls-a");
        assert_eq!(body.creator.account_id, "alice");
        assert_eq!(body.creator.device_id, "alice-phone");

        let save = prepare_http_request([
            "account-room-save",
            "--account-id",
            "alice",
            "--room-id",
            "room-a",
            "--record-json",
            r#"{"room_id":"room-a","current_epoch":2}"#,
        ])
        .expect("save request");

        assert_eq!(save.method, HttpMethod::Post);
        assert_eq!(save.url, "https://chat.finite.computer/account-rooms");
        let body: SaveAccountRoomRequest =
            serde_json::from_value(save.json.expect("json")).expect("account-room save request");
        assert_eq!(body.account_id, "alice");
        assert_eq!(body.room_id, "room-a");
        assert_eq!(body.record["current_epoch"], 2);

        let list = prepare_http_request([
            "account-rooms-list",
            "--account-id",
            "alice",
            "--after-room-id",
            "room-a",
            "--limit",
            "3",
        ])
        .expect("list request");

        assert_eq!(list.method, HttpMethod::Post);
        assert_eq!(list.url, "https://chat.finite.computer/account-rooms/list");
        let body: ListAccountRoomDirectoryRequest =
            serde_json::from_value(list.json.expect("json")).expect("account-room list request");
        assert_eq!(body.account_id, "alice");
        assert_eq!(body.after_room_id.as_deref(), Some("room-a"));
        assert_eq!(body.limit, 3);
    }

    #[test]
    fn report_invalid_commit_command_builds_route_dto() {
        let request = prepare_http_request([
            "report-invalid-commit",
            "--room-id",
            "room-a",
            "--account-id",
            "alice",
            "--device-id",
            "alice-phone",
            "--offending-seq",
            "12",
        ])
        .expect("report request");

        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(
            request.url,
            "https://chat.finite.computer/rooms/report-invalid-commit"
        );
        let body: ReportInvalidCommitRequest =
            serde_json::from_value(request.json.expect("json")).expect("report body");
        assert_eq!(body.room_id, "room-a");
        assert_eq!(body.reporter, DeviceRef::new("alice", "alice-phone"));
        assert_eq!(body.offending_seq, 12);
    }

    #[test]
    fn claim_welcomes_command_builds_claim_request() {
        let request = prepare_http_request([
            "claim-welcomes",
            "--recipient",
            "bob-device",
            "--limit",
            "3",
        ])
        .expect("request");

        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, "https://chat.finite.computer/welcomes/claim");
        let body: ClaimWelcomesRequest =
            serde_json::from_value(request.json.expect("json")).expect("claim welcomes request");
        assert_eq!(body.recipient.as_slice(), b"bob-device");
        assert_eq!(body.limit, 3);
    }

    #[test]
    fn raw_key_package_owner_rejects_device_ref_json() {
        let device_json = serde_json::to_string(&DeviceRef::new("alice", "alice-phone"))
            .expect("device ref json");
        let error = prepare_http_request([
            "publish-key-package",
            "--owner",
            &device_json,
            "--key-package-id",
            "alice-phone-1",
            "--bytes",
            "package",
        ])
        .expect_err("DeviceRef JSON is not a raw delivery owner");
        assert!(error.to_string().contains("raw delivery MemberId"));
    }

    #[test]
    fn ack_welcome_command_builds_ack_request() {
        let request =
            prepare_http_request(["ack-welcome", "--message-id", "welcome-bob"]).expect("request");

        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, "https://chat.finite.computer/welcomes/ack");
        let body: AckWelcomeRequest =
            serde_json::from_value(request.json.expect("json")).expect("ack welcome request");
        assert_eq!(body.message_id.as_slice(), b"welcome-bob");
    }

    #[test]
    fn live_client_submit_commit_claim_and_ack_welcome_over_http_server() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server_db = dir.path().join("cli-live-submit.sqlite3");
        let server_url = spawn_live_cli_server(&server_db);
        let mut creator = test_finitechat_device(CLI_LIVE_ALICE_SECRET, "alice-laptop");
        let phone = test_finitechat_device(CLI_LIVE_ALICE_SECRET, "alice-phone");
        let room_id = "room-cli-live-submit";
        let mls_group_id = "mls-cli-live-submit";
        let welcome_id = "welcome-cli-live-phone";
        creator
            .create_group_state(room_id, mls_group_id)
            .expect("creator group state");

        let bootstrap = run_cli_json([
            "http",
            "--server",
            &server_url,
            "account-room-bootstrap",
            "--room-id",
            room_id,
            "--mls-group-id",
            mls_group_id,
            "--account-id",
            &creator.device_ref().account_id,
            "--device-id",
            &creator.device_ref().device_id,
        ]);
        assert_eq!(bootstrap["bootstrapped"], true);

        let mut delivery = HttpRuntimeDelivery::new(
            ReqwestHttpRuntimeTransport::new(server_url.clone())
                .with_signer(CLI_LIVE_ALICE_SECRET),
        );
        let upload = phone
            .upload_key_package_request("key-package-add-device")
            .expect("phone upload KeyPackage request");
        delivery
            .upload_key_package(upload.clone())
            .expect("publish commit KeyPackage through product delivery");
        let claimed = delivery
            .claim_key_package_for_device(phone.device_ref())
            .expect("claim commit KeyPackage through product delivery")
            .expect("uploaded package can be claimed");
        assert_eq!(claimed.owner, *phone.device_ref());
        assert_eq!(claimed.key_package_id, upload.key_package_id);
        assert_eq!(claimed.key_package_ref, upload.key_package_ref);
        assert_eq!(claimed.key_package_hash, upload.key_package_hash);

        let prepared = creator
            .prepare_add_members_commit(
                room_id,
                &[claimed],
                &[welcome_id.to_owned()],
                "commit-cli-live-idempotency",
            )
            .expect("prepare add-device commit");
        let expected_message_id = prepared.message_id.clone();
        let submit_request = prepared.request.clone();
        let accepted = delivery
            .submit_commit(prepared.request)
            .expect("commit accepted through product delivery");
        assert_eq!(accepted.seq, 1);
        assert_eq!(accepted.message_id, expected_message_id);
        assert_eq!(accepted.released_welcomes, vec![welcome_id.to_owned()]);

        let submit_json = serde_json::to_string(&submit_request).expect("submit json");
        let replayed: CommitAccepted = serde_json::from_value(run_cli_json([
            "http",
            "--server",
            &server_url,
            "submit-commit",
            "--request-json",
            &submit_json,
        ]))
        .expect("commit replay");
        assert_eq!(replayed, accepted);

        let group_page: HttpSyncPage = serde_json::from_value(run_cli_json([
            "http",
            "--server",
            &server_url,
            "sync-group",
            "--group-id",
            room_id,
            "--limit",
            "10",
        ]))
        .expect("group sync");
        assert_eq!(group_page.entries.len(), 1);
        assert_eq!(group_page.entries[0].seq, accepted.seq);
        assert_eq!(
            group_page.entries[0].message.id.as_slice(),
            accepted.message_id.as_bytes()
        );

        let claimed = delivery
            .claim_welcomes(phone.device_ref())
            .expect("claim welcomes through product delivery");
        assert_eq!(claimed.len(), 1);
        let welcome = &claimed[0];
        assert_eq!(welcome.welcome_id, welcome_id);
        assert_eq!(welcome.commit_seq, accepted.seq);
        assert_eq!(welcome.recipient, *phone.device_ref());
        assert_eq!(welcome.state, WelcomeState::Claimed);

        let duplicate_claim = delivery
            .claim_welcomes(phone.device_ref())
            .expect("duplicate claim through product delivery");
        assert!(duplicate_claim.is_empty());

        delivery
            .ack_welcome(welcome_id)
            .expect("ack welcome through product delivery");
        delivery
            .ack_welcome(welcome_id)
            .expect("idempotent ack through product delivery");

        let listed = run_cli_json([
            "http",
            "--server",
            &server_url,
            "account-rooms-list",
            "--account-id",
            &creator.device_ref().account_id,
            "--limit",
            "10",
        ]);
        assert_eq!(listed["rooms"][0]["devices"][0]["active"], true);
        assert_eq!(listed["rooms"][0]["devices"][1]["active"], true);
    }

    #[test]
    fn live_cli_batch_key_package_claim_replays_over_http_server() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server_db = dir.path().join("cli-live-key-packages.sqlite3");
        let server_url = spawn_live_cli_server(&server_db);

        for (owner, key_package_id, bytes) in [
            ("live-laptop", "live-laptop-1", "laptop-package"),
            ("live-phone", "live-phone-1", "phone-package-1"),
            ("live-phone", "live-phone-2", "phone-package-2"),
        ] {
            let response: PublishKeyPackageResponse = serde_json::from_value(run_cli_json([
                "http",
                "--server",
                &server_url,
                "publish-key-package",
                "--owner",
                owner,
                "--key-package-id",
                key_package_id,
                "--bytes",
                bytes,
            ]))
            .expect("publish package response");
            assert!(response.published);
        }

        let claims: Vec<HttpKeyPackageClaim> = serde_json::from_value(run_cli_json([
            "http",
            "--server",
            &server_url,
            "claim-key-packages",
            "--owner",
            "live-laptop",
            "--owner",
            "live-phone",
            "--idempotency-key",
            "live-batch-claim",
        ]))
        .expect("batch claims");
        assert_eq!(claims.len(), 2);
        assert_claimed_package(&claims[0], "live-laptop", "live-laptop-1");
        assert_claimed_package(&claims[1], "live-phone", "live-phone-1");

        let replayed: Vec<HttpKeyPackageClaim> = serde_json::from_value(run_cli_json([
            "http",
            "--server",
            &server_url,
            "claim-key-packages",
            "--owner",
            "live-laptop",
            "--owner",
            "live-phone",
            "--idempotency-key",
            "live-batch-claim",
        ]))
        .expect("batch claim replay");
        assert_eq!(replayed, claims);

        let remaining: finitechat_delivery::HttpClaimedKeyPackage =
            serde_json::from_value(run_cli_json([
                "http",
                "--server",
                &server_url,
                "claim-key-package",
                "--owner",
                "live-phone",
            ]))
            .expect("remaining phone package");
        assert_eq!(remaining.key_package_id.as_slice(), b"live-phone-2");
        assert_eq!(remaining.owner.as_slice(), b"live-phone");
    }

    #[test]
    fn unknown_option_is_usage_error() {
        let error = prepare_http_request(["health", "--wat"]).expect_err("usage error");
        assert!(matches!(error, CliError::Usage(_)));
    }

    #[test]
    fn core_product_command_is_removed() {
        let mut output = Vec::new();
        let error = run(["core"], &mut output).expect_err("core command is gone");
        assert!(matches!(error, CliError::Usage(_)));
    }

    #[test]
    fn app_identity_and_state_use_runtime() {
        crate::ensure_test_finite_home();
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("app").display().to_string();

        let identity = run_cli_json([
            "app",
            "--data-dir",
            &data_dir,
            "--server",
            "http://127.0.0.1:1",
            "--device-id",
            "cli-device",
            "--now",
            "1000",
            "identity",
        ]);
        assert_eq!(identity["device_id"], "cli-device");
        assert!(identity["account_id"].as_str().unwrap().len() > 16);

        let state = run_cli_json([
            "app",
            "--data-dir",
            &data_dir,
            "--server",
            "http://127.0.0.1:1",
            "--device-id",
            "cli-device",
            "--now",
            "1000",
            "state",
        ]);
        assert_eq!(state["identity"]["account_id"], identity["account_id"]);
        assert_eq!(state["rooms"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn app_cli_add_member_and_message_flow_uses_runtime() {
        crate::ensure_test_finite_home();
        let dir = tempfile::tempdir().unwrap();
        let server_url = spawn_live_cli_server(&dir.path().join("server.sqlite3"));
        let alice_dir = dir.path().join("alice").display().to_string();
        let bob_dir = dir.path().join("bob").display().to_string();
        // Alice drives the CLI (shared test identity). Bob must be a distinct
        // account: the CLI has no secret flag (the shared identity is the only
        // CLI acquisition path), so bob runs through the core runtime with an
        // explicit in-memory secret, like a second user's device would.
        let bob_secret_hex = "42".repeat(32);
        let open_bob = || {
            finitechat_core::FiniteChatRuntime::open(finitechat_core::OpenOptions {
                data_dir: bob_dir.clone(),
                server_url: server_url.clone(),
                device_id: "bob-cli".to_owned(),
                account_secret_hex: Some(bob_secret_hex.clone()),
                now_unix_seconds: Some(1000),
            })
            .expect("bob runtime opens")
        };

        let created = run_cli_json([
            "app",
            "--data-dir",
            &alice_dir,
            "--server",
            &server_url,
            "--device-id",
            "alice-cli",
            "--now",
            "1000",
            "create-room",
            "--display-name",
            "CLI App Flow",
        ]);
        let room_id = created["selected_room_id"].as_str().unwrap().to_owned();
        assert_eq!(created["status"], "room created");

        let bob = open_bob();
        let bob_account_id = bob.state().expect("bob state").identity.account_id.clone();
        bob.dispatch_and_wait(finitechat_core::AppAction::StartRuntime)
            .expect("bob publishes key packages");
        drop(bob);

        let added = run_cli_json([
            "app",
            "--data-dir",
            &alice_dir,
            "--server",
            &server_url,
            "--device-id",
            "alice-cli",
            "--now",
            "1000",
            "add-member",
            "--room-id",
            &room_id,
            "--account-id",
            &bob_account_id,
            "--display-name",
            "Bob CLI",
        ]);
        assert_eq!(added["status"], "people added");

        run_cli_json([
            "app",
            "--data-dir",
            &alice_dir,
            "--server",
            &server_url,
            "--device-id",
            "alice-cli",
            "--now",
            "1000",
            "start",
        ]);
        let joined = open_bob()
            .dispatch_and_wait(finitechat_core::AppAction::StartRuntime)
            .expect("bob syncs");
        let bob_room = joined
            .rooms
            .iter()
            .find(|room| room.room_id == room_id)
            .expect("bob room projects");
        assert_eq!(format!("{:?}", bob_room.state), "Connected");
        let bob_home_topic = joined
            .topics
            .iter()
            .find(|topic| {
                topic.room_id == room_id && topic.topic_id == finitechat_core::HOME_TOPIC_ID
            })
            .expect("bob home topic projects");
        let bob_home_chat_id = bob_home_topic
            .active_chat_id
            .clone()
            .expect("bob home topic has an active chat");

        open_bob()
            .dispatch_and_wait(finitechat_core::AppAction::SendChatMessage {
                room_id: room_id.clone(),
                topic_id: finitechat_core::HOME_TOPIC_ID.to_owned(),
                chat_id: bob_home_chat_id.clone(),
                text: "hello from app cli".to_owned(),
                metadata_json: None,
            })
            .expect("bob sends");
        let synced = run_cli_json([
            "app",
            "--data-dir",
            &alice_dir,
            "--server",
            &server_url,
            "--device-id",
            "alice-cli",
            "--now",
            "1000",
            "start",
        ]);
        assert!(
            synced["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["text"] == "hello from app cli")
        );
    }

    fn run_cli_json<const N: usize>(args: [&str; N]) -> Value {
        let mut output = Vec::new();
        run(args, &mut output).expect("cli run");
        serde_json::from_slice(&output).expect("cli json output")
    }

    fn spawn_live_cli_server(path: &std::path::Path) -> String {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let app = finitechat_server::http_router(
            finitechat_server::HttpServerState::from_sqlite_path(path).unwrap(),
        );
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                axum::serve(listener, app).await.unwrap();
            });
        });
        let server_url = format!("http://{addr}");
        wait_for_live_cli_server(&server_url);
        server_url
    }

    fn wait_for_live_cli_server(server_url: &str) {
        let health_url = format!("{}/health", server_url.trim_end_matches('/'));
        let client = reqwest::blocking::Client::new();
        for _ in 0..100 {
            if client
                .get(&health_url)
                .send()
                .map(|response| response.status().is_success())
                .unwrap_or(false)
            {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("live CLI test server did not become healthy at {health_url}");
    }

    fn test_finitechat_device(
        account_secret_bytes: [u8; NOSTR_SECRET_KEY_BYTES],
        device_id: &str,
    ) -> FiniteChatDevice {
        let config = FiniteChatDeviceConfig {
            account_secret_key: NostrSecretKey::from_bytes(account_secret_bytes).unwrap(),
            device_id: device_id.to_owned(),
            now_unix_seconds: 1000,
            credential_not_before_unix_seconds: 0,
            credential_not_after_unix_seconds: 86_400,
        };
        FiniteChatDevice::new(config).expect("test finitechat device")
    }

    fn assert_claimed_package(claim: &HttpKeyPackageClaim, owner: &str, key_package_id: &str) {
        assert_eq!(claim.owner.as_slice(), owner.as_bytes());
        let claimed = claim.claimed.as_ref().expect("claimed package");
        assert_eq!(claimed.owner.as_slice(), owner.as_bytes());
        assert_eq!(claimed.key_package_id.as_slice(), key_package_id.as_bytes());
    }
}
