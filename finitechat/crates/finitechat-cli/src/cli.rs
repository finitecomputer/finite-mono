//! The complete `finitechat` argument surface, declared with clap derive.
//!
//! This module is the ONLY place argument names, defaults, and shapes are
//! defined; the per-family modules (`app`, `auth`, `capture`, `diagnose`,
//! `hermes`, `repair`, and the `http` family driven from `lib`) receive
//! already-parsed values and keep their service logic untouched.
//!
//! Parity invariants carried over from the previous hand-rolled parser:
//! - The family-level options (`http --server`, `app --data-dir/--server/
//!   --device-id/--now`, `hermes --agent-home/--json/--request-json`)
//!   may appear before OR after their subcommand; they are clap `global`
//!   arguments for that reason.
//! - The hermes agent home falls back to `$FINITE_AGENT_HOME`, then
//!   `$FINITECHAT_HOME`, then `~/.finite/agent`; that chain is resolved in
//!   code after parsing so its precedence is testable.
//! - Free-form string options accept values that begin with `-`
//!   (`allow_hyphen_values`), matching the previous "next token is the
//!   value" behavior.
//! - Error wording for bad arguments is clap's; exit code stays 2 for every
//!   usage error and 0 for `--help`/`--version`.

use std::net::SocketAddr;

use clap::{Args, Parser, Subcommand};

use finitechat_core::CommandClass;

use crate::DEFAULT_SERVER_URL;
use crate::DEFAULT_SYNC_LIMIT;

const DEFAULT_APP_DATA_DIR: &str = ".finitechat";
const DEFAULT_APP_DEVICE_ID: &str = "cli";
const DEFAULT_HERMES_DEVICE_ID: &str = "agent";
const DEFAULT_HERMES_AGENT_NAME: &str = "Finite Agent";
const DEFAULT_HERMES_AGENT_ABOUT: &str = "A Finite Computer agent you can chat with.";
const DEFAULT_HERMES_AGENT_PICTURE: &str = "https://avatars.githubusercontent.com/u/274919006?v=4";
const DEFAULT_HERMES_SERVICE_ADDR: &str = "127.0.0.1:0";
const DEFAULT_HERMES_PLUGIN_NAME: &str = "finitechat";
const DEFAULT_CAPTURE_MAX_PAGES: u32 =
    finitechat_client::room_log_capture::DEFAULT_MAX_CAPTURE_PAGES;
const DEFAULT_REPAIR_MAX_SKIPS: u32 = 16;

#[derive(Debug, Parser)]
#[command(
    name = "finitechat",
    version,
    about = "Finite Chat CLI: identity, app runtime, Hermes agent bridge, and operator tooling",
    subcommand_required = true,
    arg_required_else_help = false
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Print the finitechat version (install check).
    Version,

    /// Run the HTTP delivery core smoke test (no server involved).
    HttpSmoke,

    /// Raw operator access to the finitechat server HTTP routes.
    Http(HttpArgs),

    /// Inspect or import the shared Finite identity (Finite Identity
    /// Contract v1; the account key lives under $FINITE_HOME/identity).
    Auth(AuthArgs),

    /// Drive a local Finite Chat app runtime: rooms, messages, state.
    App(AppArgs),

    /// Operator room-log capture from a running server (read-only).
    Capture(CaptureArgs),

    /// Operator diagnostics over local COPIES only (never a server).
    Diagnose(DiagnoseArgs),

    /// Operator repairs that write the REAL client store (fail-closed).
    Repair(RepairArgs),

    /// Hermes agent bridge: init, install, serve, poll, send, edit, ack,
    /// recover, activity, home-channel, room-status.
    Hermes(HermesArgs),
}

// --- auth ---

#[derive(Debug, Args)]
pub(crate) struct AuthArgs {
    #[command(subcommand)]
    pub(crate) command: AuthCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AuthCommand {
    /// Report the shared Finite identity (file, account id, npub).
    Status,

    /// Import an existing nsec or 64-hex secret from PATH or stdin into the
    /// shared identity location (never a CLI argument).
    Import {
        /// File holding an nsec or 64-hex secret; stdin when omitted.
        #[arg(long, allow_hyphen_values = true)]
        file: Option<String>,
    },
}

// --- app ---

#[derive(Debug, Args)]
pub(crate) struct AppArgs {
    /// App data directory for the local runtime.
    #[arg(long, global = true, default_value = DEFAULT_APP_DATA_DIR, allow_hyphen_values = true)]
    pub(crate) data_dir: String,

    /// Base URL of the finitechat server.
    #[arg(long, global = true, default_value = DEFAULT_SERVER_URL, allow_hyphen_values = true)]
    pub(crate) server: String,

