use std::io::Write;

use finitechat_core::{AppAction, AppProfileSummary, AppState, FiniteChatRuntime, OpenOptions};
use finitechat_proto::npub_encode;

use crate::{
    CliError, DEFAULT_SERVER_URL, parse_u64, reject_extra_args, required_option, take_option,
    take_positional, write_pretty_json,
};

const DEFAULT_DATA_DIR: &str = ".finitechat";
const DEFAULT_DEVICE_ID: &str = "cli";

pub(crate) fn run<W: Write>(mut args: Vec<String>, output: &mut W) -> Result<(), CliError> {
    let data_dir = take_option(&mut args, "--data-dir")?.unwrap_or_else(|| DEFAULT_DATA_DIR.into());
    let server =
        take_option(&mut args, "--server")?.unwrap_or_else(|| DEFAULT_SERVER_URL.to_owned());
    let device_id =
        take_option(&mut args, "--device-id")?.unwrap_or_else(|| DEFAULT_DEVICE_ID.into());
    let now_unix_seconds = take_option(&mut args, "--now")?
        .map(|value| parse_u64("--now", &value))
        .transpose()?;
    let Some(command) = take_positional(&mut args) else {
        return Err(CliError::Usage(usage()));
    };

    // The account key always comes from the shared Finite identity
    // ($FINITE_HOME/identity/, else ~/.finite/identity/), minted on first
    // run; there is no per-invocation secret flag (see `finitechat auth`).
    let runtime = FiniteChatRuntime::open(OpenOptions {
        data_dir,
        server_url: server,
        device_id,
        account_secret_hex: None,
        now_unix_seconds,
    })
    .map_err(map_core_error)?;

    match command.as_str() {
        "identity" => {
            reject_extra_args(&args)?;
            write_pretty_json(output, &runtime.state().map_err(map_core_error)?.identity)
        }
        "state" => {
            let start_runtime = take_flag(&mut args, "--start-runtime");
            let wait_update = take_option(&mut args, "--wait-update-ms")?
                .map(|value| parse_u64("--wait-update-ms", &value))
                .transpose()?;
            let room_id = take_option(&mut args, "--room-id")?;
            reject_extra_args(&args)?;
            let mut state = if start_runtime {
                runtime
                    .dispatch_and_wait(AppAction::StartRuntime)
                    .map_err(map_core_error)?
            } else {
                runtime.state().map_err(map_core_error)?
            };
            if let Some(timeout_millis) = wait_update {
                state = runtime
                    .wait_for_update(timeout_millis)
                    .map_err(map_core_error)?;
            }
            if let Some(room_id) = room_id {
                state = runtime
                    .dispatch_and_wait(AppAction::OpenRoom { room_id })
                    .map_err(map_core_error)?;
            }
            write_pretty_json(output, &state)
        }
        "start" => write_state(
            output,
            runtime
                .dispatch_and_wait(AppAction::StartRuntime)
                .map_err(map_core_error)?,
        ),
        "wait" => {
            let timeout_millis = take_option(&mut args, "--timeout-ms")?
                .map(|value| parse_u64("--timeout-ms", &value))
                .transpose()?
                .unwrap_or(0);
            reject_extra_args(&args)?;
            write_state(
                output,
                runtime
                    .wait_for_update(timeout_millis)
                    .map_err(map_core_error)?,
            )
        }
        "stop" => write_state(
            output,
            runtime
                .dispatch_and_wait(AppAction::StopRuntime)
                .map_err(map_core_error)?,
        ),
        "open-room" => {
            let room_id = required_option(&mut args, "--room-id")?;
            reject_extra_args(&args)?;
            write_state(
                output,
                runtime
                    .dispatch_and_wait(AppAction::OpenRoom { room_id })
                    .map_err(map_core_error)?,
            )
        }
        "create-room" => {
            let display_name = take_option(&mut args, "--display-name")?.unwrap_or_default();
            reject_extra_args(&args)?;
            write_state(
                output,
                runtime
                    .dispatch_and_wait(AppAction::CreateRoom { display_name })
                    .map_err(map_core_error)?,
            )
        }
        "add-member" => {
            let room_id = required_option(&mut args, "--room-id")?;
            let account_id = required_option(&mut args, "--account-id")?;
            let display_name = take_option(&mut args, "--display-name")?.unwrap_or_else(|| {
                account_id
                    .get(..8)
                    .map(|prefix| format!("npub {prefix}"))
                    .unwrap_or_else(|| "Member".to_owned())
            });
            reject_extra_args(&args)?;
            let profile = AppProfileSummary {
                npub: npub_encode(&account_id).unwrap_or_else(|_| account_id.clone()),
                account_id,
                display_name,
                about: None,
                picture: None,
                stale: true,
                is_agent: false,
            };
            write_state(
                output,
                runtime
                    .dispatch_and_wait(AppAction::AddRoomMembers {
                        room_id,
                        profiles: vec![profile],
                    })
                    .map_err(map_core_error)?,
            )
        }
        "scan" => {
            let value = required_option(&mut args, "--value")?;
            reject_extra_args(&args)?;
            write_state(
                output,
                runtime
                    .dispatch_and_wait(AppAction::ScanTarget { value })
                    .map_err(map_core_error)?,
            )
        }
        "send" => {
            let room_id = required_option(&mut args, "--room-id")?;
            let text = required_option(&mut args, "--text")?;
            reject_extra_args(&args)?;
            write_state(
                output,
                runtime
                    .dispatch_and_wait(AppAction::SendMessage { room_id, text })
                    .map_err(map_core_error)?,
            )
        }
        "mark-read" => {
            let room_id = required_option(&mut args, "--room-id")?;
            reject_extra_args(&args)?;
            write_state(
                output,
                runtime
                    .dispatch_and_wait(AppAction::MarkRoomRead { room_id })
                    .map_err(map_core_error)?,
            )
        }
        "refresh-devices" => {
            reject_extra_args(&args)?;
            write_state(
                output,
                runtime
                    .dispatch_and_wait(AppAction::RefreshDevices)
                    .map_err(map_core_error)?,
            )
        }
        _ => Err(CliError::Usage(usage())),
    }
}

pub(crate) fn usage() -> String {
    "app commands:\n  finitechat app [--data-dir DIR] [--server URL] [--device-id ID] [--now SECONDS] identity\n  finitechat app [options] state [--start-runtime] [--wait-update-ms MS] [--room-id ID]\n  finitechat app [options] start\n  finitechat app [options] wait [--timeout-ms MS]\n  finitechat app [options] stop\n  finitechat app [options] open-room --room-id ID\n  finitechat app [options] create-room [--display-name NAME]\n  finitechat app [options] add-member --room-id ID --account-id ID [--display-name NAME]\n  finitechat app [options] scan --value PROFILE\n  finitechat app [options] send --room-id ID --text TEXT\n  finitechat app [options] mark-read --room-id ID\n  finitechat app [options] refresh-devices".to_owned()
}

fn write_state<W: Write>(output: &mut W, state: AppState) -> Result<(), CliError> {
    write_pretty_json(output, &state)
}

fn map_core_error(error: finitechat_core::FiniteChatCoreError) -> CliError {
    CliError::Runtime(error.to_string())
}

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    if let Some(index) = args.iter().position(|arg| arg == name) {
        args.remove(index);
        true
    } else {
        false
    }
}
