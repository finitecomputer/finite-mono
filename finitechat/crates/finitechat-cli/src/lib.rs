use std::io::Write;

mod app;
mod auth;
mod hermes;

use finitechat_delivery::{HttpKeyPackageId, HttpKeyPackagePublication};
use finitechat_http::{
    AckLinkPayloadRequest, AckWelcomeRequest, ApplicationEffectRequest,
    BootstrapAccountRoomRequest, ClaimKeyPackageRequest, ClaimKeyPackagesRequest,
    ClaimLinkPayloadRequest, ClaimWelcomesRequest, CreateLinkSessionRequest,
    ExpireKeyPackageLeaseRequest, ExpireLinkSessionRequest, GetDeviceLivenessRequest,
    GetLinkSessionRequest, GroupSyncRequest, InboxSyncRequest, KeyPackageInventoryRequest,
    LeaveRoomRequest, ListAccountRoomDirectoryRequest, ObserveDeviceLivenessRequest,
    ReleaseLinkClaimRequest, ReportInvalidCommitRequest, RevokeDeviceRequest,
    SaveAccountRoomRequest, UpdateRoomAdminsRequest, UploadLinkPayloadRequest,
};
use finitechat_proto::{DeviceRef, RoomProtocol};
use finitechat_transport::engine::KeyPackage;
use finitechat_transport::{GroupId, MemberId, MessageId};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

const DEFAULT_SERVER_URL: &str = "https://chat.finite.computer";
const DEFAULT_SYNC_LIMIT: usize = 50;

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
}

pub fn run<I, S, W>(args: I, output: &mut W) -> Result<(), CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
    W: Write,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        // Success paths so `finitechat --version` works as an install check
        // and `finitechat --help` self-describes for agents (exit 0, stdout).
        Some("--version" | "-V" | "version") => {
            writeln!(output, "finitechat {}", env!("CARGO_PKG_VERSION")).map_err(CliError::Output)
        }
        Some("--help" | "-h" | "help") => writeln!(output, "{}", usage()).map_err(CliError::Output),
        Some("http-smoke") => {
            let ids = finitechat_delivery::prove_http_delivery_core_orders_commit_then_message()
                .expect("HTTP delivery core smoke passes");
            writeln!(
                output,
                "ordered {} messages through the Finite Chat HTTP delivery core",
                ids.len()
            )
            .map_err(CliError::Output)
        }
        Some("app") => app::run(args.into_iter().skip(1).collect(), output),
        Some("auth") => auth::run(args.into_iter().skip(1).collect(), output),
        Some("hermes") => hermes::run(args.into_iter().skip(1).collect(), output),
        Some("http") => {
            let request = prepare_http_request(args.into_iter().skip(1))?;
            execute_http_request(&request, output)
        }
        _ => Err(CliError::Usage(usage())),
    }
}