    /// Device id this CLI invocation acts as.
    #[arg(long, global = true, default_value = DEFAULT_APP_DEVICE_ID, allow_hyphen_values = true)]
    pub(crate) device_id: String,

    /// Clock override for the runtime, in Unix seconds.
    #[arg(long, global = true)]
    pub(crate) now: Option<u64>,

    #[command(subcommand)]
    pub(crate) command: AppCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AppCommand {
    /// Print the resolved identity (account id, device id, npub).
    Identity,

    /// Print projected app state; optionally start the runtime, wait for an
    /// update, then open a room.
    State {
        /// Start the runtime before reading state.
        #[arg(long)]
        start_runtime: bool,

        /// Wait up to this many milliseconds for a state update.
        #[arg(long)]
        wait_update_ms: Option<u64>,

        /// Open this room after (optionally) starting.
        #[arg(long, allow_hyphen_values = true)]
        room_id: Option<String>,
    },

    /// Start the runtime and print the resulting state.
    Start,

    /// Wait up to `--timeout-ms` for a state update and print it.
    Wait {
        /// How long to wait, in milliseconds (default 0).
        #[arg(long, default_value_t = 0)]
        timeout_ms: u64,
    },

    /// Stop the runtime and print the resulting state.
    Stop,

    /// Open a room and print the resulting state.
    OpenRoom {
        /// Room to open.
        #[arg(long, allow_hyphen_values = true)]
        room_id: String,
    },

    /// Create a room and print the resulting state.
    CreateRoom {
        /// Display name for the new room (default empty).
        #[arg(long, default_value = "", allow_hyphen_values = true)]
        display_name: String,
    },

    /// Add one member to a room and print the resulting state.
    AddMember {
        /// Room to add the member to.
        #[arg(long, allow_hyphen_values = true)]
        room_id: String,

        /// Account id of the member to add.
        #[arg(long, allow_hyphen_values = true)]
        account_id: String,

        /// Display name for the member (default derives from the account id).
        #[arg(long, allow_hyphen_values = true)]
        display_name: Option<String>,
    },

    /// Scan a profile value (e.g. npub) and print the resulting state.
    Scan {
        /// Profile value to scan.
        #[arg(long, allow_hyphen_values = true)]
        value: String,
    },

    /// Send a chat message and print the resulting state.
    Send {
        /// Room to send to.
        #[arg(long, allow_hyphen_values = true)]
        room_id: String,

        /// Message text.
        #[arg(long, allow_hyphen_values = true)]
        text: String,

        /// JSON object of application metadata carried on the message
        /// payload (e.g. metadata.approve).
        #[arg(long, allow_hyphen_values = true)]
        metadata_json: Option<String>,
    },

    /// Mark a room read and print the resulting state.
    MarkRead {
        /// Room to mark read.
        #[arg(long, allow_hyphen_values = true)]
        room_id: String,
    },

    /// Refresh the room device lists and print the resulting state.
    RefreshDevices,
}

impl AppCommand {
    /// The subcommand's read/write class, declared once at registration and
    /// exhaustively: a new subcommand must name its class. The class selects
    /// the runtime open mode in `app::run` — only `Writer` commands acquire
    /// the store's single-writer lease; `ReadOnly` commands open read-only
    /// and never dispatch a writer action.
    pub(crate) fn command_class(&self) -> CommandClass {
        match self {
            // A plain `state` read is the operator's non-mutating look at a
            // resident service's home: no StartRuntime dispatch, no writer
            // lease. Any flag that starts the runtime, waits on a sync, or
            // opens a room makes the invocation a writer.
            Self::State {
                start_runtime: false,
                wait_update_ms: None,
                room_id: None,
            } => CommandClass::ReadOnly,
            Self::State { .. } => CommandClass::Writer,
            // `identity` resolves through the runtime open, which initializes
            // the store on first run; a read-only open requires an existing,
            // schema-current store, so this stays a writer open.
            Self::Identity => CommandClass::Writer,
            Self::Start => CommandClass::Writer,
            // `wait` applies sync hints to the store when an update lands.
            Self::Wait { .. } => CommandClass::Writer,
            Self::Stop => CommandClass::Writer,
            Self::OpenRoom { .. } => CommandClass::Writer,
            Self::CreateRoom { .. } => CommandClass::Writer,
            Self::AddMember { .. } => CommandClass::Writer,
            Self::Scan { .. } => CommandClass::Writer,
            Self::Send { .. } => CommandClass::Writer,
            Self::MarkRead { .. } => CommandClass::Writer,
            Self::RefreshDevices => CommandClass::Writer,
        }
    }
}

// --- http ---

#[derive(Debug, Parser)]
pub(crate) struct HttpArgs {
    /// Base URL of the finitechat server.
    #[arg(long, global = true, default_value = DEFAULT_SERVER_URL, allow_hyphen_values = true)]
    pub(crate) server: String,

