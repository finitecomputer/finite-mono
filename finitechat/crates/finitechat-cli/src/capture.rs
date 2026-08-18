//! The `finitechat capture` subcommand family: operator tooling that DOES
//! contact a running finitechat server (unlike `finitechat diagnose`, which
//! only ever touches local copies).
//!
//! `finitechat capture room-log` pages a room's full log off `/sync/group`
//! through the production client sync path and writes it as the
//! `CapturedRoomLogFile` JSON consumed by `finitechat diagnose
//! rejected-entry`. The capture is read-only: no commits, no appended
//! events, no cursor movement.
//!
//! The account secret is read from a file (`--account-secret-file`), never
//! from a command-line argument, so it cannot leak through process lists.

use std::io::Write;

use finitechat_client::room_log_capture::{RoomLogCaptureRequest, capture_room_log};
use finitechat_client::{HttpRuntimeDelivery, ReqwestHttpRuntimeTransport};
use finitechat_proto::DeviceRef;
use serde::Serialize;

use crate::CliError;
use crate::cli::CaptureArgs;
use crate::cli::CaptureCommand;
use crate::parse_account_secret;

pub(crate) fn run<W: Write>(args: CaptureArgs, output: &mut W) -> Result<(), CliError> {
    match args.command {
        CaptureCommand::RoomLog(args) => cmd_capture_room_log(args, output),
    }
}

#[derive(Serialize)]
struct CaptureSummary {
    room_id: String,
    entries: usize,
    first_seq: Option<u64>,
    last_seq: Option<u64>,
    out: String,
}

fn cmd_capture_room_log<W: Write>(
    args: crate::cli::CaptureRoomLogArgs,
    output: &mut W,
) -> Result<(), CliError> {
    let crate::cli::CaptureRoomLogArgs {
        server,
        room_id,
        device_id,
        account_secret_file,
        out,
        after_seq,
        max_pages,
    } = args;

    let secret_hex = std::fs::read_to_string(&account_secret_file).map_err(|error| {
        CliError::Runtime(format!(
            "failed to read the account secret file {account_secret_file}: {error}"
        ))
    })?;
    let account_secret = parse_account_secret(secret_hex.trim())?;
    // A device identifies itself to sync as the member id derived from its
    // account id (hex Nostr public key) and device id; there is no request
    // signature on /sync/group. Capture with the same identity a device
    // would present so the server scopes entries exactly as the device's
    // sync does.
    let requester = DeviceRef::new(hex_lower(account_secret.public_key().as_bytes()), device_id);
    requester
        .validate_limits()
        .map_err(|error| CliError::Usage(format!("invalid device identity: {error}")))?;

    let mut delivery = HttpRuntimeDelivery::new(ReqwestHttpRuntimeTransport::new(server));
    let capture = capture_room_log(
        &mut delivery,
        &RoomLogCaptureRequest {
            room_id: room_id.clone(),
            requester,
            after_seq,
            max_pages,
        },
    )
    .map_err(|error| CliError::Runtime(format!("room log capture failed: {error}")))?;

    // Refuse to overwrite: a previous capture is incident evidence.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&out)
        .map_err(|error| {
            CliError::Runtime(format!(
                "failed to create the capture output {out} (it must not exist yet): {error}"
            ))
        })?;
    serde_json::to_writer_pretty(&mut file, &capture).map_err(CliError::Serialize)?;
    writeln!(file).map_err(CliError::Output)?;

    let entries = &capture.rooms[0].entries;
    let summary = CaptureSummary {
        room_id,
        entries: entries.len(),
        first_seq: entries.first().map(|entry| entry.seq),
        last_seq: entries.last().map(|entry| entry.seq),
        out,
    };
    crate::write_pretty_json(output, &summary)
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
