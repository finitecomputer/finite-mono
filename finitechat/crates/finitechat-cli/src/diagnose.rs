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

use crate::CliError;
use crate::cli::DiagnoseArgs;
use crate::cli::DiagnoseCommand;
use crate::cli::DiagnoseRejectedEntryArgs;
use crate::parse_account_secret;
use crate::write_pretty_json;

pub(crate) fn run<W: Write>(args: DiagnoseArgs, output: &mut W) -> Result<(), CliError> {
    match args.command {
        DiagnoseCommand::RejectedEntry(args) => cmd_rejected_entry(args, output),
    }
}

fn cmd_rejected_entry<W: Write>(
    DiagnoseRejectedEntryArgs {
        store,
        work_dir,
        room_log,
        device_id,
        account_secret_hex,
        incident_alias,
        now_unix_seconds,
    }: DiagnoseRejectedEntryArgs,
    output: &mut W,
) -> Result<(), CliError> {
    let now_unix_seconds = now_unix_seconds.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    });

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

pub(crate) fn split_capture(
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