    #[command(subcommand)]
    pub(crate) command: HttpCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum HttpCommand {
    /// GET /health
    Health,

    /// POST /commits with a raw request JSON.
    SubmitCommit {
        /// Raw request JSON body.
        #[arg(long, allow_hyphen_values = true)]
        request_json: String,
    },

    /// POST /events with a raw request JSON.
    AppendEvent {
        /// Raw request JSON body.
        #[arg(long, allow_hyphen_values = true)]
        request_json: String,
    },

    /// POST /application-effects/get for one message.
    ApplicationEffectGet {
        /// Message id to look up.
        #[arg(long, allow_hyphen_values = true)]
        message_id: String,
    },

    /// POST /application-effects/counts.
    ApplicationEffectCounts,

    /// POST /activities with a raw request JSON.
    AppendActivity {
        /// Raw request JSON body.
        #[arg(long, allow_hyphen_values = true)]
        request_json: String,
    },

    /// POST /sync/group for one group.
    SyncGroup {
        /// Group (room) id to sync.
        #[arg(long, allow_hyphen_values = true)]
        group_id: String,

        /// Resume after this sequence (default 0).
        #[arg(long, default_value_t = 0)]
        after_seq: u64,

        /// Page size (default 50).
        #[arg(long, default_value_t = DEFAULT_SYNC_LIMIT)]
        limit: usize,

        /// Member id requesting the sync.
        #[arg(long, allow_hyphen_values = true)]
        requester: Option<String>,
    },

    /// POST /sync/inbox for one recipient.
    SyncInbox {
        /// Member id whose inbox is synced.
        #[arg(long, allow_hyphen_values = true)]
        recipient: String,

        /// Resume after this sequence (default 0).
        #[arg(long, default_value_t = 0)]
        after_seq: u64,

        /// Page size (default 50).
        #[arg(long, default_value_t = DEFAULT_SYNC_LIMIT)]
        limit: usize,
    },

    /// POST /devices/revoke for one device.
    RevokeDevice {
        /// Account id of the device.
        #[arg(long, allow_hyphen_values = true)]
        account_id: String,

        /// Device id of the device.
        #[arg(long, allow_hyphen_values = true)]
        device_id: String,
    },

    /// POST /devices/liveness to observe a device.
    ObserveDeviceLiveness {
        /// Account id of the device.
        #[arg(long, allow_hyphen_values = true)]
        account_id: String,

        /// Device id of the device.
        #[arg(long, allow_hyphen_values = true)]
        device_id: String,

        /// Observation time, Unix milliseconds.
        #[arg(long)]
        observed_at_ms: u64,

        /// Observation expiry, Unix milliseconds.
        #[arg(long)]
        expires_at_ms: u64,
    },

    /// POST /devices/liveness/get for one device.
    GetDeviceLiveness {
        /// Account id of the device.
        #[arg(long, allow_hyphen_values = true)]
        account_id: String,

        /// Device id of the device.
        #[arg(long, allow_hyphen_values = true)]
        device_id: String,

        /// Current time, Unix milliseconds.
        #[arg(long)]
        now_ms: u64,
    },

    /// POST /key-packages to publish one raw delivery KeyPackage.
    PublishKeyPackage {
        /// Raw delivery MemberId owning the package (NOT DeviceRef JSON).
        #[arg(long, allow_hyphen_values = true)]
        owner: String,

        /// KeyPackage id.
        #[arg(long, allow_hyphen_values = true)]
        key_package_id: String,

        /// Raw KeyPackage bytes.
        #[arg(long, allow_hyphen_values = true)]
        bytes: String,
    },

    /// POST /key-packages/inventory for one owner.
    KeyPackageInventory {
        /// Raw delivery MemberId to inventory.
        #[arg(long, allow_hyphen_values = true)]
        owner: String,
    },

    /// POST /key-packages/claim for one owner.
    ClaimKeyPackage {
        /// Raw delivery MemberId to claim for.
        #[arg(long, allow_hyphen_values = true)]
        owner: String,
    },

    /// POST /key-packages/claims for a batch of owners.
    ClaimKeyPackages {
        /// Raw delivery MemberId to claim for (repeatable, at least one).
        #[arg(long = "owner", required = true, allow_hyphen_values = true)]
        owners: Vec<String>,

        /// Idempotency key for the batch claim.
        #[arg(long, allow_hyphen_values = true)]
        idempotency_key: Option<String>,
    },

    /// POST /key-packages/leases/expire for one package.
    ExpireKeyPackageLease {
        /// KeyPackage id whose lease expires.
        #[arg(long, allow_hyphen_values = true)]
        key_package_id: String,
    },

    /// POST /account-rooms/bootstrap for one room.
    AccountRoomBootstrap {
        /// Room id to bootstrap.
        #[arg(long, allow_hyphen_values = true)]
        room_id: String,

        /// MLS group id for the room.
        #[arg(long, allow_hyphen_values = true)]
        mls_group_id: String,

        /// Account id of the creating device.
        #[arg(long, allow_hyphen_values = true)]
        account_id: String,

        /// Device id of the creating device.
        #[arg(long, allow_hyphen_values = true)]
        device_id: String,
    },

    /// POST /account-rooms to save one room record.
    AccountRoomSave {
        /// Account id owning the room.
        #[arg(long, allow_hyphen_values = true)]
        account_id: String,

        /// Room id to save.
        #[arg(long, allow_hyphen_values = true)]
        room_id: String,

        /// Room record JSON body.
        #[arg(long, allow_hyphen_values = true)]
        record_json: String,
    },

    /// POST /account-rooms/list for one account.
    AccountRoomsList {
        /// Account id whose room directory is listed.
        #[arg(long, allow_hyphen_values = true)]
        account_id: String,

        /// Resume after this room id.
        #[arg(long, allow_hyphen_values = true)]
        after_room_id: Option<String>,

        /// Page size (default 50).
        #[arg(long, default_value_t = DEFAULT_SYNC_LIMIT)]
        limit: usize,
    },

    /// POST /rooms/leave for one room.
    RoomLeave {
        /// Room to leave.
        #[arg(long, allow_hyphen_values = true)]
        room_id: String,

        /// Account id of the leaving device.
        #[arg(long, allow_hyphen_values = true)]
        account_id: String,

        /// Device id of the leaving device.
        #[arg(long, allow_hyphen_values = true)]
        device_id: String,
    },

    /// POST /rooms/admins to grant or revoke room admins.
    RoomAdmins {
        /// Room to update admins for.
        #[arg(long, allow_hyphen_values = true)]
        room_id: String,

        /// Account id of the sender.
        #[arg(long, allow_hyphen_values = true)]
        account_id: String,

        /// Device id of the sender.
        #[arg(long, allow_hyphen_values = true)]
        device_id: String,

        /// Account id to grant admin.
        #[arg(long, allow_hyphen_values = true)]
        grant: Option<String>,

        /// Account id to revoke admin.
        #[arg(long, allow_hyphen_values = true)]
        revoke: Option<String>,
    },

    /// POST /rooms/report-invalid-commit for one room.
    ReportInvalidCommit {
        /// Room containing the offending commit.
        #[arg(long, allow_hyphen_values = true)]
        room_id: String,

        /// Account id of the reporter.
        #[arg(long, allow_hyphen_values = true)]
        account_id: String,

        /// Device id of the reporter.
        #[arg(long, allow_hyphen_values = true)]
        device_id: String,

        /// Sequence number of the offending commit.
        #[arg(long)]
        offending_seq: u64,
    },

    /// POST /welcomes/claim for one recipient.
    ClaimWelcomes {
        /// Member id claiming welcomes.
        #[arg(long, allow_hyphen_values = true)]
        recipient: String,

        /// Page size (default 50).
        #[arg(long, default_value_t = DEFAULT_SYNC_LIMIT)]
        limit: usize,
    },

    /// POST /welcomes/ack for one welcome.
    AckWelcome {
        /// Message id of the welcome to ack.
        #[arg(long, allow_hyphen_values = true)]
        message_id: String,
    },
}

// --- capture ---

#[derive(Debug, Args)]
pub(crate) struct CaptureArgs {
    #[command(subcommand)]
    pub(crate) command: CaptureCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum CaptureCommand {
    /// Page a room's full log off /sync/group (read-only) and write it as
    /// CapturedRoomLogFile JSON for `finitechat diagnose rejected-entry`.
    RoomLog(CaptureRoomLogArgs),
}

#[derive(Debug, Args)]
pub(crate) struct CaptureRoomLogArgs {
    /// Base URL of the finitechat server to capture from (required, no
    /// default).
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) server: String,

