use std::io::Write;

use finitechat_core::{AppAction, AppProfileSummary, AppState, FiniteChatRuntime, OpenOptions};
use finitechat_proto::npub_encode;

use crate::CliError;
use crate::cli::AppArgs;
use crate::cli::AppCommand;
use crate::write_pretty_json;

pub(crate) fn run<W: Write>(args: AppArgs, output: &mut W) -> Result<(), CliError> {
    // The account key always comes from the shared Finite identity
    // ($FINITE_HOME/identity/, else ~/.finite/identity/), minted on first
    // run; there is no per-invocation secret flag (see `finitechat auth`).
    let options = OpenOptions {
        data_dir: args.data_dir,
        server_url: args.server,
        device_id: args.device_id,
        account_secret_hex: None,
        now_unix_seconds: args.now,
    };
    // A plain `state` read is the operator's non-mutating look at a resident
    // service's home: open read-only so it neither takes the writer lease nor
    // writes (StartRuntime/OpenRoom need the writer and fail fast when the
    // resident service holds the lease).
    let read_only = matches!(
        &args.command,
        AppCommand::State {
            start_runtime: false,
            wait_update_ms: None,
            room_id: None,
        }
    );
    let runtime = if read_only {
        FiniteChatRuntime::open_read_only(options).map_err(map_core_error)?
    } else {
        FiniteChatRuntime::open(options).map_err(map_core_error)?
    };

    match args.command {
        AppCommand::Identity => {
            write_pretty_json(output, &runtime.state().map_err(map_core_error)?.identity)
        }
        AppCommand::State {
            start_runtime,
            wait_update_ms,
            room_id,
        } => {
            let mut state = if start_runtime {
                runtime
                    .dispatch_and_wait(AppAction::StartRuntime)
                    .map_err(map_core_error)?
            } else {
                runtime.state().map_err(map_core_error)?
            };
            if let Some(timeout_millis) = wait_update_ms {
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
        AppCommand::Start => write_state(
            output,
            runtime
                .dispatch_and_wait(AppAction::StartRuntime)
                .map_err(map_core_error)?,
        ),
        AppCommand::Wait { timeout_ms } => write_state(
            output,
            runtime
                .wait_for_update(timeout_ms)
                .map_err(map_core_error)?,
        ),
        AppCommand::Stop => write_state(
            output,
            runtime
                .dispatch_and_wait(AppAction::StopRuntime)
                .map_err(map_core_error)?,
        ),
        AppCommand::OpenRoom { room_id } => write_state(
            output,
            runtime
                .dispatch_and_wait(AppAction::OpenRoom { room_id })
                .map_err(map_core_error)?,
        ),
        AppCommand::CreateRoom { display_name } => write_state(
            output,
            runtime
                .dispatch_and_wait(AppAction::CreateRoom { display_name })
                .map_err(map_core_error)?,
        ),
        AppCommand::AddMember {
            room_id,
            account_id,
            display_name,
        } => {
            let display_name = display_name.unwrap_or_else(|| {
                account_id
                    .get(..8)
                    .map(|prefix| format!("npub {prefix}"))
                    .unwrap_or_else(|| "Member".to_owned())
            });
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
        AppCommand::Scan { value } => write_state(
            output,
            runtime
                .dispatch_and_wait(AppAction::ScanTarget { value })
                .map_err(map_core_error)?,
        ),
        AppCommand::Send {
            room_id,
            text,
            metadata_json,
        } => write_state(
            output,
            runtime
                .dispatch_and_wait(AppAction::SendMessage {
                    room_id,
                    text,
                    metadata_json,
                })
                .map_err(map_core_error)?,
        ),
        AppCommand::MarkRead { room_id } => write_state(
            output,
            runtime
                .dispatch_and_wait(AppAction::MarkRoomRead { room_id })
                .map_err(map_core_error)?,
        ),
        AppCommand::RefreshDevices => write_state(
            output,
            runtime
                .dispatch_and_wait(AppAction::RefreshDevices)
                .map_err(map_core_error)?,
        ),
    }
}

fn write_state<W: Write>(output: &mut W, state: AppState) -> Result<(), CliError> {
    write_pretty_json(output, &state)
}

fn map_core_error(error: finitechat_core::FiniteChatCoreError) -> CliError {
    CliError::Runtime(error.to_string())
}
