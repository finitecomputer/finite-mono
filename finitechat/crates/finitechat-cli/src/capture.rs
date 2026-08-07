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

use finitechat_client::room_log_capture::{
    DEFAULT_MAX_CAPTURE_PAGES, RoomLogCaptureRequest, capture_room_log,
};
use finitechat_client::{HttpRuntimeDelivery, ReqwestHttpRuntimeTransport};
use finitechat_proto::DeviceRef;
use serde::Serialize;

use crate::{
    CliError, parse_account_secret, parse_u64, reject_extra_args, required_option, take_option,
    take_positional,
};

pub(crate) fn run<W: Write>(mut args: Vec<String>, output: &mut W) -> Result<(), CliError> {
    let Some(command) = take_positional(&mut args) else {
        return Err(CliError::Usage(usage()));
    };
    match command.as_str() {
        "room-log" => cmd_capture_room_log(&mut args, output),
        _ => Err(CliError::Usage(usage())),
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

fn cmd_capture_room_log<W: Write>(args: &mut Vec<String>, output: &mut W) -> Result<(), CliError> {
    let server = required_option(args, "--server")?;
    let room_id = required_option(args, "--room-id")?;
    let device_id = required_option(args, "--device-id")?;
    let account_secret_file = required_option(args, "--account-secret-file")?;
    let out = required_option(args, "--out")?;
    let after_seq = take_option(args, "--after-seq")?
        .map(|value| parse_u64("--after-seq", &value))
        .transpose()?
        .unwrap_or(0);
    let max_pages = take_option(args, "--max-pages")?
        .map(|value| parse_u64("--max-pages", &value))
        .transpose()?
        .map(u32::try_from)
        .transpose()
        .map_err(|_| CliError::Usage("--max-pages must fit in a u32".to_owned()))?
        .unwrap_or(DEFAULT_MAX_CAPTURE_PAGES);
    reject_extra_args(args)?;

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

pub(crate) fn usage() -> String {
    "capture commands (operator-only; CONTACTS the given server, read-only):\n  finitechat capture room-log --server URL --room-id ID --device-id ID --account-secret-file PATH --out PATH [--after-seq N] [--max-pages N]\n    --server: base URL of the finitechat server to capture from (required, no default)\n    --account-secret-file: file containing the 64-char lowercase hex account secret (never a CLI arg)\n    --out: output path for the CapturedRoomLogFile JSON (must not exist yet)\n    pages /sync/group as the given device until the log is complete; feed the output to `finitechat diagnose rejected-entry --room-log`".to_owned()
}