    /// Room to capture.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) room_id: String,

    /// Device id to sync as.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) device_id: String,

    /// File containing the 64-char lowercase hex account secret (never
    /// a CLI argument).
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) account_secret_file: String,

    /// Output path for the CapturedRoomLogFile JSON (must not exist
    /// yet).
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) out: String,

    /// Resume capture after this sequence (default 0).
    #[arg(long, default_value_t = 0)]
    pub(crate) after_seq: u64,

    /// Bound on capture pages.
    #[arg(long, default_value_t = DEFAULT_CAPTURE_MAX_PAGES)]
    pub(crate) max_pages: u32,
}

// --- diagnose ---

#[derive(Debug, Args)]
pub(crate) struct DiagnoseArgs {
    #[command(subcommand)]
    pub(crate) command: DiagnoseCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum DiagnoseCommand {
    /// Replay a captured room log against a byte copy of a client store and
    /// emit the rejected-entry classification record.
    RejectedEntry(DiagnoseRejectedEntryArgs),
}

#[derive(Debug, Args)]
pub(crate) struct DiagnoseRejectedEntryArgs {
    /// A COPY of the client store sqlite file (only read; byte-copied
    /// again internally).
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) store: String,

    /// Scratch directory for the replay copy.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) work_dir: String,

    /// Captured logs JSON (from `finitechat capture room-log`).
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) room_log: String,

