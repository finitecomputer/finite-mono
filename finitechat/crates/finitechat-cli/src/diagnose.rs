//! The `finitechat diagnose` subcommand family: operator-only diagnostics
//! that run against local COPIES and never contact any server.
//!
//! `finitechat diagnose rejected-entry` replays a captured room log against
//! a byte copy of a client store and emits the Track B rejected-entry
//! classification record (see `finitechat-client`'s
//! `rejected_entry_diagnostic` module). The emitted record contains no
//! identifiers, plaintext, ciphertext, or raw error strings; it carries
//! only the caller-supplied incident alias, sequence numbers, an entry
//! binding digest, and a coarse error class.

use std::io::Write;
use std::path::PathBuf;

use finitechat_client::FiniteChatDeviceConfig;
use finitechat_client::rejected_entry_diagnostic::{
    CapturedRoomLogFile, RejectedEntryDiagnosticRequest, run_rejected_entry_diagnostic,
};

use crate::{CliError, reject_extra_args, required_option, take_option, take_positional};
use crate::{parse_account_secret, parse_u64, write_pretty_json};

pub(crate) fn run<W: Write>(mut args: Vec<String>, output: &mut W) -> Result<(), CliError> {
    let Some(command) = take_positional(&mut args) else {
        return Err(CliError::Usage(usage()));
    };
    match command.as_str() {
        "rejected-entry" => cmd_rejected_entry(&mut args, output),
        _ => Err(CliError::Usage(usage())),
    }
}

fn cmd_rejected_entry<W: Write>(args: &mut Vec<String>, output: &mut W) -> Result<(), CliError> {
    let store = required_option(args, "--store")?;
    let work_dir = required_option(args, "--work-dir")?;
    let room_log = required_option(args, "--room-log")?;
    let device_id = required_option(args, "--device-id")?;
    let account_secret_hex = required_option(args, "--account-secret-hex")?;
    let incident_alias = required_option(args, "--incident-alias")?;
    let now_unix_seconds = take_option(args, "--now-unix-seconds")?
        .map(|value| parse_u64("--now-unix-seconds", &value))
        .transpose()?
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0)
        });
    reject_extra_args(args)?;

    let capture_bytes = std::fs::read(&room_log).map_err(|error| {
        CliError::Runtime(format!("failed to read the captured room log: {error}"))
    })?;
    let capture: CapturedRoomLogFile =
        serde_json::from_slice(&capture_bytes).map_err(CliError::Json)?;
    let (target, other_rooms) = split_capture(capture)?;

    let config = FiniteChatDeviceConfig {
        account_secret_key: parse_account_secret(&account_secret_hex)?,
        device_id,
        now_unix_seconds,
        credential_not_before_unix_seconds: now_unix_seconds.saturating_sub(60),
        credential_not_after_unix_seconds: now_unix_seconds.saturating_add(60),
    };
    let request = RejectedEntryDiagnosticRequest {
        source_db_path: PathBuf::from(store),
        work_dir: PathBuf::from(work_dir),
        config,
        incident_alias,
        target,
        other_rooms,
    };
    let record = run_rejected_entry_diagnostic(&request)
        .map_err(|error| CliError::Runtime(format!("rejected-entry diagnostic failed: {error}")))?;
    write_pretty_json(output, &record)
}

fn split_capture(
    capture: CapturedRoomLogFile,
) -> Result<
    (
        finitechat_client::rejected_entry_diagnostic::CapturedRoomLog,
        Vec<finitechat_client::rejected_entry_diagnostic::CapturedRoomLog>,
    ),
    CliError,
> {
    let mut rooms = capture.rooms;
    let Some(index) = rooms
        .iter()
        .position(|room| room.room_id == capture.target_room_id)
    else {
        return Err(CliError::Usage(
            "the captured room log does not contain the target room".to_owned(),
        ));
    };
    let target = rooms.remove(index);
    Ok((target, rooms))
}

pub(crate) fn usage() -> String {
    "diagnose commands (operator-only; local copies only, never a server):\n  finitechat diagnose rejected-entry --store PATH --work-dir PATH --room-log PATH --device-id ID --account-secret-hex HEX --incident-alias ALIAS [--now-unix-seconds N]\n    --store: a COPY of the client store sqlite file (only read; byte-copied again internally)\n    --room-log: captured logs JSON: {\"target_room_id\": ID, \"rooms\": [{\"room_id\": ID, \"entries\": [...]}]}\n    emits the rejected-entry classification record (no identifiers, no plaintext, no ciphertext)".to_owned()
}