pub fn prepare_http_request<I, S>(args: I) -> Result<PreparedHttpRequest, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let server =
        take_option(&mut args, "--server")?.unwrap_or_else(|| DEFAULT_SERVER_URL.to_owned());
    let Some(command) = take_positional(&mut args) else {
        return Err(CliError::Usage(http_usage()));
    };

    match command.as_str() {
        "health" => {
            reject_extra_args(&args)?;
            Ok(PreparedHttpRequest {
                method: HttpMethod::Get,
                url: route_url(&server, "/health"),
                json: None,
            })
        }
        "submit-commit" => submit_commit_request(&server, args),
        "append-event" => append_event_request(&server, args),
        "application-effect-get" => application_effect_get_request(&server, args),
        "application-effect-counts" => application_effect_counts_request(&server, args),
        "append-activity" => append_activity_request(&server, args),
        "sync-group" => sync_group_request(&server, args),
        "sync-inbox" => sync_inbox_request(&server, args),
        "revoke-device" => revoke_device_request(&server, args),
        "observe-device-liveness" => observe_device_liveness_request(&server, args),
        "get-device-liveness" => get_device_liveness_request(&server, args),
        "publish-key-package" => publish_key_package_request(&server, args),
        "key-package-inventory" => key_package_inventory_request(&server, args),
        "claim-key-package" => claim_key_package_request(&server, args),
        "claim-key-packages" => claim_key_packages_request(&server, args),
        "expire-key-package-lease" => expire_key_package_lease_request(&server, args),
        "link-session-create" => link_session_create_request(&server, args),
        "link-session-get" => link_session_get_request(&server, args),
        "link-session-upload" => link_session_upload_request(&server, args),
        "link-session-claim" => link_session_claim_request(&server, args),
        "link-session-release" => link_session_release_request(&server, args),
        "link-session-ack" => link_session_ack_request(&server, args),
        "link-session-expire" => link_session_expire_request(&server, args),
        "account-room-bootstrap" => account_room_bootstrap_request(&server, args),
        "account-room-save" => account_room_save_request(&server, args),
        "account-rooms-list" => account_rooms_list_request(&server, args),
        "room-leave" => room_leave_request(&server, args),
        "room-admins" => room_admins_request(&server, args),
        "report-invalid-commit" => report_invalid_commit_request(&server, args),
        "claim-welcomes" => claim_welcomes_request(&server, args),
        "ack-welcome" => ack_welcome_request(&server, args),
        _ => Err(CliError::Usage(http_usage())),
    }
}

fn submit_commit_request(server: &str, args: Vec<String>) -> Result<PreparedHttpRequest, CliError> {
    request_json_passthrough(server, "/commits", args)
}

fn append_event_request(server: &str, args: Vec<String>) -> Result<PreparedHttpRequest, CliError> {
    request_json_passthrough(server, "/events", args)
}

fn application_effect_get_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let message_id = required_option(&mut args, "--message-id")?;
    reject_extra_args(&args)?;

    let request = ApplicationEffectRequest { message_id };
    post_json_request(server, "/application-effects/get", &request)
}

fn application_effect_counts_request(
    server: &str,
    args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    reject_extra_args(&args)?;
    post_json_request(
        server,
        "/application-effects/counts",
        &serde_json::json!({}),
    )
}

fn append_activity_request(
    server: &str,
    args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    request_json_passthrough(server, "/activities", args)
}

fn sync_group_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let group_id = required_option(&mut args, "--group-id")?;
    let after_seq = optional_u64(&mut args, "--after-seq", 0)?;
    let limit = optional_usize(&mut args, "--limit", DEFAULT_SYNC_LIMIT)?;
    let requester = take_option(&mut args, "--requester")?;
    reject_extra_args(&args)?;

    let request = GroupSyncRequest {
        group_id: GroupId::new(group_id.into_bytes()),
        after_seq,
        limit,
        requester: requester.map(|requester| MemberId::new(requester.into_bytes())),
    };
    post_json_request(server, "/sync/group", &request)
}

fn sync_inbox_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let recipient = required_option(&mut args, "--recipient")?;
    let after_seq = optional_u64(&mut args, "--after-seq", 0)?;
    let limit = optional_usize(&mut args, "--limit", DEFAULT_SYNC_LIMIT)?;
    reject_extra_args(&args)?;

    let request = InboxSyncRequest {
        recipient: MemberId::new(recipient.into_bytes()),
        after_seq,
        limit,
    };
    post_json_request(server, "/sync/inbox", &request)
}

fn revoke_device_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let account_id = required_option(&mut args, "--account-id")?;
    let device_id = required_option(&mut args, "--device-id")?;
    reject_extra_args(&args)?;

    let request = RevokeDeviceRequest {
        device: DeviceRef {
            account_id,
            device_id,
        },
    };
    post_json_request(server, "/devices/revoke", &request)
}