    /// Device id to replay as.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) device_id: String,

    /// 64-char lowercase hex account secret.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) account_secret_hex: String,

    /// Incident alias recorded in the classification record.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) incident_alias: String,

    /// Clock override for the replay, Unix seconds (default now).
    #[arg(long)]
    pub(crate) now_unix_seconds: Option<u64>,
}

// --- repair ---

#[derive(Debug, Args)]
pub(crate) struct RepairArgs {
    #[command(subcommand)]
    pub(crate) command: RepairCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum RepairCommand {
    /// The only sanctioned way to advance a durable room cursor past a
    /// rejected entry: rehearse against byte copies, derive the skip list
    /// from the classification replay, then apply to the real store.
    SkipEntry(RepairSkipEntryArgs),
}

#[derive(Debug, Args)]
pub(crate) struct RepairSkipEntryArgs {
    /// The REAL client store sqlite file (phase 2 writes it).
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) store: String,

    /// Scratch directory for rehearsal copies.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) work_dir: String,

    /// Captured logs JSON (from `finitechat capture room-log`).
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) room_log: String,

    /// Device id to repair as.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) device_id: String,

    /// 64-char lowercase hex account secret.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) account_secret_hex: String,

    /// Incident alias recorded in the audit trail.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) incident_alias: String,

    /// Append-only JSONL audit trail (created mode 0600); must not be
    /// inside --work-dir.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) audit_log: String,

    /// Bound on derived skips (default 16, hard cap 64).
    #[arg(long, default_value_t = DEFAULT_REPAIR_MAX_SKIPS)]
    pub(crate) max_skips: u32,
}

// --- hermes ---

#[derive(Debug, Args)]
pub(crate) struct HermesArgs {
    /// Durable agent home directory (default: $FINITE_AGENT_HOME, then
    /// $FINITECHAT_HOME, then ~/.finite/agent).
    #[arg(long = "agent-home", global = true, allow_hyphen_values = true)]
    pub(crate) agent_home: Option<String>,

    /// Machine-readable JSON output for the commands that support it.
    #[arg(long, global = true)]
    pub(crate) json: bool,

    /// Read the request JSON from this string instead of stdin.
    #[arg(long, global = true, allow_hyphen_values = true)]
    pub(crate) request_json: Option<String>,

    #[command(subcommand)]
    pub(crate) command: HermesCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum HermesCommand {
    /// Initialize an agent home against a server using the shared Finite
    /// identity.
    Init(HermesInitArgs),

    /// Install the Finite Chat Hermes plugin into a Hermes plugins
    /// directory.
    Install(HermesInstallArgs),

    /// Serve the resident Hermes HTTP service and stream bridge.
    Serve(HermesServeArgs),

    /// Show, set, or clear the agent home channel.
    HomeChannel {
        #[command(subcommand)]
        command: HermesHomeChannelCommand,
    },

    /// Manage chat room admission (who may add this agent to rooms).
    Admission {
        #[command(subcommand)]
        command: HermesAdmissionCommand,
    },

    /// Report one room's connection/pairing status.
    RoomStatus(HermesRoomStatusArgs),

    /// Poll inbound Hermes events ({room_id?, limit?, timeout_millis?}).
    Poll,

    /// Ack delivered events (HermesAckRequestV1).
    Ack,

    /// Release a leased-but-unprocessed event back to the inbox for
    /// redelivery (HermesAckRequestV1 shape: room_id, seq, message_id).
    Release,

    /// Send a Hermes message (HermesSendRequestV1).
    Send,

    /// Edit a Hermes message (HermesEditRequestV1).
    Edit,

    /// Recover interrupted Hermes turns.
    Recover,

    /// Report agent activity (HermesActivityRequestV1).
    Activity,
}

#[derive(Debug, Parser)]
pub(crate) struct HermesInitArgs {
    /// Base URL of the finitechat server.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) server: String,

