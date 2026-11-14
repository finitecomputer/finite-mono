use std::io::Write;

use finitechat_core::{
    AppAction, AppProfileSummary, AppState, AppView, ChatMediaAttachment, ChatMessage,
    ChatMessageKind, ChatMessageStatus, FiniteChatRuntime, OpenOptions,
};
use finitechat_proto::npub_encode;
use serde::Serialize;

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
    // The subcommand's declared class selects the open mode: a plain `state`
    // read is the operator's non-mutating look at a resident service's home
    // (no writer lease, no dispatch — the smoke-mystery incident class),
    // while writer commands acquire the store's single-writer lease.
    let runtime = FiniteChatRuntime::open_for_class(options, args.command.command_class())?;

    match args.command {
        AppCommand::Identity => write_pretty_json(output, &runtime.state()?.identity),
        AppCommand::State {
            start_runtime,
            wait_update_ms,
            room_id,
        } => {
            let mut state = if start_runtime {
                runtime.dispatch_and_wait(AppAction::StartRuntime)?
            } else {
                runtime.state()?
            };
            if let Some(timeout_millis) = wait_update_ms {
                state = runtime.wait_for_update(timeout_millis)?;
            }
            if let Some(room_id) = room_id {
                state = runtime.dispatch_and_wait(AppAction::OpenRoom { room_id })?;
            }
            write_pretty_json(output, &state)
        }
        AppCommand::Start => {
            write_state(output, runtime.dispatch_and_wait(AppAction::StartRuntime)?)
        }
        AppCommand::Wait { timeout_ms } => {
            write_state(output, runtime.wait_for_update(timeout_ms)?)
        }
        AppCommand::Stop => write_state(output, runtime.dispatch_and_wait(AppAction::StopRuntime)?),
        AppCommand::OpenRoom { room_id } => write_state(
            output,
            runtime.dispatch_and_wait(AppAction::OpenRoom { room_id })?,
        ),
        AppCommand::CreateRoom { display_name } => write_state(
            output,
            runtime.dispatch_and_wait(AppAction::CreateRoom { display_name })?,
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
                runtime.dispatch_and_wait(AppAction::AddRoomMembers {
                    room_id,
                    profiles: vec![profile],
                })?,
            )
        }
        AppCommand::Scan { value } => write_state(
            output,
            runtime.dispatch_and_wait(AppAction::ScanTarget { value })?,
        ),
        AppCommand::Send {
            room_id,
            text,
            metadata_json,
        } => write_state(
            output,
            runtime.dispatch_and_wait(AppAction::SendMessage {
                room_id,
                text,
                metadata_json,
            })?,
        ),
        AppCommand::MarkRead { room_id } => write_state(
            output,
            runtime.dispatch_and_wait(AppAction::MarkRoomRead { room_id })?,
        ),
        AppCommand::RefreshDevices => write_state(
            output,
            runtime.dispatch_and_wait(AppAction::RefreshDevices)?,
        ),
        AppCommand::ExportHistory => write_pretty_json(output, &export_history(&runtime)?),
    }
}

/// The Device's complete stored history, grouped the way people see it.
/// Each chat is read through a read-only transcript view, so the export never
/// changes the saved selection or publishes an update.
#[derive(Serialize)]
struct HistoryExport {
    format: &'static str,
    account_id: String,
    device_id: String,
    rooms: Vec<HistoryRoom>,
}

#[derive(Serialize)]
struct HistoryRoom {
    room_id: String,
    display_name: String,
    is_agent_chat: bool,
    topics: Vec<HistoryTopic>,
}

#[derive(Serialize)]
struct HistoryTopic {
    topic_id: String,
    title: String,
    archived: bool,
    message_count: u32,
    chats: Vec<HistoryChat>,
}

#[derive(Serialize)]
struct HistoryChat {
    chat_id: String,
    title: String,
    archived: bool,
    message_count: u32,
    messages: Vec<HistoryMessage>,
}

#[derive(Serialize)]
struct HistoryMessage {
    message_id: String,
    seq: u64,
    sender_account_id: String,
    sender_display_name: String,
    is_mine: bool,
    kind: ChatMessageKind,
    status: ChatMessageStatus,
    final_delivery: bool,
    text: String,
    timestamp_unix_seconds: u64,
    edit_of_message_id: Option<String>,
    reply_to_message_id: Option<String>,
    media: Vec<HistoryMedia>,
}

#[derive(Serialize)]
struct HistoryMedia {
    attachment_id: String,
    filename: String,
    mime_type: String,
}

fn export_history(runtime: &FiniteChatRuntime) -> Result<HistoryExport, CliError> {
    let state = runtime.state()?;
    let mut rooms = Vec::with_capacity(state.rooms.len());
    for room in &state.rooms {
        let mut topics = Vec::new();
        for topic in state
            .topics
            .iter()
            .filter(|topic| topic.room_id == room.room_id)
        {
            let mut chats = Vec::with_capacity(topic.chats.len());
            for chat in &topic.chats {
                let view = runtime.state_for_view(AppView {
                    room_id: Some(room.room_id.clone()),
                    topic_id: Some(topic.topic_id.clone()),
                    chat_id: Some(chat.chat_id.clone()),
                    limit: Some(u32::MAX),
                    oldest_message_id: None,
                })?;
                chats.push(HistoryChat {
                    chat_id: chat.chat_id.clone(),
                    title: chat.title.clone(),
                    archived: chat.archived,
                    message_count: chat.message_count,
                    messages: view.messages.into_iter().map(history_message).collect(),
                });
            }
            topics.push(HistoryTopic {
                topic_id: topic.topic_id.clone(),
                title: topic.title.clone(),
                archived: topic.archived,
                message_count: topic.message_count,
                chats,
            });
        }
        rooms.push(HistoryRoom {
            room_id: room.room_id.clone(),
            display_name: room.display_name.clone(),
            is_agent_chat: room.is_agent_chat,
            topics,
        });
    }
    Ok(HistoryExport {
        format: "finitechat.history.v1",
        account_id: state.identity.account_id,
        device_id: state.identity.device_id,
        rooms,
    })
}

fn history_message(message: ChatMessage) -> HistoryMessage {
    HistoryMessage {
        message_id: message.message_id,
        seq: message.seq,
        sender_account_id: message.sender_account_id,
        sender_display_name: message.sender_display_name,
        is_mine: message.is_mine,
        kind: message.kind,
        status: message.status,
        final_delivery: message.final_delivery,
        text: message.text,
        timestamp_unix_seconds: message.timestamp_unix_seconds,
        edit_of_message_id: message.edit_of_message_id,
        reply_to_message_id: message.reply_to_message_id,
        media: message.media.into_iter().map(history_media).collect(),
    }
}

fn history_media(media: ChatMediaAttachment) -> HistoryMedia {
    HistoryMedia {
        attachment_id: media.attachment_id,
        filename: media.filename,
        mime_type: media.mime_type,
    }
}

fn write_state<W: Write>(output: &mut W, state: AppState) -> Result<(), CliError> {
    write_pretty_json(output, &state)
}