fn observe_device_liveness_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let account_id = required_option(&mut args, "--account-id")?;
    let device_id = required_option(&mut args, "--device-id")?;
    let observed_at_ms = required_option(&mut args, "--observed-at-ms")?;
    let expires_at_ms = required_option(&mut args, "--expires-at-ms")?;
    reject_extra_args(&args)?;

    let request = ObserveDeviceLivenessRequest {
        device: DeviceRef {
            account_id,
            device_id,
        },
        observed_at_ms: parse_u64("--observed-at-ms", &observed_at_ms)?,
        expires_at_ms: parse_u64("--expires-at-ms", &expires_at_ms)?,
    };
    post_json_request(server, "/devices/liveness", &request)
}

fn get_device_liveness_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let account_id = required_option(&mut args, "--account-id")?;
    let device_id = required_option(&mut args, "--device-id")?;
    let now_ms = required_option(&mut args, "--now-ms")?;
    reject_extra_args(&args)?;

    let request = GetDeviceLivenessRequest {
        device: DeviceRef {
            account_id,
            device_id,
        },
        now_ms: parse_u64("--now-ms", &now_ms)?,
    };
    post_json_request(server, "/devices/liveness/get", &request)
}

fn publish_key_package_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let owner = required_option(&mut args, "--owner")?;
    let key_package_id = required_option(&mut args, "--key-package-id")?;
    let bytes = required_option(&mut args, "--bytes")?;
    reject_extra_args(&args)?;

    let request = HttpKeyPackagePublication {
        key_package_id: HttpKeyPackageId::new(key_package_id.into_bytes()),
        owner: raw_delivery_owner_from_cli(owner)?,
        key_package: KeyPackage::new(bytes.into_bytes()),
    };
    post_json_request(server, "/key-packages", &request)
}

fn claim_key_package_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let owner = required_option(&mut args, "--owner")?;
    reject_extra_args(&args)?;

    let request = ClaimKeyPackageRequest {
        owner: raw_delivery_owner_from_cli(owner)?,
    };
    post_json_request(server, "/key-packages/claim", &request)
}

fn key_package_inventory_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let owner = required_option(&mut args, "--owner")?;
    reject_extra_args(&args)?;

    let request = KeyPackageInventoryRequest {
        owner: raw_delivery_owner_from_cli(owner)?,
    };
    post_json_request(server, "/key-packages/inventory", &request)
}

fn claim_key_packages_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let owners = take_repeated_option(&mut args, "--owner")?;
    let idempotency_key = take_option(&mut args, "--idempotency-key")?;
    reject_extra_args(&args)?;

    if owners.is_empty() {
        return Err(CliError::Usage(
            "claim-key-packages requires at least one --owner".to_owned(),
        ));
    }

    let request = ClaimKeyPackagesRequest {
        owners: owners
            .into_iter()
            .map(raw_delivery_owner_from_cli)
            .collect::<Result<Vec<_>, _>>()?,
        idempotency_key,
    };
    post_json_request(server, "/key-packages/claims", &request)
}

fn raw_delivery_owner_from_cli(owner: String) -> Result<MemberId, CliError> {
    if serde_json::from_str::<DeviceRef>(&owner).is_ok() {
        return Err(CliError::Usage(
            "--owner is a raw delivery MemberId, not DeviceRef JSON; use finitechat-client runtime delivery for Finite device KeyPackages".to_owned(),
        ));
    }
    Ok(MemberId::new(owner.into_bytes()))
}

fn expire_key_package_lease_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let key_package_id = required_option(&mut args, "--key-package-id")?;
    reject_extra_args(&args)?;

    let request = ExpireKeyPackageLeaseRequest {
        key_package_id: HttpKeyPackageId::new(key_package_id.into_bytes()),
    };
    post_json_request(server, "/key-packages/leases/expire", &request)
}