    /// Device id for the agent (default "agent").
    #[arg(long, default_value = DEFAULT_HERMES_DEVICE_ID, allow_hyphen_values = true)]
    pub(crate) device_id: String,

    /// Agent profile display name.
    #[arg(long, default_value = DEFAULT_HERMES_AGENT_NAME, allow_hyphen_values = true)]
    pub(crate) agent_name: String,

    /// Agent profile about text.
    #[arg(long, default_value = DEFAULT_HERMES_AGENT_ABOUT, allow_hyphen_values = true)]
    pub(crate) agent_about: String,

    /// Agent profile picture URL (http(s)).
    #[arg(long, default_value = DEFAULT_HERMES_AGENT_PICTURE, allow_hyphen_values = true)]
    pub(crate) agent_picture_url: String,

    /// Do not publish the agent Nostr profile during init.
    #[arg(long)]
    pub(crate) skip_agent_profile: bool,
}

#[derive(Debug, Parser)]
pub(crate) struct HermesInstallArgs {
    /// Install directly into this plugin directory.
    #[arg(long, conflicts_with = "plugins_dir", allow_hyphen_values = true)]
    pub(crate) plugin_dir: Option<String>,

    /// Install as `plugins-dir/<plugin-name>`.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) plugins_dir: Option<String>,

    /// Plugin (directory) name (default "finitechat").
    #[arg(long, default_value = DEFAULT_HERMES_PLUGIN_NAME, allow_hyphen_values = true)]
    pub(crate) plugin_name: String,

    /// finitechat binary the plugin env points at (default: this
    /// executable).
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) finitechat_bin: Option<String>,

    /// Hermes service URL advertised in the plugin env.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) service_url: Option<String>,

    /// Overwrite managed plugin files that were locally edited.
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Debug, Args)]
pub(crate) struct HermesServeArgs {
    /// Listen address (default 127.0.0.1:0).
    #[arg(long, default_value = DEFAULT_HERMES_SERVICE_ADDR)]
    pub(crate) addr: SocketAddr,

    /// Write the ready record (with the bound URL) to this file.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) ready_file: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct HermesRoomStatusArgs {
    /// Room to report on.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) room_id: String,
}

#[derive(Debug, Subcommand)]
pub(crate) enum HermesHomeChannelCommand {
    /// Print the current home channel.
    Show,

    /// Set the home channel room (and optional conversation).
    Set(HermesHomeChannelSetArgs),

    /// Clear the home channel.
    Clear,
}

#[derive(Debug, Subcommand)]
pub(crate) enum HermesAdmissionCommand {
    /// Run the admission birth-seed step: consume the environment seed into
    /// the store's Welcome admission policy exactly once (a store with a
    /// policy row only refreshes the gateway's allowed-users mirror), then
    /// print the current admission state. Safe to run repeatedly; agentd runs
    /// it before starting the gateway and sidecar, and `hermes serve` runs it
    /// again at boot.
    Seed,
}

#[derive(Debug, Args)]
pub(crate) struct HermesHomeChannelSetArgs {
    /// Room id to set as the home channel.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) room_id: String,