fn link_session_create_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let link_session_id = required_option(&mut args, "--link-session-id")?;
    let pairing_public_key = required_option(&mut args, "--pairing-public-key")?;
    reject_extra_args(&args)?;

    let request = CreateLinkSessionRequest {
        link_session_id,
        pairing_public_key,
    };
    post_json_request(server, "/link-sessions", &request)
}

fn link_session_get_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let link_session_id = required_option(&mut args, "--link-session-id")?;
    reject_extra_args(&args)?;

    let request = GetLinkSessionRequest { link_session_id };
    post_json_request(server, "/link-sessions/get", &request)
}

fn link_session_upload_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let link_session_id = required_option(&mut args, "--link-session-id")?;
    let encrypted_payload = required_option(&mut args, "--payload")?;
    reject_extra_args(&args)?;

    let request = UploadLinkPayloadRequest {
        link_session_id,
        encrypted_payload: encrypted_payload.into_bytes(),
    };
    post_json_request(server, "/link-sessions/payload", &request)
}

fn link_session_claim_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let link_session_id = required_option(&mut args, "--link-session-id")?;
    reject_extra_args(&args)?;

    let request = ClaimLinkPayloadRequest { link_session_id };
    post_json_request(server, "/link-sessions/claim", &request)
}

fn link_session_release_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let link_session_id = required_option(&mut args, "--link-session-id")?;
    reject_extra_args(&args)?;

    let request = ReleaseLinkClaimRequest { link_session_id };
    post_json_request(server, "/link-sessions/release", &request)
}

fn link_session_ack_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let link_session_id = required_option(&mut args, "--link-session-id")?;
    let claim_token = required_option(&mut args, "--claim-token")?;
    reject_extra_args(&args)?;

    let request = AckLinkPayloadRequest {
        link_session_id,
        claim_token,
    };
    post_json_request(server, "/link-sessions/ack", &request)
}

fn link_session_expire_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let link_session_id = required_option(&mut args, "--link-session-id")?;
    reject_extra_args(&args)?;

    let request = ExpireLinkSessionRequest { link_session_id };
    post_json_request(server, "/link-sessions/expire", &request)
}

fn account_room_save_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let account_id = required_option(&mut args, "--account-id")?;
    let room_id = required_option(&mut args, "--room-id")?;
    let record_json = required_option(&mut args, "--record-json")?;
    reject_extra_args(&args)?;

    let request = SaveAccountRoomRequest {
        account_id,
        room_id,
        record: serde_json::from_str(&record_json).map_err(CliError::Json)?,
    };
    post_json_request(server, "/account-rooms", &request)
}

fn account_room_bootstrap_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let room_id = required_option(&mut args, "--room-id")?;
    let mls_group_id = required_option(&mut args, "--mls-group-id")?;
    let account_id = required_option(&mut args, "--account-id")?;
    let device_id = required_option(&mut args, "--device-id")?;
    reject_extra_args(&args)?;

    let request = BootstrapAccountRoomRequest {
        room_id,
        mls_group_id,
        creator: DeviceRef {
            account_id,
            device_id,
        },
        protocol: RoomProtocol::default(),
    };
    post_json_request(server, "/account-rooms/bootstrap", &request)
}

fn account_rooms_list_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let account_id = required_option(&mut args, "--account-id")?;
    let after_room_id = take_option(&mut args, "--after-room-id")?;
    let limit = optional_usize(&mut args, "--limit", DEFAULT_SYNC_LIMIT)?;
    reject_extra_args(&args)?;

    let request = ListAccountRoomDirectoryRequest {
        account_id,
        after_room_id,
        limit,
    };
    post_json_request(server, "/account-rooms/list", &request)
}

fn room_leave_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let room_id = required_option(&mut args, "--room-id")?;
    let account_id = required_option(&mut args, "--account-id")?;
    let device_id = required_option(&mut args, "--device-id")?;
    reject_extra_args(&args)?;

    let request = LeaveRoomRequest {
        room_id,
        sender: DeviceRef {
            account_id,
            device_id,
        },
    };
    post_json_request(server, "/rooms/leave", &request)
}

fn room_admins_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let room_id = required_option(&mut args, "--room-id")?;
    let account_id = required_option(&mut args, "--account-id")?;
    let device_id = required_option(&mut args, "--device-id")?;
    let grant = take_option(&mut args, "--grant")?;
    let revoke = take_option(&mut args, "--revoke")?;
    reject_extra_args(&args)?;

    let request = UpdateRoomAdminsRequest {
        room_id,
        sender: DeviceRef {
            account_id,
            device_id,
        },
        grant,
        revoke,
    };
    post_json_request(server, "/rooms/admins", &request)
}

fn report_invalid_commit_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let room_id = required_option(&mut args, "--room-id")?;
    let account_id = required_option(&mut args, "--account-id")?;
    let device_id = required_option(&mut args, "--device-id")?;
    let offending_seq = required_option(&mut args, "--offending-seq")?;
    reject_extra_args(&args)?;

    let request = ReportInvalidCommitRequest {
        room_id,
        reporter: DeviceRef {
            account_id,
            device_id,
        },
        offending_seq: parse_u64("--offending-seq", &offending_seq)?,
    };
    post_json_request(server, "/rooms/report-invalid-commit", &request)
}

fn claim_welcomes_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let recipient = required_option(&mut args, "--recipient")?;
    let limit = optional_usize(&mut args, "--limit", DEFAULT_SYNC_LIMIT)?;
    reject_extra_args(&args)?;

    let request = ClaimWelcomesRequest {
        recipient: MemberId::new(recipient.into_bytes()),
        limit,
    };
    post_json_request(server, "/welcomes/claim", &request)
}

fn ack_welcome_request(
    server: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let message_id = required_option(&mut args, "--message-id")?;
    reject_extra_args(&args)?;

    let request = AckWelcomeRequest {
        message_id: MessageId::new(message_id.into_bytes()),
    };
    post_json_request(server, "/welcomes/ack", &request)
}

fn request_json_passthrough(
    server: &str,
    path: &str,
    mut args: Vec<String>,
) -> Result<PreparedHttpRequest, CliError> {
    let request_json = required_option(&mut args, "--request-json")?;
    reject_extra_args(&args)?;

    let request: Value = serde_json::from_str(&request_json).map_err(CliError::Json)?;
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

pub(crate) fn take_positional(args: &mut Vec<String>) -> Option<String> {
    if args.is_empty() {
        None
    } else {
        Some(args.remove(0))
    }
}

pub(crate) fn required_option(
    args: &mut Vec<String>,
    name: &'static str,
) -> Result<String, CliError> {
    take_option(args, name)?.ok_or_else(|| CliError::Usage(format!("missing required {name}")))
}

pub(crate) fn take_option(
    args: &mut Vec<String>,
    name: &'static str,
) -> Result<Option<String>, CliError> {
    let Some(index) = args.iter().position(|arg| arg == name) else {
        return Ok(None);
    };
    if index + 1 >= args.len() {
        return Err(CliError::Usage(format!("missing value for {name}")));
    }
    let value = args.remove(index + 1);
    args.remove(index);
    Ok(Some(value))
}

fn take_repeated_option(
    args: &mut Vec<String>,
    name: &'static str,
) -> Result<Vec<String>, CliError> {
    let mut values = Vec::new();
    while let Some(index) = args.iter().position(|arg| arg == name) {
        if index + 1 >= args.len() {
            return Err(CliError::Usage(format!("missing value for {name}")));
        }
        let value = args.remove(index + 1);
        args.remove(index);
        values.push(value);
    }
    Ok(values)
}

fn optional_u64(args: &mut Vec<String>, name: &'static str, default: u64) -> Result<u64, CliError> {
    take_option(args, name)?
        .map(|value| parse_u64(name, &value))
        .unwrap_or(Ok(default))
}

fn optional_usize(
    args: &mut Vec<String>,
    name: &'static str,
    default: usize,
) -> Result<usize, CliError> {
    take_option(args, name)?
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| CliError::Usage(format!("{name} must be an unsigned integer")))
        })
        .unwrap_or(Ok(default))
}