    /// Optional conversation id within the room.
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) conversation_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("finitechat").chain(args.iter().copied()))
            .unwrap_or_else(|error| panic!("finitechat {args:?} must parse: {error}"))
    }

    fn parse_err(args: &[&str]) -> String {
        Cli::try_parse_from(std::iter::once("finitechat").chain(args.iter().copied()))
            .expect_err("finitechat {args:?} must be a usage error")
            .render()
            .to_string()
    }

    fn hermes(args: &[&str]) -> HermesArgs {
        match parse(args).command {
            Command::Hermes(args) => args,
            other => panic!("expected hermes, got {other:?}"),
        }
    }

    /// Every invocation form below is copied from a real caller: the
    /// embedded Hermes Python adapter (integrations/hermes/finitechat/
    /// adapter.py), the canary/docker smoke scripts under finitechat/
    /// scripts/, .github/workflows/runtime-image.yml, and current local
    /// integration docs. If one of these stops parsing, operator
    /// tooling breaks — keep them green.

    #[test]
    fn removed_home_alias_is_rejected() {
        // `--home` was a hidden compatibility alias of `--agent-home` kept
        // for the Hermes Python adapter; every in-repo caller now spells
        // `--agent-home`, so the alias is gone. A stale caller (e.g. an
        // adapter copy installed by an older CLI) must fail loudly with a
        // usage error instead of silently acting on a different home.
        let stderr = parse_err(&["hermes", "--home", "/agent/home", "poll", "--json"]);
        assert!(stderr.contains("--home"), "{stderr:?}");
    }

    #[test]
    fn adapter_service_spawn_form_parses() {
        // adapter.py _ensure_service: spawn the resident service.
        let args = hermes(&[
            "hermes",
            "--agent-home",
            "/agent/home",
            "serve",
            "--addr",
            "127.0.0.1:8390",
            "--ready-file",
            "/agent/home/service-ready.json",
            "--json",
        ]);
        assert_eq!(args.agent_home.as_deref(), Some("/agent/home"));
        assert!(args.json);
        let HermesCommand::Serve(serve) = args.command else {
            panic!("expected serve");
        };
        assert_eq!(serve.addr.to_string(), "127.0.0.1:8390");
        assert_eq!(
            serve.ready_file.as_deref(),
            Some("/agent/home/service-ready.json")
        );
    }

    #[test]
    fn adapter_cli_fallback_action_forms_parse() {
        // adapter.py _finitechat_json fallback: `<bin> hermes --agent-home H
        // <action> --json` with the request on stdin.
        for action in ["poll", "ack", "send", "edit", "recover", "activity"] {
            let args = hermes(&["hermes", "--agent-home", "/agent/home", action, "--json"]);
            assert!(args.json, "{action} must see --json");
            assert!(args.request_json.is_none());
        }

        // hermes_flow.rs request form: --request-json replaces stdin.
        let args = hermes(&[
            "hermes",
            "--agent-home",
            "/agent/home",
            "poll",
            "--request-json",
            r#"{"timeout_millis":1000}"#,
        ]);
        assert_eq!(
            args.request_json.as_deref(),
            Some(r#"{"timeout_millis":1000}"#)
        );
        assert!(matches!(args.command, HermesCommand::Poll));
    }

    #[test]
    fn canary_install_forms_parse() {
        // hermes-real-gateway-admission-smoke.py and hermes-phone-canary.py.
        let args = hermes(&[
            "hermes",
            "--agent-home",
            "/agent/home",
            "install",
            "--plugins-dir",
            "/hermes/plugins",
            "--plugin-name",
            "finitechat",
            "--finitechat-bin",
            "/usr/local/bin/finitechat",
            "--service-url",
            "http://127.0.0.1:8390",
            "--force",
            "--json",
        ]);
        let HermesCommand::Install(install) = args.command else {
            panic!("expected install");
        };
        assert_eq!(install.plugins_dir.as_deref(), Some("/hermes/plugins"));
        assert_eq!(install.plugin_name, "finitechat");
        assert_eq!(
            install.finitechat_bin.as_deref(),
            Some("/usr/local/bin/finitechat")
        );
        assert_eq!(
            install.service_url.as_deref(),
            Some("http://127.0.0.1:8390")
        );
        assert!(install.force);
        assert!(install.plugin_dir.is_none());
        assert!(args.json);
    }

    #[test]
    fn docker_smoke_forms_parse() {
        // hermes-durable-home-docker-smoke.py / hermes-remote-docker-canary.py:
        // docker exec ... finitechat app --data-dir ... state (no
        // --start-runtime, deliberately).
        let args = parse(&[
            "app",
            "--data-dir",
            "/home/node/.finitechat/agent",
            "--server",
            "http://server",
            "--device-id",
            "durable-docker",
            "state",
        ]);
        let Command::App(app) = args.command else {
            panic!("expected app");
        };
        assert_eq!(app.data_dir, "/home/node/.finitechat/agent");
        assert_eq!(app.server, "http://server");
        assert_eq!(app.device_id, "durable-docker");
        assert!(app.now.is_none());
        let AppCommand::State {
            start_runtime,
            wait_update_ms,
            room_id,
        } = app.command
        else {
            panic!("expected app state");
        };
        assert!(!start_runtime);
        assert!(wait_update_ms.is_none());
        assert!(room_id.is_none());

        // The docker hermes wrapper runs `finitechat hermes --agent-home ...`.
        let args = hermes(&[
            "hermes",
            "--agent-home",
            "/home/node/.finitechat/agent",
            "room-status",
            "--room-id",
            "room",
        ]);
        let HermesCommand::RoomStatus(status) = args.command else {
            panic!("expected room-status");
        };
        assert_eq!(status.room_id, "room");
    }

    #[test]
    fn documented_forms_parse() {
        // Current local integration documentation operator quickstart.
        assert!(matches!(
            parse(&["auth", "status"]).command,
            Command::Auth(AuthArgs {
                command: AuthCommand::Status
            })
        ));

        let args = hermes(&["hermes", "init", "--server", "https://chat.finite.computer"]);
        let HermesCommand::Init(init) = args.command else {
            panic!("expected init");
        };
        assert_eq!(init.server, "https://chat.finite.computer");
        assert_eq!(init.device_id, "agent");
        assert_eq!(init.agent_name, "Finite Agent");
        assert!(!init.skip_agent_profile);

        // hermes_flow.rs init form, flags after the subcommand.
        let args = hermes(&[
            "hermes",
            "--agent-home",
            "/agent/home",
            "init",
            "--server",
            "http://127.0.0.1:1",
            "--device-id",
            "agent",
            "--skip-agent-profile",
            "--json",
        ]);
        let HermesCommand::Init(init) = args.command else {
            panic!("expected init");
        };
        assert!(init.skip_agent_profile);
        assert!(args.json);

        let args = hermes(&["hermes", "install"]);
        let HermesCommand::Install(install) = args.command else {
            panic!("expected install");
        };
        assert_eq!(install.plugin_name, "finitechat");
        assert!(install.plugin_dir.is_none() && install.plugins_dir.is_none());
    }

    #[test]
    fn family_options_parse_before_or_after_the_subcommand() {
        // The previous hand-rolled parser scraped family-level options from
        // anywhere in the family's arguments; global args keep that.
        for http_args in [
            vec!["http", "--server", "http://x", "health"],
            vec!["http", "health", "--server", "http://x"],
        ] {
            let args = parse(&http_args);
            let Command::Http(http) = args.command else {
                panic!("expected http");
            };
            assert_eq!(http.server, "http://x");
            assert!(matches!(http.command, HttpCommand::Health));
        }

        // Default server when --server is absent.
        let args = parse(&["http", "health"]);
        let Command::Http(http) = args.command else {
            panic!("expected http");
        };
        assert_eq!(http.server, crate::DEFAULT_SERVER_URL);

        let args = parse(&["app", "state", "--data-dir", "/tmp/x", "--start-runtime"]);
        let Command::App(app) = args.command else {
            panic!("expected app");
        };
        assert_eq!(app.data_dir, "/tmp/x");
        assert!(matches!(
            app.command,
            AppCommand::State {
                start_runtime: true,
                ..
            }
        ));

        let error = parse_err(&["http", "claim-key-packages", "--idempotency-key", "k"]);
        assert!(error.contains("--owner"), "unexpected error: {error}");
    }

    #[test]
    fn app_subcommand_classes_pin_the_smoke_harness_forms() {
        // The durable smoke's probe exec (`app state`, no flags — the exact
        // form hermes-durable-home-docker-smoke.py runs against the agent
        // container) is the read-only command class; every setup/action form
        // is a writer. The classes select the runtime open mode in app::run.
        fn class(args: &[&str]) -> CommandClass {
            match parse(args).command {
                Command::App(args) => args.command.command_class(),
                other => panic!("expected app, got {other:?}"),
            }
        }

        assert_eq!(
            class(&[
                "app",
                "--data-dir",
                "/home/node/.finitechat/agent",
                "--server",
                "http://server",
                "--device-id",
                "durable-docker",
                "state",
            ]),
            CommandClass::ReadOnly
        );
        for writer_form in [
            &["app", "state", "--start-runtime"][..],
            &["app", "state", "--wait-update-ms", "4000"][..],
            &["app", "state", "--room-id", "room"][..],
            &["app", "identity"][..],
            &["app", "start"][..],
            &["app", "wait"][..],
            &["app", "stop"][..],
            &["app", "open-room", "--room-id", "room"][..],
            &["app", "create-room"][..],
            &[
                "app",
                "add-member",
                "--room-id",
                "room",
                "--account-id",
                "acct",
            ][..],
            &["app", "scan", "--value", "npub"][..],
            &["app", "send", "--room-id", "room", "--text", "hi"][..],
            &["app", "mark-read", "--room-id", "room"][..],
            &["app", "refresh-devices"][..],
        ] {
            assert_eq!(class(writer_form), CommandClass::Writer, "{writer_form:?}");
        }
    }

    #[test]
    fn version_and_help_forms_succeed_through_run() {
        for form in [["--version"], ["-V"], ["version"]] {
            let mut output = Vec::new();
            crate::run(form.map(str::to_owned), &mut output).expect("version form succeeds");
            assert_eq!(
                String::from_utf8(output).unwrap(),
                format!("finitechat {}\n", env!("CARGO_PKG_VERSION"))
            );
        }
        for form in [["--help"], ["-h"], ["help"]] {
            let mut output = Vec::new();
            crate::run(form.map(str::to_owned), &mut output).expect("help form succeeds");
            let help = String::from_utf8(output).unwrap();
            assert!(help.contains("Usage: finitechat"), "help: {help}");
            // runtime-image.yml runs `finitechat --help` as its install
            // check; the help must describe every family.
            for family in [
                "http", "auth", "hermes", "app", "capture", "diagnose", "repair",
            ] {
                assert!(help.contains(family), "help must mention {family}: {help}");
            }
        }
    }
}