pub(crate) fn parse_u64(name: &'static str, value: &str) -> Result<u64, CliError> {
    value
        .parse::<u64>()
        .map_err(|_| CliError::Usage(format!("{name} must be an unsigned integer")))
}

pub(crate) fn reject_extra_args(args: &[String]) -> Result<(), CliError> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(CliError::Usage(format!(
            "unexpected argument '{}'",
            args[0]
        )))
    }
}

fn usage() -> String {
    format!(
        "usage: finitechat <http-smoke|http|auth|hermes|app>\n\n{}\n\n{}\n\n{}\n\n{}",
        auth::usage(),
        hermes::hermes_usage(),
        app::usage(),
        http_usage()
    )
}

fn http_usage() -> String {
    "http commands:\n  finitechat http [--server URL] health\n  finitechat http [--server URL] submit-commit --request-json JSON\n  finitechat http [--server URL] append-event --request-json JSON\n  finitechat http [--server URL] application-effect-get --message-id ID\n  finitechat http [--server URL] application-effect-counts\n  finitechat http [--server URL] append-activity --request-json JSON\n  finitechat http [--server URL] sync-group --group-id ID [--after-seq N] [--limit N] [--requester ID]\n  finitechat http [--server URL] sync-inbox --recipient ID [--after-seq N] [--limit N]\n  finitechat http [--server URL] revoke-device --account-id ID --device-id ID\n  finitechat http [--server URL] observe-device-liveness --account-id ID --device-id ID --observed-at-ms N --expires-at-ms N\n  finitechat http [--server URL] get-device-liveness --account-id ID --device-id ID --now-ms N\n  finitechat http [--server URL] publish-key-package --owner ID --key-package-id ID --bytes BYTES\n  finitechat http [--server URL] key-package-inventory --owner ID\n  finitechat http [--server URL] claim-key-package --owner ID\n  finitechat http [--server URL] claim-key-packages --owner ID [--owner ID ...] [--idempotency-key KEY]\n  finitechat http [--server URL] expire-key-package-lease --key-package-id ID\n  finitechat http [--server URL] link-session-create --link-session-id ID --pairing-public-key KEY\n  finitechat http [--server URL] link-session-get --link-session-id ID\n  finitechat http [--server URL] link-session-upload --link-session-id ID --payload BYTES\n  finitechat http [--server URL] link-session-claim --link-session-id ID\n  finitechat http [--server URL] link-session-release --link-session-id ID\n  finitechat http [--server URL] link-session-ack --link-session-id ID --claim-token TOKEN\n  finitechat http [--server URL] link-session-expire --link-session-id ID\n  finitechat http [--server URL] account-room-bootstrap --room-id ID --mls-group-id ID --account-id ID --device-id ID\n  finitechat http [--server URL] account-room-save --account-id ID --room-id ID --record-json JSON\n  finitechat http [--server URL] account-rooms-list --account-id ID [--after-room-id ID] [--limit N]\n  finitechat http [--server URL] room-leave --room-id ID --account-id ID --device-id ID\n  finitechat http [--server URL] room-admins --room-id ID --account-id ID --device-id ID [--grant ACCOUNT] [--revoke ACCOUNT]\n  finitechat http [--server URL] report-invalid-commit --room-id ID --account-id ID --device-id ID --offending-seq N\n  finitechat http [--server URL] claim-welcomes --recipient ID [--limit N]\n  finitechat http [--server URL] ack-welcome --message-id ID".to_owned()
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
        AckLinkPayloadRequest, AckWelcomeRequest, ApplicationEffectRequest,
        BootstrapAccountRoomRequest, ClaimKeyPackagesRequest, ClaimLinkPayloadRequest,
        ClaimWelcomesRequest, CreateLinkSessionRequest, ExpireKeyPackageLeaseRequest,
        ExpireLinkSessionRequest, GetDeviceLivenessRequest, GetLinkSessionRequest,
        GroupSyncRequest, HttpKeyPackageClaim, KeyPackageInventoryRequest,
        ListAccountRoomDirectoryRequest, ObserveDeviceLivenessRequest, PublishKeyPackageResponse,
        ReleaseLinkClaimRequest, ReportInvalidCommitRequest, RevokeDeviceRequest,
        SaveAccountRoomRequest, UploadLinkPayloadRequest,
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
    fn link_session_commands_build_route_dtos() {
        let create = prepare_http_request([
            "link-session-create",
            "--link-session-id",
            "link-a",
            "--pairing-public-key",
            "pairing-a",
        ])
        .expect("create request");

        assert_eq!(create.method, HttpMethod::Post);
        assert_eq!(create.url, "https://chat.finite.computer/link-sessions");
        let body: CreateLinkSessionRequest =
            serde_json::from_value(create.json.expect("json")).expect("create body");
        assert_eq!(body.link_session_id, "link-a");
        assert_eq!(body.pairing_public_key, "pairing-a");

        let get = prepare_http_request(["link-session-get", "--link-session-id", "link-a"])
            .expect("get request");
        assert_eq!(get.url, "https://chat.finite.computer/link-sessions/get");
        let body: GetLinkSessionRequest =
            serde_json::from_value(get.json.expect("json")).expect("get body");
        assert_eq!(body.link_session_id, "link-a");

        let upload = prepare_http_request([
            "link-session-upload",
            "--link-session-id",
            "link-a",
            "--payload",
            "ciphertext",
        ])
        .expect("upload request");
        assert_eq!(
            upload.url,
            "https://chat.finite.computer/link-sessions/payload"
        );
        let body: UploadLinkPayloadRequest =
            serde_json::from_value(upload.json.expect("json")).expect("upload body");
        assert_eq!(body.link_session_id, "link-a");
        assert_eq!(body.encrypted_payload, b"ciphertext");

        let claim = prepare_http_request(["link-session-claim", "--link-session-id", "link-a"])
            .expect("claim request");
        assert_eq!(
            claim.url,
            "https://chat.finite.computer/link-sessions/claim"
        );
        let body: ClaimLinkPayloadRequest =
            serde_json::from_value(claim.json.expect("json")).expect("claim body");
        assert_eq!(body.link_session_id, "link-a");

        let release = prepare_http_request(["link-session-release", "--link-session-id", "link-a"])
            .expect("release request");
        assert_eq!(
            release.url,
            "https://chat.finite.computer/link-sessions/release"
        );
        let body: ReleaseLinkClaimRequest =
            serde_json::from_value(release.json.expect("json")).expect("release body");
        assert_eq!(body.link_session_id, "link-a");

        let ack = prepare_http_request([
            "link-session-ack",
            "--link-session-id",
            "link-a",
            "--claim-token",
            "token-a",
        ])
        .expect("ack request");
        assert_eq!(ack.url, "https://chat.finite.computer/link-sessions/ack");
        let body: AckLinkPayloadRequest =
            serde_json::from_value(ack.json.expect("json")).expect("ack body");
        assert_eq!(body.link_session_id, "link-a");
        assert_eq!(body.claim_token, "token-a");

        let expire = prepare_http_request(["link-session-expire", "--link-session-id", "link-a"])
            .expect("expire request");
        assert_eq!(
            expire.url,
            "https://chat.finite.computer/link-sessions/expire"
        );
        let body: ExpireLinkSessionRequest =
            serde_json::from_value(expire.json.expect("json")).expect("expire body");
        assert_eq!(body.link_session_id, "link-a");
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

        let mut delivery =
            HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server_url.clone()));
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
