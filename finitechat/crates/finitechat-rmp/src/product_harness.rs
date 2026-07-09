use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use finitechat_core::{
    AppAction, AppOutboxDebugRow, AppProfileSummary, AppRoomState, AppState, FiniteChatRuntime,
    OpenOptions as CoreOpenOptions, OutboundLocalSendState, OutboundServerDeliveryState,
    npub_from_account_id,
};
use rusqlite::Connection;
use serde::Serialize;

use crate::cli::{CliError, JsonOk, ProductHarnessArgs, ProductHarnessPlatform, json_print};
use crate::product_store::{
    checked_path_component, product_store_root, product_store_root_dry_run,
    reset_product_store_root,
};
use crate::run::{self, IosDeviceInstalledApp, IosInstalledApp};

const HARNESS_SERVER_START_TIMEOUT: Duration = Duration::from_secs(180);

pub fn product_harness(
    root: &Path,
    json: bool,
    verbose: bool,
    args: ProductHarnessArgs,
) -> Result<(), CliError> {
    match args.platform {
        ProductHarnessPlatform::IosSimulator | ProductHarnessPlatform::IosDevice => {
            ios_product_harness(root, json, verbose, args)
        }
    }
}

fn ios_product_harness(
    root: &Path,
    json: bool,
    verbose: bool,
    args: ProductHarnessArgs,
) -> Result<(), CliError> {
    ios_product_harness_with_ios_development_team_env(
        root,
        json,
        verbose,
        args,
        std::env::var("RMP_IOS_DEVELOPMENT_TEAM").ok(),
    )
}

fn ios_product_harness_with_ios_development_team_env(
    root: &Path,
    json: bool,
    verbose: bool,
    args: ProductHarnessArgs,
    ios_development_team_env: Option<String>,
) -> Result<(), CliError> {
    let scenario = checked_path_component("scenario", &args.scenario)?;
    let device = checked_path_component("device", &args.device)?;
    if !matches!(scenario.as_str(), "text-offline" | "profile-dm") {
        return Err(CliError::user(format!(
            "unsupported product harness scenario `{scenario}`; expected `text-offline` or `profile-dm`"
        )));
    }
    let server_url = checked_server_url(&args.server_url)?;
    if matches!(args.platform, ProductHarnessPlatform::IosDevice) {
        checked_ios_device_server_url(&server_url)?;
    }
    let server_addr = match args.server_addr {
        Some(value) => checked_server_addr(&value)?,
        None if matches!(args.platform, ProductHarnessPlatform::IosDevice) => {
            ios_device_default_server_addr_from_url(&server_url)?
        }
        None => server_addr_from_url(&server_url)?,
    };
    if matches!(args.platform, ProductHarnessPlatform::IosDevice) {
        checked_ios_device_server_addr(&server_addr)?;
    }
    let server_probe_addr = server_probe_addr(&server_addr)?.to_string();
    let platform_label = product_harness_platform_label(args.platform).to_owned();
    let target_udid = checked_product_harness_udid(args.platform, args.udid)?;
    let ios_development_team = checked_product_harness_ios_development_team(
        args.platform,
        args.ios_development_team,
        ios_development_team_env,
    )?;
    let support_root = if args.dry_run {
        product_store_root_dry_run(root, "ios", &scenario, &device)?
    } else {
        product_store_root(root, "ios", &scenario, &device)?
    };
    let config_path = support_root.join("finitechat_config.json");
    let store_path = support_root.join("FiniteChatStore");
    let peer_store_path = support_root.join("PeerFiniteChatStore");
    let server_sqlite = support_root.join("server.sqlite3");
    let server_log = support_root.join("server.log");
    let peer_device = format!("{device}-peer");
    let phases = harness_phases(&scenario);

    if args.dry_run {
        render_harness_result(
            json,
            "dry-run",
            HarnessResult {
                platform: platform_label,
                scenario,
                device,
                server_url,
                server_addr,
                server_probe_addr,
                support_root,
                config_path,
                store_path,
                peer_store_path,
                server_sqlite,
                server_log,
                phases,
                udid: target_udid,
                ios_development_team,
                bundle_id: None,
                assertions: None,
                profile_dm_assertions: None,
            },
        );
        return Ok(());
    }

    if !args.no_reset {
        reset_product_store_root(root, "ios", &scenario, &device, verbose)?;
    } else {
        validate_existing_harness_config(&config_path, &server_url, &device)?;
    }
    fs::create_dir_all(&support_root).map_err(|error| {
        CliError::operational(format!(
            "failed to create harness support root {}: {error}",
            support_root.display()
        ))
    })?;
    write_harness_config(&config_path, &server_url, &device)?;

    let target = build_install_ios_target(
        root,
        verbose,
        args.platform,
        target_udid,
        ios_development_team.clone(),
        !args.no_reset,
    )?;

    let mut server = HarnessServer::start(root, &server_addr, &server_sqlite, &server_log)?;
    server.wait_until_ready(&server_addr, HARNESS_SERVER_START_TIMEOUT)?;
    if scenario == "profile-dm" {
        let assertions = run_profile_dm_harness(ProfileDmHarnessInput {
            target: &target,
            store_path: &store_path,
            peer_store_path: &peer_store_path,
            support_root: &support_root,
            server_url: &server_url,
            device: &device,
            peer_device: &peer_device,
            server_sqlite: &server_sqlite,
            verbose,
            settle_seconds: args.settle_seconds,
        })?;
        server.stop()?;
        render_harness_result(
            json,
            "asserted",
            HarnessResult {
                platform: platform_label,
                scenario,
                device,
                server_url,
                server_addr,
                server_probe_addr,
                support_root,
                config_path,
                store_path,
                peer_store_path,
                server_sqlite,
                server_log,
                phases,
                udid: Some(target.udid().to_owned()),
                ios_development_team,
                bundle_id: Some(target.bundle_id().to_owned()),
                assertions: None,
                profile_dm_assertions: Some(assertions),
            },
        );
        return Ok(());
    }
    let launch = launch_phase(
        &target,
        &support_root,
        &server_url,
        &device,
        &[
            "--finitechat-auto-create-room",
            "Product Harness",
            "--finitechat-auto-send",
            "online product harness message",
        ],
        verbose,
        args.settle_seconds,
    )?;

    terminate_phase(&target, launch, verbose)?;
    pull_device_store_if_needed(&target, &store_path, verbose)?;
    let after_online = assert_server_delivery_snapshot(
        &server_sqlite,
        1,
        &device,
        "after online create/send phase",
    )?;
    let key_packages_after_online = assert_server_key_package_availability(
        &server_sqlite,
        &device,
        1,
        "after online create/send phase",
    )?;
    let local_after_online = assert_local_projection_snapshot(
        &store_path,
        &server_url,
        &device,
        "after online create/send phase",
        LocalProjectionExpectation {
            expected_messages: 1,
            expected_delivered: 1,
            expected_undelivered: 0,
            required_delivered_message_ids: &[],
        },
    )?;
    let outbox_after_online = assert_local_outbox_snapshot(
        &store_path,
        &server_url,
        &device,
        "after online create/send phase",
        OutboxExpectation {
            expected_rows: 0,
            required_room_id: None,
            required_message_id: None,
            expected_local_state: None,
            expected_server_delivery_state: None,
        },
    )?;
    let online_message_id = local_after_online
        .delivered_message_ids
        .first()
        .cloned()
        .ok_or_else(|| {
            CliError::operational(
                "after online create/send phase: expected one delivered local message id"
                    .to_owned(),
            )
        })?;
    assert_server_snapshot_contains_message(
        &after_online,
        &online_message_id,
        "after online create/send phase",
    )?;
    let room_id = local_after_online.selected_room_id.clone().ok_or_else(|| {
        CliError::operational(
            "after online create/send phase: expected selected local room for peer Welcome"
                .to_owned(),
        )
    })?;
    establish_peer_membership(
        &store_path,
        &peer_store_path,
        &server_url,
        &device,
        &peer_device,
        &room_id,
        "after online create/send phase",
    )?;
    push_device_store_if_needed(&target, &store_path, verbose)?;
    server.stop()?;
    let launch = launch_phase(
        &target,
        &support_root,
        &server_url,
        &device,
        &["--finitechat-auto-send", "offline product harness message"],
        verbose,
        args.settle_seconds,
    )?;
    terminate_phase(&target, launch, verbose)?;
    pull_device_store_if_needed(&target, &store_path, verbose)?;
    let after_offline = assert_server_delivery_snapshot(
        &server_sqlite,
        1,
        &device,
        "after offline send with server stopped",
    )?;
    let local_after_offline = assert_local_projection_snapshot(
        &store_path,
        &server_url,
        &device,
        "after offline send with server stopped",
        LocalProjectionExpectation {
            expected_messages: 2,
            expected_delivered: 1,
            expected_undelivered: 1,
            required_delivered_message_ids: &[online_message_id.as_str()],
        },
    )?;
    let offline_message_id = local_after_offline
        .undelivered_message_ids
        .first()
        .cloned()
        .ok_or_else(|| {
            CliError::operational(
                "after offline send with server stopped: expected one undelivered local message id"
                    .to_owned(),
            )
        })?;
    assert_server_snapshot_excludes_message(
        &after_offline,
        &offline_message_id,
        "after offline send with server stopped",
    )?;
    let outbox_after_offline = assert_local_outbox_snapshot(
        &store_path,
        &server_url,
        &device,
        "after offline send with server stopped",
        OutboxExpectation {
            expected_rows: 1,
            required_room_id: local_after_offline.selected_room_id.as_deref(),
            required_message_id: Some(offline_message_id.as_str()),
            expected_local_state: Some("sent"),
            expected_server_delivery_state: Some("undelivered"),
        },
    )?;

    let launch = launch_phase(
        &target,
        &support_root,
        &server_url,
        &device,
        &[
            "--finitechat-auto-send-attachment-text",
            "offline attachment should fail fast",
        ],
        verbose,
        args.settle_seconds,
    )?;
    terminate_phase(&target, launch, verbose)?;
    pull_device_store_if_needed(&target, &store_path, verbose)?;
    let after_offline_attachment = assert_server_delivery_snapshot(
        &server_sqlite,
        1,
        &device,
        "after offline attachment fail-fast attempt",
    )?;
    assert_server_snapshot_excludes_message(
        &after_offline_attachment,
        &offline_message_id,
        "after offline attachment fail-fast attempt",
    )?;
    let local_after_offline_attachment = assert_local_projection_snapshot(
        &store_path,
        &server_url,
        &device,
        "after offline attachment fail-fast attempt",
        LocalProjectionExpectation {
            expected_messages: 2,
            expected_delivered: 1,
            expected_undelivered: 1,
            required_delivered_message_ids: &[online_message_id.as_str()],
        },
    )?;
    assert_local_projection_same_visible_outbound(
        &local_after_offline_attachment,
        &local_after_offline,
        "after offline attachment fail-fast attempt",
    )?;
    let outbox_after_offline_attachment = assert_local_outbox_snapshot(
        &store_path,
        &server_url,
        &device,
        "after offline attachment fail-fast attempt",
        OutboxExpectation {
            expected_rows: 1,
            required_room_id: local_after_offline_attachment.selected_room_id.as_deref(),
            required_message_id: Some(offline_message_id.as_str()),
            expected_local_state: Some("sent"),
            expected_server_delivery_state: Some("undelivered"),
        },
    )?;

    let mut server = HarnessServer::start(root, &server_addr, &server_sqlite, &server_log)?;
    server.wait_until_ready(&server_addr, HARNESS_SERVER_START_TIMEOUT)?;
    let launch = launch_phase(
        &target,
        &support_root,
        &server_url,
        &device,
        &[],
        verbose,
        args.settle_seconds,
    )?;
    terminate_phase(&target, launch, verbose)?;
    pull_device_store_if_needed(&target, &store_path, verbose)?;
    let after_restart = assert_server_delivery_snapshot(
        &server_sqlite,
        2,
        &device,
        "after same-url server restart/drain",
    )?;
    let local_after_restart = assert_local_projection_snapshot(
        &store_path,
        &server_url,
        &device,
        "after same-url server restart/drain",
        LocalProjectionExpectation {
            expected_messages: 2,
            expected_delivered: 2,
            expected_undelivered: 0,
            required_delivered_message_ids: &[
                online_message_id.as_str(),
                offline_message_id.as_str(),
            ],
        },
    )?;
    assert_server_snapshot_contains_message(
        &after_restart,
        &online_message_id,
        "after same-url server restart/drain",
    )?;
    assert_server_snapshot_contains_message(
        &after_restart,
        &offline_message_id,
        "after same-url server restart/drain",
    )?;
    let outbox_after_restart = assert_local_outbox_snapshot(
        &store_path,
        &server_url,
        &device,
        "after same-url server restart/drain",
        OutboxExpectation {
            expected_rows: 0,
            required_room_id: None,
            required_message_id: None,
            expected_local_state: None,
            expected_server_delivery_state: None,
        },
    )?;
    let peer_after_restart = assert_peer_delivery_snapshot(
        &peer_store_path,
        &server_url,
        &peer_device,
        &room_id,
        &device,
        &offline_message_id,
        "after same-url server restart/drain",
    )?;
    server.stop()?;
    let assertions = HarnessAssertions {
        after_online,
        key_packages_after_online,
        after_offline,
        after_offline_attachment,
        after_restart,
        local_after_online,
        local_after_offline,
        local_after_offline_attachment,
        local_after_restart,
        outbox_after_online,
        outbox_after_offline,
        outbox_after_offline_attachment,
        outbox_after_restart,
        peer_after_restart,
    };

    render_harness_result(
        json,
        "asserted",
        HarnessResult {
            platform: platform_label,
            scenario,
            device,
            server_url,
            server_addr,
            server_probe_addr,
            support_root,
            config_path,
            store_path,
            peer_store_path,
            server_sqlite,
            server_log,
            phases,
            udid: Some(target.udid().to_owned()),
            ios_development_team,
            bundle_id: Some(target.bundle_id().to_owned()),
            assertions: Some(assertions),
            profile_dm_assertions: None,
        },
    );
    Ok(())
}

struct ProfileDmHarnessInput<'a> {
    target: &'a HarnessIosTarget,
    store_path: &'a Path,
    peer_store_path: &'a Path,
    support_root: &'a Path,
    server_url: &'a str,
    device: &'a str,
    peer_device: &'a str,
    server_sqlite: &'a Path,
    verbose: bool,
    settle_seconds: u64,
}

fn run_profile_dm_harness(
    input: ProfileDmHarnessInput<'_>,
) -> Result<ProfileDmHarnessAssertions, CliError> {
    let ProfileDmHarnessInput {
        target,
        store_path,
        peer_store_path,
        support_root,
        server_url,
        device,
        peer_device,
        server_sqlite,
        verbose,
        settle_seconds,
    } = input;
    let peer = open_product_runtime(
        peer_store_path,
        server_url,
        peer_device,
        "profile DM peer preparation",
    )?;
    let peer_state = peer
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(|error| {
            CliError::operational(format!(
                "profile DM peer preparation: peer failed to publish key packages: {error}"
            ))
        })?;
    let peer_account_id = peer_state.identity.account_id;
    let peer_npub = npub_from_account_id(peer_account_id.clone()).map_err(|error| {
        CliError::operational(format!(
            "profile DM peer preparation: failed to encode peer npub: {error}"
        ))
    })?;

    let launch = launch_phase(
        target,
        support_root,
        server_url,
        device,
        &[
            "--finitechat-auto-start-profile-chat-npub",
            peer_npub.as_str(),
            "--finitechat-auto-send",
            "profile dm product harness message",
        ],
        verbose,
        settle_seconds,
    )?;
    terminate_phase(target, launch, verbose)?;
    pull_device_store_if_needed(target, store_path, verbose)?;

    let after_profile_dm = assert_server_delivery_snapshot(
        server_sqlite,
        1,
        device,
        "after profile DM start/send phase",
    )?;
    let local_after_profile_dm = assert_local_projection_snapshot(
        store_path,
        server_url,
        device,
        "after profile DM start/send phase",
        LocalProjectionExpectation {
            expected_messages: 1,
            expected_delivered: 1,
            expected_undelivered: 0,
            required_delivered_message_ids: &[],
        },
    )?;
    let message_id = local_after_profile_dm
        .delivered_message_ids
        .first()
        .cloned()
        .ok_or_else(|| {
            CliError::operational(
                "after profile DM start/send phase: expected one delivered message id".to_owned(),
            )
        })?;
    assert_server_snapshot_contains_message(
        &after_profile_dm,
        &message_id,
        "after profile DM start/send phase",
    )?;
    let room_id = local_after_profile_dm
        .selected_room_id
        .clone()
        .ok_or_else(|| {
            CliError::operational(
                "after profile DM start/send phase: expected selected direct room".to_owned(),
            )
        })?;
    let peer_after_profile_dm = assert_peer_delivery_snapshot(
        peer_store_path,
        server_url,
        peer_device,
        &room_id,
        device,
        &message_id,
        "after profile DM start/send phase",
    )?;

    Ok(ProfileDmHarnessAssertions {
        after_profile_dm,
        local_after_profile_dm,
        peer_after_profile_dm,
        peer_account_id,
        peer_npub,
    })
}

struct HarnessResult {
    platform: String,
    scenario: String,
    device: String,
    server_url: String,
    server_addr: String,
    server_probe_addr: String,
    support_root: PathBuf,
    config_path: PathBuf,
    store_path: PathBuf,
    peer_store_path: PathBuf,
    server_sqlite: PathBuf,
    server_log: PathBuf,
    phases: Vec<&'static str>,
    udid: Option<String>,
    ios_development_team: Option<String>,
    bundle_id: Option<String>,
    assertions: Option<HarnessAssertions>,
    profile_dm_assertions: Option<ProfileDmHarnessAssertions>,
}

#[derive(Clone, Debug, Serialize)]
struct HarnessAssertions {
    after_online: ServerDeliverySnapshot,
    key_packages_after_online: KeyPackageAvailabilitySnapshot,
    after_offline: ServerDeliverySnapshot,
    after_offline_attachment: ServerDeliverySnapshot,
    after_restart: ServerDeliverySnapshot,
    local_after_online: LocalProjectionSnapshot,
    local_after_offline: LocalProjectionSnapshot,
    local_after_offline_attachment: LocalProjectionSnapshot,
    local_after_restart: LocalProjectionSnapshot,
    outbox_after_online: LocalOutboxSnapshot,
    outbox_after_offline: LocalOutboxSnapshot,
    outbox_after_offline_attachment: LocalOutboxSnapshot,
    outbox_after_restart: LocalOutboxSnapshot,
    peer_after_restart: PeerReceiptSnapshot,
}

#[derive(Clone, Debug, Serialize)]
struct ProfileDmHarnessAssertions {
    after_profile_dm: ServerDeliverySnapshot,
    local_after_profile_dm: LocalProjectionSnapshot,
    peer_after_profile_dm: PeerReceiptSnapshot,
    peer_account_id: String,
    peer_npub: String,
}

#[derive(Clone, Debug, Serialize)]
struct KeyPackageAvailabilitySnapshot {
    device_id: String,
    account_ids: Vec<String>,
    available: i64,
    claimed: i64,
    consumed: i64,
}

#[derive(Clone, Debug, Serialize)]
struct ServerDeliverySnapshot {
    publish_messages: i64,
    application_delivery_effects: i64,
    publish_idempotency_rows: i64,
    rooms: Vec<String>,
    message_ids: Vec<String>,
    sender_devices: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct LocalProjectionSnapshot {
    rooms: usize,
    connected_rooms: usize,
    unavailable_on_device_rooms: usize,
    selected_room_id: Option<String>,
    messages: usize,
    local_outbound_messages: usize,
    delivered_outbound_messages: usize,
    undelivered_outbound_messages: usize,
    failed_outbound_messages: usize,
    sending_outbound_messages: usize,
    nonlocal_outbound_delivery_messages: usize,
    delivered_message_ids: Vec<String>,
    undelivered_message_ids: Vec<String>,
    visible_outbound_message_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct LocalOutboxSnapshot {
    rows: usize,
    device_ids: Vec<String>,
    room_ids: Vec<String>,
    message_ids: Vec<String>,
    local_states: Vec<String>,
    server_delivery_states: Vec<String>,
    append_request_message_ids: Vec<String>,
    idempotency_material_rows: usize,
}

struct LocalOutboxKeyRow {
    device_id: String,
    room_id: String,
    message_id: String,
}

#[derive(Clone, Debug, Serialize)]
struct PeerReceiptSnapshot {
    room_id: String,
    peer_device: String,
    message_id: String,
    total_peer_messages: usize,
    matching_messages: usize,
    inbound_matching_messages: usize,
    local_matching_messages: usize,
    matching_with_outbound_delivery: usize,
    sender_devices: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
struct LocalProjectionExpectation<'a> {
    expected_messages: usize,
    expected_delivered: usize,
    expected_undelivered: usize,
    required_delivered_message_ids: &'a [&'a str],
}

#[derive(Clone, Copy, Debug)]
struct OutboxExpectation<'a> {
    expected_rows: usize,
    required_room_id: Option<&'a str>,
    required_message_id: Option<&'a str>,
    expected_local_state: Option<&'a str>,
    expected_server_delivery_state: Option<&'a str>,
}

enum HarnessIosTarget {
    Simulator(IosInstalledApp),
    Device(IosDeviceInstalledApp),
}

impl HarnessIosTarget {
    fn udid(&self) -> &str {
        match self {
            Self::Simulator(installed) => &installed.udid,
            Self::Device(installed) => &installed.udid,
        }
    }

    fn bundle_id(&self) -> &str {
        match self {
            Self::Simulator(installed) => &installed.bundle_id,
            Self::Device(installed) => &installed.bundle_id,
        }
    }
}

enum HarnessLaunch {
    Simulator,
    Device { pid: u64 },
}

fn render_harness_result(json: bool, status: &str, result: HarnessResult) {
    if json {
        json_print(&JsonOk {
            ok: true,
            data: serde_json::json!({
                "status": status,
                "platform": result.platform,
                "scenario": result.scenario,
                "device": result.device,
                "server_url": result.server_url,
                "server_addr": result.server_addr,
                "server_probe_addr": result.server_probe_addr,
                "support_root": result.support_root,
                "config_path": result.config_path,
                "store_path": result.store_path,
                "peer_store_path": result.peer_store_path,
                "server_sqlite": result.server_sqlite,
                "server_log": result.server_log,
                "phases": result.phases,
                "udid": result.udid,
                "ios_development_team": result.ios_development_team,
                "bundle_id": result.bundle_id,
                "assertions": result.assertions,
                "profile_dm_assertions": result.profile_dm_assertions,
            }),
        });
        return;
    }

    eprintln!("ok: product harness {status}");
    eprintln!("platform: {}", result.platform);
    eprintln!("scenario: {}", result.scenario);
    eprintln!("device: {}", result.device);
    eprintln!("server url: {}", result.server_url);
    eprintln!("server addr: {}", result.server_addr);
    eprintln!("server probe addr: {}", result.server_probe_addr);
    eprintln!("support root: {}", result.support_root.display());
    eprintln!("store path: {}", result.store_path.display());
    eprintln!("peer store path: {}", result.peer_store_path.display());
    eprintln!("server sqlite: {}", result.server_sqlite.display());
    eprintln!("server log: {}", result.server_log.display());
    eprintln!("phases: {}", result.phases.join(", "));
    if let Some(udid) = result.udid {
        eprintln!("udid: {udid}");
    }
    if let Some(team) = result.ios_development_team {
        eprintln!("ios development team: {team}");
    }
    if let Some(assertions) = result.assertions {
        eprintln!(
            "assertions: publish messages online={}, offline={}, after offline attachment={}, after restart={}",
            assertions.after_online.publish_messages,
            assertions.after_offline.publish_messages,
            assertions.after_offline_attachment.publish_messages,
            assertions.after_restart.publish_messages
        );
        eprintln!(
            "assertions: key packages for {} after online launch: available={}, claimed={}, consumed={}, accounts={}",
            assertions.key_packages_after_online.device_id,
            assertions.key_packages_after_online.available,
            assertions.key_packages_after_online.claimed,
            assertions.key_packages_after_online.consumed,
            assertions.key_packages_after_online.account_ids.join(",")
        );
        eprintln!(
            "assertions: final app effects={}, idempotency rows={}, rooms={}, sender devices={}",
            assertions.after_restart.application_delivery_effects,
            assertions.after_restart.publish_idempotency_rows,
            assertions.after_restart.rooms.join(","),
            assertions.after_restart.sender_devices.join(",")
        );
        eprintln!(
            "assertions: server accepted message ids={}",
            assertions.after_restart.message_ids.join(",")
        );
        eprintln!(
            "assertions: local messages online={}, offline={}, after offline attachment={}, after restart={}; undelivered offline={}; final delivered ids={}",
            assertions.local_after_online.messages,
            assertions.local_after_offline.messages,
            assertions.local_after_offline_attachment.messages,
            assertions.local_after_restart.messages,
            assertions
                .local_after_offline
                .undelivered_message_ids
                .join(","),
            assertions
                .local_after_restart
                .delivered_message_ids
                .join(",")
        );
        eprintln!(
            "assertions: outbox rows online={}, offline={}, after offline attachment={}, after restart={}; offline ids={}; states={}/{}; idempotency material rows={}",
            assertions.outbox_after_online.rows,
            assertions.outbox_after_offline.rows,
            assertions.outbox_after_offline_attachment.rows,
            assertions.outbox_after_restart.rows,
            assertions.outbox_after_offline.message_ids.join(","),
            assertions.outbox_after_offline.local_states.join(","),
            assertions
                .outbox_after_offline
                .server_delivery_states
                .join(","),
            assertions.outbox_after_offline.idempotency_material_rows
        );
        eprintln!(
            "assertions: peer received message {} exactly {} time(s) as inbound from {}",
            assertions.peer_after_restart.message_id,
            assertions.peer_after_restart.inbound_matching_messages,
            assertions.peer_after_restart.sender_devices.join(",")
        );
    }
    if let Some(assertions) = result.profile_dm_assertions {
        eprintln!(
            "assertions: profile DM server messages={}, rooms={}, peer={}",
            assertions.after_profile_dm.application_delivery_effects,
            assertions.after_profile_dm.rooms.join(","),
            assertions.peer_npub
        );
        eprintln!(
            "assertions: profile DM local messages={}, delivered ids={}",
            assertions.local_after_profile_dm.messages,
            assertions
                .local_after_profile_dm
                .delivered_message_ids
                .join(",")
        );
        eprintln!(
            "assertions: profile DM peer received message {} exactly {} time(s) as inbound from {}",
            assertions.peer_after_profile_dm.message_id,
            assertions.peer_after_profile_dm.inbound_matching_messages,
            assertions.peer_after_profile_dm.sender_devices.join(",")
        );
    }
}

fn build_install_ios_target(
    root: &Path,
    verbose: bool,
    platform: ProductHarnessPlatform,
    udid: Option<String>,
    development_team: Option<String>,
    reset_app: bool,
) -> Result<HarnessIosTarget, CliError> {
    let args = crate::cli::RunIosArgs {
        udid,
        development_team,
    };
    match platform {
        ProductHarnessPlatform::IosSimulator => {
            run::build_install_ios_simulator(root, verbose, args, false)
                .map(HarnessIosTarget::Simulator)
        }
        ProductHarnessPlatform::IosDevice => {
            run::build_install_ios_device(root, verbose, args, false, reset_app)
                .map(HarnessIosTarget::Device)
        }
    }
}

fn launch_phase(
    target: &HarnessIosTarget,
    support_root: &Path,
    server_url: &str,
    device: &str,
    phase_args: &[&str],
    verbose: bool,
    settle_seconds: u64,
) -> Result<HarnessLaunch, CliError> {
    let mut launch_args = Vec::new();
    if matches!(target, HarnessIosTarget::Simulator(_)) {
        launch_args.extend([
            "--finitechat-product-harness-root".to_owned(),
            support_root.display().to_string(),
        ]);
    }
    launch_args.extend([
        "--finitechat-server".to_owned(),
        server_url.to_owned(),
        "--finitechat-device".to_owned(),
        device.to_owned(),
        "--finitechat-persist-launch-config".to_owned(),
    ]);
    launch_args.extend(phase_args.iter().map(|arg| (*arg).to_owned()));
    let launch = match target {
        HarnessIosTarget::Simulator(installed) => {
            run::launch_ios_simulator_app(
                &installed.dev_dir,
                &installed.udid,
                &installed.bundle_id,
                &launch_args,
                verbose,
            )?;
            HarnessLaunch::Simulator
        }
        HarnessIosTarget::Device(installed) => {
            let pid = run::launch_ios_device_app(
                &installed.dev_dir,
                &installed.udid,
                &installed.bundle_id,
                &launch_args,
                verbose,
            )?;
            HarnessLaunch::Device { pid }
        }
    };
    thread::sleep(Duration::from_secs(settle_seconds));
    Ok(launch)
}

fn terminate_phase(
    target: &HarnessIosTarget,
    launch: HarnessLaunch,
    verbose: bool,
) -> Result<(), CliError> {
    match (target, launch) {
        (HarnessIosTarget::Simulator(installed), HarnessLaunch::Simulator) => {
            run::terminate_ios_simulator_app(
                &installed.dev_dir,
                &installed.udid,
                &installed.bundle_id,
                verbose,
            )
        }
        (HarnessIosTarget::Device(installed), HarnessLaunch::Device { pid }) => {
            run::terminate_ios_device_app(&installed.dev_dir, &installed.udid, pid, verbose)
        }
        _ => Err(CliError::operational(
            "product harness launch target did not match terminate target",
        )),
    }
}

fn pull_device_store_if_needed(
    target: &HarnessIosTarget,
    store_path: &Path,
    verbose: bool,
) -> Result<(), CliError> {
    match target {
        HarnessIosTarget::Simulator(_) => Ok(()),
        HarnessIosTarget::Device(installed) => run::pull_ios_device_app_store(
            &installed.dev_dir,
            &installed.udid,
            &installed.bundle_id,
            store_path,
            verbose,
        ),
    }
}

fn push_device_store_if_needed(
    target: &HarnessIosTarget,
    store_path: &Path,
    verbose: bool,
) -> Result<(), CliError> {
    match target {
        HarnessIosTarget::Simulator(_) => Ok(()),
        HarnessIosTarget::Device(installed) => run::push_ios_device_app_store(
            &installed.dev_dir,
            &installed.udid,
            &installed.bundle_id,
            store_path,
            verbose,
        ),
    }
}

struct HarnessServer {
    child: Child,
}

impl HarnessServer {
    fn start(
        root: &Path,
        server_addr: &str,
        sqlite_path: &Path,
        log_path: &Path,
    ) -> Result<Self, CliError> {
        let probe_addr = server_probe_addr(server_addr)?;
        if server_addr_is_reachable(probe_addr) {
            return Err(CliError::operational(format!(
                "server address {server_addr} is already reachable at probe address {probe_addr}; refusing to run product harness against a pre-existing server"
            )));
        }
        if let Some(parent) = log_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                CliError::operational(format!(
                    "failed to create server log parent {}: {error}",
                    parent.display()
                ))
            })?;
        }
        let stdout = append_log_file(log_path)?;
        let stderr = append_log_file(log_path)?;
        let child = Command::new("cargo")
            .current_dir(root)
            .arg("run")
            .arg("-p")
            .arg("finitechat-server")
            .arg("--")
            .arg("serve")
            .arg(server_addr)
            .arg("--sqlite")
            .arg(sqlite_path)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(|error| {
                CliError::operational(format!("failed to start finitechat-server: {error}"))
            })?;
        Ok(Self { child })
    }

    fn wait_until_ready(&mut self, server_addr: &str, timeout: Duration) -> Result<(), CliError> {
        let probe_addr = server_probe_addr(server_addr)?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if server_addr_is_reachable(probe_addr) {
                return Ok(());
            }
            if let Some(status) = self.child.try_wait().map_err(|error| {
                CliError::operational(format!("failed to inspect finitechat-server: {error}"))
            })? {
                return Err(CliError::operational(format!(
                    "finitechat-server exited before becoming reachable at {server_addr} via probe address {probe_addr} (status {status}); see harness server log"
                )));
            }
            thread::sleep(Duration::from_millis(250));
        }
        Err(CliError::operational(format!(
            "finitechat-server did not become reachable at {server_addr} via probe address {probe_addr}"
        )))
    }

    fn stop(&mut self) -> Result<(), CliError> {
        if let Some(_status) = self.child.try_wait().map_err(|error| {
            CliError::operational(format!("failed to inspect finitechat-server: {error}"))
        })? {
            return Ok(());
        }
        self.child
            .kill()
            .map_err(|error| CliError::operational(format!("failed to stop server: {error}")))?;
        self.child.wait().map_err(|error| {
            CliError::operational(format!("failed to wait for server: {error}"))
        })?;
        Ok(())
    }
}

impl Drop for HarnessServer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn append_log_file(path: &Path) -> Result<File, CliError> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            CliError::operational(format!(
                "failed to open server log {}: {error}",
                path.display()
            ))
        })
}

fn server_probe_addr(server_addr: &str) -> Result<SocketAddr, CliError> {
    let addr: SocketAddr = server_addr.parse().map_err(|error| {
        CliError::user(format!(
            "server address `{server_addr}` is invalid: {error}"
        ))
    })?;
    let probe_ip = match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        other => other,
    };
    Ok(SocketAddr::new(probe_ip, addr.port()))
}

fn server_addr_is_reachable(addr: SocketAddr) -> bool {
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

fn assert_server_delivery_snapshot(
    sqlite_path: &Path,
    expected_application_messages: i64,
    expected_device: &str,
    label: &str,
) -> Result<ServerDeliverySnapshot, CliError> {
    let snapshot = read_server_delivery_snapshot(sqlite_path)?;
    if snapshot.application_delivery_effects != expected_application_messages {
        return Err(CliError::operational(format!(
            "{label}: expected {expected_application_messages} application delivery effect row(s), got {}",
            snapshot.application_delivery_effects
        )));
    }
    if snapshot.publish_messages < expected_application_messages {
        return Err(CliError::operational(format!(
            "{label}: expected at least {expected_application_messages} accepted publish_message row(s), got {}",
            snapshot.publish_messages
        )));
    }
    if snapshot.publish_idempotency_rows < expected_application_messages {
        return Err(CliError::operational(format!(
            "{label}: expected at least {expected_application_messages} publish idempotency row(s), got {}",
            snapshot.publish_idempotency_rows
        )));
    }
    if expected_application_messages > 0 && snapshot.rooms.len() != 1 {
        return Err(CliError::operational(format!(
            "{label}: expected exactly one delivered room, got {:?}",
            snapshot.rooms
        )));
    }
    if expected_application_messages > 0
        && snapshot.sender_devices != vec![expected_device.to_owned()]
    {
        return Err(CliError::operational(format!(
            "{label}: expected sender device `{expected_device}`, got {:?}",
            snapshot.sender_devices
        )));
    }
    Ok(snapshot)
}

fn assert_server_key_package_availability(
    sqlite_path: &Path,
    expected_device: &str,
    min_available: i64,
    label: &str,
) -> Result<KeyPackageAvailabilitySnapshot, CliError> {
    let snapshot = read_server_key_package_availability(sqlite_path, expected_device)?;
    if snapshot.available < min_available {
        return Err(CliError::operational(format!(
            "{label}: expected at least {min_available} available KeyPackage row(s) for device `{expected_device}`, got available={}, claimed={}, consumed={}, account_ids={:?}",
            snapshot.available, snapshot.claimed, snapshot.consumed, snapshot.account_ids
        )));
    }
    Ok(snapshot)
}

fn read_server_key_package_availability(
    sqlite_path: &Path,
    expected_device: &str,
) -> Result<KeyPackageAvailabilitySnapshot, CliError> {
    let conn = Connection::open(sqlite_path).map_err(|error| {
        CliError::operational(format!(
            "failed to open harness server sqlite {}: {error}",
            sqlite_path.display()
        ))
    })?;
    let mut stmt = conn
        .prepare("SELECT owner_json, state_json FROM http_key_package_inventory")
        .map_err(|error| {
            CliError::operational(format!(
                "failed to prepare key package inventory query: {error}"
            ))
        })?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| {
            CliError::operational(format!(
                "failed to query key package inventory rows: {error}"
            ))
        })?;
    let mut account_ids = BTreeSet::new();
    let mut available = 0;
    let mut claimed = 0;
    let mut consumed = 0;
    for row in rows {
        let (owner_json, state_json) = row.map_err(|error| {
            CliError::operational(format!("failed to read key package inventory row: {error}"))
        })?;
        let Some(owner) = decode_key_package_owner(&owner_json)? else {
            continue;
        };
        if owner.device_id != expected_device {
            continue;
        }
        account_ids.insert(owner.account_id);
        let state: String = serde_json::from_str(&state_json).map_err(|error| {
            CliError::operational(format!(
                "failed to parse key package state_json `{state_json}`: {error}"
            ))
        })?;
        match state.as_str() {
            "Available" => available += 1,
            "Claimed" => claimed += 1,
            "Consumed" => consumed += 1,
            other => {
                return Err(CliError::operational(format!(
                    "unexpected key package inventory state `{other}`"
                )));
            }
        }
    }
    Ok(KeyPackageAvailabilitySnapshot {
        device_id: expected_device.to_owned(),
        account_ids: account_ids.into_iter().collect(),
        available,
        claimed,
        consumed,
    })
}

#[derive(Debug)]
struct DecodedKeyPackageOwner {
    account_id: String,
    device_id: String,
}

fn decode_key_package_owner(owner_json: &str) -> Result<Option<DecodedKeyPackageOwner>, CliError> {
    let owner_bytes: Vec<u8> = serde_json::from_str(owner_json).map_err(|error| {
        CliError::operational(format!(
            "failed to parse key package owner_json as member bytes: {error}"
        ))
    })?;
    let owner: serde_json::Value = match serde_json::from_slice(&owner_bytes) {
        Ok(owner) => owner,
        Err(_) => return Ok(None),
    };
    let Some(account_id) = owner.get("account_id").and_then(|value| value.as_str()) else {
        return Ok(None);
    };
    let Some(device_id) = owner.get("device_id").and_then(|value| value.as_str()) else {
        return Ok(None);
    };
    Ok(Some(DecodedKeyPackageOwner {
        account_id: account_id.to_owned(),
        device_id: device_id.to_owned(),
    }))
}

fn read_server_delivery_snapshot(sqlite_path: &Path) -> Result<ServerDeliverySnapshot, CliError> {
    let conn = Connection::open(sqlite_path).map_err(|error| {
        CliError::operational(format!(
            "failed to open harness server sqlite {}: {error}",
            sqlite_path.display()
        ))
    })?;
    let publish_messages = query_i64(
        &conn,
        "SELECT COUNT(*) FROM http_delivery_ops WHERE kind = 'publish_message'",
    )?;
    let application_delivery_effects = query_i64(
        &conn,
        "SELECT COUNT(*) FROM http_application_delivery_effects",
    )?;
    let publish_idempotency_rows =
        query_i64(&conn, "SELECT COUNT(*) FROM http_publish_idempotency")?;
    let rooms = query_distinct_strings(
        &conn,
        "SELECT DISTINCT room_id FROM http_application_delivery_effects ORDER BY room_id",
    )?;
    let message_ids = query_distinct_strings(
        &conn,
        "SELECT DISTINCT message_id FROM http_application_delivery_effects ORDER BY message_id",
    )?;
    let sender_devices = query_sender_devices(&conn)?;
    Ok(ServerDeliverySnapshot {
        publish_messages,
        application_delivery_effects,
        publish_idempotency_rows,
        rooms,
        message_ids,
        sender_devices,
    })
}

fn assert_server_snapshot_contains_message(
    snapshot: &ServerDeliverySnapshot,
    message_id: &str,
    label: &str,
) -> Result<(), CliError> {
    if !snapshot.message_ids.iter().any(|value| value == message_id) {
        return Err(CliError::operational(format!(
            "{label}: expected server delivery log to contain visible message id `{message_id}`, got {:?}",
            snapshot.message_ids
        )));
    }
    Ok(())
}

fn assert_server_snapshot_excludes_message(
    snapshot: &ServerDeliverySnapshot,
    message_id: &str,
    label: &str,
) -> Result<(), CliError> {
    if snapshot.message_ids.iter().any(|value| value == message_id) {
        return Err(CliError::operational(format!(
            "{label}: server delivery log already contains offline visible message id `{message_id}` before drain"
        )));
    }
    Ok(())
}

fn assert_local_projection_snapshot(
    store_path: &Path,
    server_url: &str,
    device: &str,
    label: &str,
    expected: LocalProjectionExpectation<'_>,
) -> Result<LocalProjectionSnapshot, CliError> {
    let snapshot = read_local_projection_snapshot(store_path, server_url, device, label)?;
    assert_local_projection_counts(&snapshot, expected, label)?;
    Ok(snapshot)
}

fn assert_local_outbox_snapshot(
    store_path: &Path,
    server_url: &str,
    device: &str,
    label: &str,
    expected: OutboxExpectation<'_>,
) -> Result<LocalOutboxSnapshot, CliError> {
    let snapshot = read_local_outbox_snapshot(store_path, server_url, device, label)?;
    assert_local_outbox_counts(&snapshot, device, expected, label)?;
    Ok(snapshot)
}

fn establish_peer_membership(
    owner_store_path: &Path,
    peer_store_path: &Path,
    server_url: &str,
    owner_device: &str,
    peer_device: &str,
    room_id: &str,
    label: &str,
) -> Result<(), CliError> {
    let peer = open_product_runtime(peer_store_path, server_url, peer_device, label)?;
    let peer_state = peer
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(|error| {
            CliError::operational(format!(
                "{label}: peer failed to publish key packages: {error}"
            ))
        })?;
    let peer_account_id = peer_state.identity.account_id;
    let peer_npub = npub_from_account_id(peer_account_id.clone()).map_err(|error| {
        CliError::operational(format!("{label}: failed to encode peer npub: {error}"))
    })?;
    let peer_profile = AppProfileSummary {
        account_id: peer_account_id,
        npub: peer_npub,
        display_name: peer_device.to_owned(),
        about: None,
        picture: None,
        stale: false,
        is_agent: false,
    };

    let owner = open_product_runtime(owner_store_path, server_url, owner_device, label)?;
    owner
        .dispatch_and_wait(AppAction::AddRoomMembers {
            room_id: room_id.to_owned(),
            profiles: vec![peer_profile],
        })
        .map_err(|error| {
            CliError::operational(format!(
                "{label}: owner failed to add peer by Welcome: {error}"
            ))
        })?;

    let peer_state = peer
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(|error| {
            CliError::operational(format!("{label}: peer failed to claim Welcome: {error}"))
        })?;
    let Some(room) = peer_state.rooms.iter().find(|room| room.room_id == room_id) else {
        return Err(CliError::operational(format!(
            "{label}: peer state missing joined room `{room_id}`"
        )));
    };
    if room.state != AppRoomState::Connected {
        return Err(CliError::operational(format!(
            "{label}: expected peer room `{room_id}` to be connected, got {:?}",
            room.state
        )));
    }
    Ok(())
}

fn read_local_outbox_snapshot(
    store_path: &Path,
    server_url: &str,
    device: &str,
    label: &str,
) -> Result<LocalOutboxSnapshot, CliError> {
    let key_rows = read_local_outbox_key_rows(store_path)?;
    let runtime = open_product_runtime(store_path, server_url, device, label)?;
    let debug_rows = runtime.app_outbox_debug_rows().map_err(|error| {
        CliError::operational(format!(
            "{label}: failed to read app outbox debug rows: {error}"
        ))
    })?;
    local_outbox_snapshot_from_rows(key_rows, debug_rows, label)
}

fn read_local_outbox_key_rows(store_path: &Path) -> Result<Vec<LocalOutboxKeyRow>, CliError> {
    let client_sqlite = store_path.join("client.sqlite3");
    let conn = Connection::open(&client_sqlite).map_err(|error| {
        CliError::operational(format!(
            "failed to open harness client sqlite {}: {error}",
            client_sqlite.display()
        ))
    })?;
    let mut stmt = conn
        .prepare(
            r#"
            SELECT device_id, room_id, message_id
            FROM client_app_outbox
            ORDER BY rowid ASC
            "#,
        )
        .map_err(|error| {
            CliError::operational(format!(
                "failed to prepare harness client outbox query: {error}"
            ))
        })?;
    let rows = stmt
        .query_map([], |row| {
            Ok(LocalOutboxKeyRow {
                device_id: row.get::<_, String>(0)?,
                room_id: row.get::<_, String>(1)?,
                message_id: row.get::<_, String>(2)?,
            })
        })
        .map_err(|error| {
            CliError::operational(format!(
                "failed to query harness client outbox rows: {error}"
            ))
        })?;

    let mut key_rows = Vec::new();
    for row in rows {
        key_rows.push(row.map_err(|error| {
            CliError::operational(format!("failed to read harness client outbox row: {error}"))
        })?);
    }
    Ok(key_rows)
}

fn local_outbox_snapshot_from_rows(
    key_rows: Vec<LocalOutboxKeyRow>,
    debug_rows: Vec<AppOutboxDebugRow>,
    label: &str,
) -> Result<LocalOutboxSnapshot, CliError> {
    if key_rows.len() != debug_rows.len() {
        return Err(CliError::operational(format!(
            "{label}: raw client_app_outbox row count {} did not match Rust debug row count {}",
            key_rows.len(),
            debug_rows.len()
        )));
    }

    let mut device_ids = BTreeSet::new();
    let mut room_ids = BTreeSet::new();
    let mut message_ids = Vec::new();
    let mut local_states = Vec::new();
    let mut server_delivery_states = Vec::new();
    let mut append_request_message_ids = Vec::new();
    let mut idempotency_material_rows = 0usize;

    for (key_row, debug_row) in key_rows.into_iter().zip(debug_rows.into_iter()) {
        if key_row.room_id != debug_row.room_id || key_row.message_id != debug_row.message_id {
            return Err(CliError::operational(format!(
                "{label}: raw outbox key {}/{} did not match Rust debug row {}/{}",
                key_row.room_id, key_row.message_id, debug_row.room_id, debug_row.message_id
            )));
        }
        if key_row.device_id != debug_row.sender_device_id {
            return Err(CliError::operational(format!(
                "{label}: raw outbox device `{}` did not match Rust debug sender device `{}` for message `{}`",
                key_row.device_id, debug_row.sender_device_id, key_row.message_id
            )));
        }
        device_ids.insert(key_row.device_id);
        room_ids.insert(key_row.room_id);
        message_ids.push(key_row.message_id);
        local_states.push(debug_row.local_state);
        server_delivery_states.push(debug_row.server_delivery_state);
        append_request_message_ids.push(debug_row.append_request_message_id);
        if debug_row.idempotency_key_present {
            idempotency_material_rows += 1;
        }
    }
    Ok(LocalOutboxSnapshot {
        rows: message_ids.len(),
        device_ids: device_ids.into_iter().collect(),
        room_ids: room_ids.into_iter().collect(),
        message_ids,
        local_states,
        server_delivery_states,
        append_request_message_ids,
        idempotency_material_rows,
    })
}

fn read_local_projection_snapshot(
    store_path: &Path,
    server_url: &str,
    device: &str,
    label: &str,
) -> Result<LocalProjectionSnapshot, CliError> {
    let runtime = open_product_runtime(store_path, server_url, device, label)?;
    let state = runtime.state().map_err(|error| {
        CliError::operational(format!(
            "{label}: failed to read local product state: {error}"
        ))
    })?;
    Ok(local_projection_snapshot(&state))
}

fn assert_peer_delivery_snapshot(
    peer_store_path: &Path,
    server_url: &str,
    peer_device: &str,
    room_id: &str,
    sender_device: &str,
    message_id: &str,
    label: &str,
) -> Result<PeerReceiptSnapshot, CliError> {
    let runtime = open_product_runtime(peer_store_path, server_url, peer_device, label)?;
    runtime
        .dispatch_and_wait(AppAction::StartRuntime)
        .map_err(|error| {
            CliError::operational(format!("{label}: peer failed to sync after drain: {error}"))
        })?;
    let state = runtime
        .dispatch_and_wait(AppAction::OpenRoom {
            room_id: room_id.to_owned(),
        })
        .map_err(|error| {
            CliError::operational(format!(
                "{label}: peer failed to open drained room: {error}"
            ))
        })?;
    let snapshot = peer_receipt_snapshot(&state, room_id, peer_device, message_id);
    assert_peer_receipt_counts(&snapshot, sender_device, label)?;
    Ok(snapshot)
}

fn open_product_runtime(
    store_path: &Path,
    server_url: &str,
    device: &str,
    label: &str,
) -> Result<std::sync::Arc<FiniteChatRuntime>, CliError> {
    FiniteChatRuntime::open(CoreOpenOptions {
        data_dir: store_path.display().to_string(),
        server_url: server_url.to_owned(),
        device_id: device.to_owned(),
        account_secret_hex: None,
        now_unix_seconds: None,
    })
    .map_err(|error| {
        CliError::operational(format!(
            "{label}: failed to open product store {}: {error}",
            store_path.display()
        ))
    })
}

fn local_projection_snapshot(state: &AppState) -> LocalProjectionSnapshot {
    let mut delivered_message_ids = Vec::new();
    let mut undelivered_message_ids = Vec::new();
    let mut visible_outbound_message_ids = Vec::new();
    let mut delivered_outbound_messages = 0;
    let mut undelivered_outbound_messages = 0;
    let mut failed_outbound_messages = 0;
    let mut sending_outbound_messages = 0;
    let mut local_outbound_messages = 0;
    let mut nonlocal_outbound_delivery_messages = 0;

    for message in &state.messages {
        let Some(outbound) = &message.outbound_delivery else {
            continue;
        };
        if !message.is_mine {
            nonlocal_outbound_delivery_messages += 1;
            continue;
        }
        local_outbound_messages += 1;
        visible_outbound_message_ids.push(message.message_id.clone());
        if outbound.local_send == OutboundLocalSendState::Sending {
            sending_outbound_messages += 1;
        }
        match &outbound.server_delivery {
            OutboundServerDeliveryState::Undelivered => {
                undelivered_outbound_messages += 1;
                undelivered_message_ids.push(message.message_id.clone());
            }
            OutboundServerDeliveryState::Delivered => {
                delivered_outbound_messages += 1;
                delivered_message_ids.push(message.message_id.clone());
            }
            OutboundServerDeliveryState::Failed { .. } => {
                failed_outbound_messages += 1;
            }
        }
    }

    LocalProjectionSnapshot {
        rooms: state.rooms.len(),
        connected_rooms: state
            .rooms
            .iter()
            .filter(|room| room.state == AppRoomState::Connected)
            .count(),
        unavailable_on_device_rooms: state
            .rooms
            .iter()
            .filter(|room| room.state == AppRoomState::UnavailableOnDevice)
            .count(),
        selected_room_id: state.selected_room_id.clone(),
        messages: state.messages.len(),
        local_outbound_messages,
        delivered_outbound_messages,
        undelivered_outbound_messages,
        failed_outbound_messages,
        sending_outbound_messages,
        nonlocal_outbound_delivery_messages,
        delivered_message_ids,
        undelivered_message_ids,
        visible_outbound_message_ids,
    }
}

fn peer_receipt_snapshot(
    state: &AppState,
    room_id: &str,
    peer_device: &str,
    message_id: &str,
) -> PeerReceiptSnapshot {
    let matching = state
        .messages
        .iter()
        .filter(|message| message.room_id == room_id && message.message_id == message_id)
        .collect::<Vec<_>>();
    let mut sender_devices = matching
        .iter()
        .map(|message| message.sender_device_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    sender_devices.sort();
    PeerReceiptSnapshot {
        room_id: room_id.to_owned(),
        peer_device: peer_device.to_owned(),
        message_id: message_id.to_owned(),
        total_peer_messages: state
            .messages
            .iter()
            .filter(|message| message.room_id == room_id)
            .count(),
        matching_messages: matching.len(),
        inbound_matching_messages: matching
            .iter()
            .filter(|message| !message.is_mine && message.outbound_delivery.is_none())
            .count(),
        local_matching_messages: matching.iter().filter(|message| message.is_mine).count(),
        matching_with_outbound_delivery: matching
            .iter()
            .filter(|message| message.outbound_delivery.is_some())
            .count(),
        sender_devices,
    }
}

fn assert_local_projection_counts(
    snapshot: &LocalProjectionSnapshot,
    expected: LocalProjectionExpectation<'_>,
    label: &str,
) -> Result<(), CliError> {
    if snapshot.rooms != 1 {
        return Err(CliError::operational(format!(
            "{label}: expected exactly one local room, got {}",
            snapshot.rooms
        )));
    }
    if snapshot.connected_rooms != 1 || snapshot.unavailable_on_device_rooms != 0 {
        return Err(CliError::operational(format!(
            "{label}: expected one connected room and no unavailable rooms, got connected={} unavailable={}",
            snapshot.connected_rooms, snapshot.unavailable_on_device_rooms
        )));
    }
    if snapshot.selected_room_id.is_none() {
        return Err(CliError::operational(format!(
            "{label}: expected selected local room after force-close projection"
        )));
    }
    if snapshot.messages != expected.expected_messages {
        return Err(CliError::operational(format!(
            "{label}: expected {} visible local message(s), got {}",
            expected.expected_messages, snapshot.messages
        )));
    }
    if snapshot.local_outbound_messages != expected.expected_messages {
        return Err(CliError::operational(format!(
            "{label}: expected every visible message to be a local outbound message, got {} outbound for {} visible",
            snapshot.local_outbound_messages, snapshot.messages
        )));
    }
    if snapshot.nonlocal_outbound_delivery_messages != 0 {
        return Err(CliError::operational(format!(
            "{label}: outbound_delivery projected on {} non-local message(s)",
            snapshot.nonlocal_outbound_delivery_messages
        )));
    }
    if snapshot.sending_outbound_messages != 0 {
        return Err(CliError::operational(format!(
            "{label}: expected no transient Sending rows after force close, got {}",
            snapshot.sending_outbound_messages
        )));
    }
    if snapshot.failed_outbound_messages != 0 {
        return Err(CliError::operational(format!(
            "{label}: expected no failed automatic-drain rows, got {}",
            snapshot.failed_outbound_messages
        )));
    }
    if snapshot.delivered_outbound_messages != expected.expected_delivered {
        return Err(CliError::operational(format!(
            "{label}: expected {} delivered local outbound message(s), got {}",
            expected.expected_delivered, snapshot.delivered_outbound_messages
        )));
    }
    if snapshot.undelivered_outbound_messages != expected.expected_undelivered {
        return Err(CliError::operational(format!(
            "{label}: expected {} undelivered local outbound message(s), got {}",
            expected.expected_undelivered, snapshot.undelivered_outbound_messages
        )));
    }
    if snapshot.visible_outbound_message_ids.len() != expected.expected_messages {
        return Err(CliError::operational(format!(
            "{label}: expected {} visible outbound id(s), got {:?}",
            expected.expected_messages, snapshot.visible_outbound_message_ids
        )));
    }
    let unique_visible_ids = snapshot
        .visible_outbound_message_ids
        .iter()
        .collect::<BTreeSet<_>>();
    if unique_visible_ids.len() != snapshot.visible_outbound_message_ids.len() {
        return Err(CliError::operational(format!(
            "{label}: duplicate visible outbound message id(s): {:?}",
            snapshot.visible_outbound_message_ids
        )));
    }
    for message_id in expected.required_delivered_message_ids {
        if !snapshot
            .delivered_message_ids
            .iter()
            .any(|value| value == message_id)
        {
            return Err(CliError::operational(format!(
                "{label}: expected message id `{message_id}` to be delivered without changing identity; delivered ids were {:?}",
                snapshot.delivered_message_ids
            )));
        }
    }
    Ok(())
}

fn assert_local_projection_same_visible_outbound(
    actual: &LocalProjectionSnapshot,
    expected: &LocalProjectionSnapshot,
    label: &str,
) -> Result<(), CliError> {
    if actual.visible_outbound_message_ids != expected.visible_outbound_message_ids {
        return Err(CliError::operational(format!(
            "{label}: expected offline attachment fail-fast to leave visible outbound ids unchanged at {:?}, got {:?}",
            expected.visible_outbound_message_ids, actual.visible_outbound_message_ids
        )));
    }
    if actual.delivered_message_ids != expected.delivered_message_ids {
        return Err(CliError::operational(format!(
            "{label}: expected delivered outbound ids unchanged at {:?}, got {:?}",
            expected.delivered_message_ids, actual.delivered_message_ids
        )));
    }
    if actual.undelivered_message_ids != expected.undelivered_message_ids {
        return Err(CliError::operational(format!(
            "{label}: expected undelivered outbound ids unchanged at {:?}, got {:?}",
            expected.undelivered_message_ids, actual.undelivered_message_ids
        )));
    }
    Ok(())
}

fn assert_local_outbox_counts(
    snapshot: &LocalOutboxSnapshot,
    device: &str,
    expected: OutboxExpectation<'_>,
    label: &str,
) -> Result<(), CliError> {
    if expected.expected_rows == 0
        && (expected.required_room_id.is_some()
            || expected.required_message_id.is_some()
            || expected.expected_local_state.is_some()
            || expected.expected_server_delivery_state.is_some())
    {
        return Err(CliError::operational(format!(
            "{label}: zero-row outbox expectation must not specify row identity or delivery state"
        )));
    }
    if expected.expected_rows > 0
        && (expected.required_room_id.is_none()
            || expected.expected_local_state.is_none()
            || expected.expected_server_delivery_state.is_none())
    {
        return Err(CliError::operational(format!(
            "{label}: non-empty outbox expectation must specify room id plus local and server delivery state"
        )));
    }
    if snapshot.rows != expected.expected_rows {
        return Err(CliError::operational(format!(
            "{label}: expected {} durable outbox row(s), got {} with ids {:?}",
            expected.expected_rows, snapshot.rows, snapshot.message_ids
        )));
    }
    if snapshot.rows > 0 && snapshot.device_ids != vec![device.to_owned()] {
        return Err(CliError::operational(format!(
            "{label}: expected outbox rows only for device `{device}`, got {:?}",
            snapshot.device_ids
        )));
    }
    if let Some(room_id) = expected.required_room_id
        && snapshot.room_ids != vec![room_id.to_owned()]
    {
        return Err(CliError::operational(format!(
            "{label}: expected durable outbox row in room `{room_id}`, got {:?}",
            snapshot.room_ids
        )));
    }
    let unique_message_ids = snapshot.message_ids.iter().collect::<BTreeSet<_>>();
    if unique_message_ids.len() != snapshot.message_ids.len() {
        return Err(CliError::operational(format!(
            "{label}: duplicate durable outbox message id(s): {:?}",
            snapshot.message_ids
        )));
    }
    if let Some(message_id) = expected.required_message_id
        && !snapshot.message_ids.iter().any(|value| value == message_id)
    {
        return Err(CliError::operational(format!(
            "{label}: expected durable outbox row for visible message id `{message_id}`, got {:?}",
            snapshot.message_ids
        )));
    }
    if snapshot.local_states.len() != snapshot.rows
        || snapshot.server_delivery_states.len() != snapshot.rows
        || snapshot.append_request_message_ids.len() != snapshot.rows
    {
        return Err(CliError::operational(format!(
            "{label}: decrypted outbox metadata shape did not match raw row count {}; local_states={:?} server_delivery_states={:?} append_request_message_ids={:?}",
            snapshot.rows,
            snapshot.local_states,
            snapshot.server_delivery_states,
            snapshot.append_request_message_ids
        )));
    }
    if snapshot.append_request_message_ids != snapshot.message_ids {
        return Err(CliError::operational(format!(
            "{label}: append request message ids {:?} did not match visible outbox ids {:?}",
            snapshot.append_request_message_ids, snapshot.message_ids
        )));
    }
    if snapshot.idempotency_material_rows != snapshot.rows {
        return Err(CliError::operational(format!(
            "{label}: expected idempotency material for all {} outbox row(s), got {}",
            snapshot.rows, snapshot.idempotency_material_rows
        )));
    }
    if let Some(expected_state) = expected.expected_local_state
        && snapshot
            .local_states
            .iter()
            .any(|actual| actual != expected_state)
    {
        return Err(CliError::operational(format!(
            "{label}: expected durable outbox local state `{expected_state}`, got {:?}",
            snapshot.local_states
        )));
    }
    if let Some(expected_state) = expected.expected_server_delivery_state
        && snapshot
            .server_delivery_states
            .iter()
            .any(|actual| actual != expected_state)
    {
        return Err(CliError::operational(format!(
            "{label}: expected durable outbox server delivery state `{expected_state}`, got {:?}",
            snapshot.server_delivery_states
        )));
    }
    Ok(())
}

fn assert_peer_receipt_counts(
    snapshot: &PeerReceiptSnapshot,
    sender_device: &str,
    label: &str,
) -> Result<(), CliError> {
    if snapshot.matching_messages != 1 {
        return Err(CliError::operational(format!(
            "{label}: expected peer `{}` to receive message `{}` exactly once, got {} matching rows",
            snapshot.peer_device, snapshot.message_id, snapshot.matching_messages
        )));
    }
    if snapshot.inbound_matching_messages != 1
        || snapshot.local_matching_messages != 0
        || snapshot.matching_with_outbound_delivery != 0
    {
        return Err(CliError::operational(format!(
            "{label}: expected peer message `{}` to be one inbound row with no outbound_delivery, got inbound={} local={} outbound_delivery={}",
            snapshot.message_id,
            snapshot.inbound_matching_messages,
            snapshot.local_matching_messages,
            snapshot.matching_with_outbound_delivery
        )));
    }
    if snapshot.sender_devices != vec![sender_device.to_owned()] {
        return Err(CliError::operational(format!(
            "{label}: expected peer message `{}` sender device `{sender_device}`, got {:?}",
            snapshot.message_id, snapshot.sender_devices
        )));
    }
    Ok(())
}

fn query_i64(conn: &Connection, sql: &str) -> Result<i64, CliError> {
    conn.query_row(sql, [], |row| row.get(0))
        .map_err(|error| CliError::operational(format!("failed to query harness sqlite: {error}")))
}

fn query_distinct_strings(conn: &Connection, sql: &str) -> Result<Vec<String>, CliError> {
    let mut stmt = conn.prepare(sql).map_err(|error| {
        CliError::operational(format!("failed to prepare harness sqlite query: {error}"))
    })?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| {
            CliError::operational(format!("failed to query harness sqlite rows: {error}"))
        })?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row.map_err(|error| {
            CliError::operational(format!("failed to read harness sqlite row: {error}"))
        })?);
    }
    Ok(values)
}

fn query_sender_devices(conn: &Connection) -> Result<Vec<String>, CliError> {
    let sender_json_values = query_distinct_strings(
        conn,
        "SELECT DISTINCT sender_json FROM http_application_delivery_effects ORDER BY sender_json",
    )?;
    let mut devices = BTreeSet::new();
    for sender_json in sender_json_values {
        let value = serde_json::from_str::<serde_json::Value>(&sender_json).map_err(|error| {
            CliError::operational(format!(
                "failed to parse sender_json from harness sqlite: {error}"
            ))
        })?;
        let Some(device) = value.get("device_id").and_then(|value| value.as_str()) else {
            return Err(CliError::operational(
                "sender_json in harness sqlite does not contain device_id",
            ));
        };
        devices.insert(device.to_owned());
    }
    Ok(devices.into_iter().collect())
}

fn write_harness_config(path: &Path, server_url: &str, device: &str) -> Result<(), CliError> {
    let config = serde_json::json!({
        "server_url": server_url,
        "device_id": device,
    });
    let bytes = serde_json::to_vec_pretty(&config)
        .map_err(|error| CliError::operational(format!("failed to encode config: {error}")))?;
    fs::write(path, bytes).map_err(|error| {
        CliError::operational(format!(
            "failed to write harness config {}: {error}",
            path.display()
        ))
    })
}

fn validate_existing_harness_config(
    path: &Path,
    server_url: &str,
    device: &str,
) -> Result<(), CliError> {
    if !path.exists() {
        return Ok(());
    }
    let bytes = fs::read(path).map_err(|error| {
        CliError::operational(format!(
            "failed to read existing harness config {}: {error}",
            path.display()
        ))
    })?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::operational(format!(
            "existing harness config {} is not valid JSON: {error}",
            path.display()
        ))
    })?;
    let existing_server_url = value.get("server_url").and_then(|value| value.as_str());
    let existing_device = value.get("device_id").and_then(|value| value.as_str());
    if existing_server_url != Some(server_url) || existing_device != Some(device) {
        return Err(CliError::user(format!(
            "--no-reset would change harness identity for {}; reset the store or rerun with --server-url {} --device {}",
            path.display(),
            existing_server_url.unwrap_or("<missing>"),
            existing_device.unwrap_or("<missing>")
        )));
    }
    Ok(())
}

fn product_harness_platform_label(platform: ProductHarnessPlatform) -> &'static str {
    match platform {
        ProductHarnessPlatform::IosSimulator => "ios-simulator",
        ProductHarnessPlatform::IosDevice => "ios-device",
    }
}

fn checked_product_harness_udid(
    platform: ProductHarnessPlatform,
    udid: Option<String>,
) -> Result<Option<String>, CliError> {
    let udid = udid.map(|value| value.trim().to_owned());
    match platform {
        ProductHarnessPlatform::IosSimulator => Ok(udid.filter(|value| !value.is_empty())),
        ProductHarnessPlatform::IosDevice => udid
            .filter(|value| !value.is_empty())
            .map(Some)
            .ok_or_else(|| {
                CliError::user(
                    "product harness ios-device requires --udid for an attached physical iPhone",
                )
            }),
    }
}

fn checked_product_harness_ios_development_team(
    platform: ProductHarnessPlatform,
    development_team: Option<String>,
    env_value: Option<String>,
) -> Result<Option<String>, CliError> {
    match platform {
        ProductHarnessPlatform::IosSimulator => Ok(None),
        ProductHarnessPlatform::IosDevice => {
            run::require_ios_development_team_with(development_team, env_value).map(Some)
        }
    }
}

fn checked_server_url(value: &str) -> Result<String, CliError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CliError::user("server URL must not be empty"));
    }
    if !trimmed.starts_with("http://") {
        return Err(CliError::user(
            "product harness server URL must be an explicit http:// URL",
        ));
    }
    let normalized = trimmed.trim_end_matches('/');
    let (host, port) = http_url_host_port(normalized).ok_or_else(|| {
        CliError::user("product harness server URL must be an origin-only http://host:port URL")
    })?;
    if host.contains('@') {
        return Err(CliError::user(
            "product harness server URL must not include userinfo",
        ));
    }
    let port = port.parse::<u16>().map_err(|error| {
        CliError::user(format!(
            "product harness server URL port `{port}` is invalid: {error}"
        ))
    })?;
    if port == 0 {
        return Err(CliError::user(
            "product harness server URL port must be greater than zero",
        ));
    }
    Ok(normalized.to_owned())
}

fn checked_ios_device_server_url(server_url: &str) -> Result<(), CliError> {
    let host = http_url_host(server_url).ok_or_else(|| {
        CliError::user("product harness server URL must include a host and explicit port")
    })?;
    let normalized = host.trim_matches(['[', ']']).to_ascii_lowercase();
    let is_loopback = normalized == "localhost"
        || normalized == "::1"
        || normalized == "::"
        || normalized == "0.0.0.0"
        || normalized.starts_with("127.");
    if is_loopback {
        return Err(CliError::user(format!(
            "product harness ios-device server URL host `{host}` is not reachable from a physical phone; use the Mac LAN address in --server-url and bind with --server-addr 0.0.0.0:<port> when needed"
        )));
    }
    Ok(())
}

fn checked_ios_device_server_addr(server_addr: &str) -> Result<(), CliError> {
    let addr: SocketAddr = server_addr.parse().map_err(|error| {
        CliError::user(format!(
            "server address `{server_addr}` is invalid: {error}"
        ))
    })?;
    if addr.ip().is_loopback() {
        return Err(CliError::user(format!(
            "product harness ios-device server bind address `{server_addr}` is loopback and cannot accept physical-phone connections; use 0.0.0.0:<port> or the Mac LAN address"
        )));
    }
    Ok(())
}

fn checked_server_addr(value: &str) -> Result<String, CliError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CliError::user("server address must not be empty"));
    }
    let _: SocketAddr = trimmed.parse().map_err(|error| {
        CliError::user(format!("server address `{trimmed}` is invalid: {error}"))
    })?;
    Ok(trimmed.to_owned())
}

fn http_url_host(server_url: &str) -> Option<&str> {
    http_url_host_port(server_url).map(|(host, _port)| host)
}

fn http_url_host_port(server_url: &str) -> Option<(&str, &str)> {
    let authority = server_url.strip_prefix("http://")?;
    if authority.is_empty()
        || authority.contains('/')
        || authority.contains('?')
        || authority.contains('#')
    {
        return None;
    }
    if authority.starts_with('[') {
        let close = authority.find(']')?;
        let host = &authority[1..close];
        let rest = &authority[close + 1..];
        if host.is_empty() || !rest.starts_with(':') || rest.len() <= 1 {
            return None;
        }
        return Some((host, &rest[1..]));
    }
    let (host, port) = authority.rsplit_once(':')?;
    if host.is_empty() || port.is_empty() || host.contains(':') {
        return None;
    }
    Some((host, port))
}

fn server_addr_from_url(server_url: &str) -> Result<String, CliError> {
    let (host, port) = http_url_host_port(server_url).ok_or_else(|| {
        CliError::user("product harness server URL must be an origin-only http://host:port URL")
    })?;
    let authority = socket_addr_authority(host, port);
    checked_server_addr(&authority)
}

fn ios_device_default_server_addr_from_url(server_url: &str) -> Result<String, CliError> {
    let (_host, port) = http_url_host_port(server_url).ok_or_else(|| {
        CliError::user("product harness server URL must be an origin-only http://host:port URL")
    })?;
    checked_server_addr(&format!("0.0.0.0:{port}"))
}

fn socket_addr_authority(host: &str, port: &str) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn harness_phases(scenario: &str) -> Vec<&'static str> {
    match scenario {
        "text-offline" => vec![
            "online-create-room-and-send",
            "terminate-server-and-send-offline",
            "attempt-offline-attachment-fail-fast",
            "restart-same-server-url-and-drain",
        ],
        "profile-dm" => vec!["peer-publishes-key-packages", "profile-dm-start-and-send"],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    struct ProductHarnessArgsBuilder {
        args: ProductHarnessArgs,
    }

    impl ProductHarnessArgsBuilder {
        fn scenario(mut self, value: &str) -> Self {
            self.args.scenario = value.to_owned();
            self
        }

        fn server_url(mut self, value: &str) -> Self {
            self.args.server_url = value.to_owned();
            self
        }

        fn server_addr(mut self, value: Option<&str>) -> Self {
            self.args.server_addr = value.map(ToOwned::to_owned);
            self
        }

        fn udid(mut self, value: Option<&str>) -> Self {
            self.args.udid = value.map(ToOwned::to_owned);
            self
        }

        fn ios_development_team(mut self, value: Option<&str>) -> Self {
            self.args.ios_development_team = value.map(ToOwned::to_owned);
            self
        }

        fn dry_run(mut self, value: bool) -> Self {
            self.args.dry_run = value;
            self
        }

        fn build(self) -> ProductHarnessArgs {
            self.args
        }
    }

    fn product_harness_args(platform: ProductHarnessPlatform) -> ProductHarnessArgsBuilder {
        ProductHarnessArgsBuilder {
            args: ProductHarnessArgs {
                platform,
                scenario: "text-offline".to_owned(),
                device: "phone-a".to_owned(),
                server_url: "http://127.0.0.1:8787".to_owned(),
                server_addr: None,
                udid: None,
                ios_development_team: None,
                dry_run: false,
                no_reset: false,
                settle_seconds: 0,
            },
        }
    }

    #[test]
    fn server_addr_is_derived_from_same_configured_server_url() {
        assert_eq!(
            server_addr_from_url("http://127.0.0.1:8787").expect("addr"),
            "127.0.0.1:8787"
        );
        assert_eq!(
            server_addr_from_url("http://[::1]:8787").expect("addr"),
            "[::1]:8787"
        );
    }

    #[test]
    fn server_url_requires_http_with_explicit_port() {
        assert!(checked_server_url("https://127.0.0.1:8787").is_err());
        assert!(server_addr_from_url("http://127.0.0.1").is_err());
    }

    #[test]
    fn server_url_is_origin_only_and_normalized() {
        assert_eq!(
            checked_server_url(" http://127.0.0.1:8787/ ").expect("server url"),
            "http://127.0.0.1:8787"
        );
        assert!(checked_server_url("http://127.0.0.1:8787/path").is_err());
        assert!(checked_server_url("http://127.0.0.1:8787?x=1").is_err());
        assert!(checked_server_url("http://127.0.0.1:8787#frag").is_err());
        assert!(checked_server_url("http://user@127.0.0.1:8787").is_err());
        assert!(checked_server_url("http://127.0.0.1:0").is_err());
    }

    #[test]
    fn ios_device_default_bind_addr_uses_all_interfaces_and_server_url_port() {
        assert_eq!(
            ios_device_default_server_addr_from_url("http://192.168.1.40:18793")
                .expect("device bind addr"),
            "0.0.0.0:18793"
        );
        assert_eq!(
            ios_device_default_server_addr_from_url("http://macbook-pro.local:18793")
                .expect("device bind addr"),
            "0.0.0.0:18793"
        );
    }

    #[test]
    fn server_probe_addr_maps_unspecified_binds_to_loopback() {
        assert_eq!(
            server_probe_addr("0.0.0.0:18793").expect("probe addr"),
            "127.0.0.1:18793".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            server_probe_addr("[::]:18793").expect("probe addr"),
            "[::1]:18793".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            server_probe_addr("127.0.0.1:18793").expect("probe addr"),
            "127.0.0.1:18793".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn server_addr_is_reachable_detects_existing_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("listener addr");

        assert!(server_addr_is_reachable(addr));
    }

    #[test]
    fn ios_device_server_url_rejects_phone_unreachable_loopback_hosts() {
        assert!(checked_ios_device_server_url("http://127.0.0.1:8787").is_err());
        assert!(checked_ios_device_server_url("http://localhost:8787").is_err());
        assert!(checked_ios_device_server_url("http://0.0.0.0:8787").is_err());
        assert!(checked_ios_device_server_url("http://[::1]:8787").is_err());
    }

    #[test]
    fn ios_device_server_url_accepts_lan_ip_or_hostname() {
        checked_ios_device_server_url("http://192.168.1.40:8787").expect("lan ip");
        checked_ios_device_server_url("http://macbook-pro.local:8787").expect("hostname");
    }

    #[test]
    fn ios_device_server_addr_rejects_loopback_binds() {
        for value in ["127.0.0.1:8787", "[::1]:8787"] {
            let error =
                checked_ios_device_server_addr(value).expect_err("loopback bind should fail");
            assert!(
                error.to_string().contains("server bind address"),
                "unexpected error: {error}"
            );
        }

        checked_ios_device_server_addr("0.0.0.0:8787").expect("all interfaces bind");
        checked_ios_device_server_addr("192.168.1.40:8787").expect("lan bind");
    }

    #[test]
    fn ios_device_dry_run_preflights_without_creating_state() {
        let temp = tempfile::tempdir().expect("tempdir");

        ios_product_harness_with_ios_development_team_env(
            temp.path(),
            false,
            false,
            product_harness_args(ProductHarnessPlatform::IosDevice)
                .server_url("http://192.168.1.40:18793")
                .udid(Some("PHONE-UDID"))
                .ios_development_team(Some("TEAM123"))
                .dry_run(true)
                .build(),
            None,
        )
        .expect("device dry run");

        assert!(
            !temp.path().join(".state").exists(),
            "dry-run must not create product harness state"
        );
    }

    #[test]
    fn ios_device_dry_run_accepts_env_signing_team_without_creating_state() {
        let temp = tempfile::tempdir().expect("tempdir");

        ios_product_harness_with_ios_development_team_env(
            temp.path(),
            false,
            false,
            product_harness_args(ProductHarnessPlatform::IosDevice)
                .server_url("http://192.168.1.40:18793")
                .udid(Some("PHONE-UDID"))
                .dry_run(true)
                .build(),
            Some("TEAM123".to_owned()),
        )
        .expect("device dry run with env team");

        assert!(
            !temp.path().join(".state").exists(),
            "dry-run must not create product harness state"
        );
    }

    #[test]
    fn profile_dm_simulator_dry_run_preflights_without_creating_state() {
        let temp = tempfile::tempdir().expect("tempdir");

        ios_product_harness_with_ios_development_team_env(
            temp.path(),
            false,
            false,
            product_harness_args(ProductHarnessPlatform::IosSimulator)
                .scenario("profile-dm")
                .dry_run(true)
                .build(),
            None,
        )
        .expect("profile dm simulator dry run");

        assert_eq!(
            harness_phases("profile-dm"),
            vec!["peer-publishes-key-packages", "profile-dm-start-and-send"]
        );
        assert!(
            !temp.path().join(".state").exists(),
            "dry-run must not create product harness state"
        );
    }

    #[test]
    fn ios_device_dry_run_rejects_loopback_server_url_without_creating_state() {
        let temp = tempfile::tempdir().expect("tempdir");

        let error = ios_product_harness_with_ios_development_team_env(
            temp.path(),
            false,
            false,
            product_harness_args(ProductHarnessPlatform::IosDevice)
                .server_url("http://127.0.0.1:18793")
                .udid(Some("PHONE-UDID"))
                .ios_development_team(Some("TEAM123"))
                .dry_run(true)
                .build(),
            None,
        )
        .expect_err("loopback server URL should be rejected for phone dry-run");

        assert!(
            error
                .to_string()
                .contains("not reachable from a physical phone"),
            "unexpected error: {error}"
        );
        assert!(
            !temp.path().join(".state").exists(),
            "rejected dry-run must not create product harness state"
        );
    }

    #[test]
    fn ios_device_dry_run_rejects_loopback_bind_without_creating_state() {
        let temp = tempfile::tempdir().expect("tempdir");

        let error = ios_product_harness_with_ios_development_team_env(
            temp.path(),
            false,
            false,
            product_harness_args(ProductHarnessPlatform::IosDevice)
                .server_url("http://192.168.1.40:18793")
                .server_addr(Some("127.0.0.1:18793"))
                .udid(Some("PHONE-UDID"))
                .ios_development_team(Some("TEAM123"))
                .dry_run(true)
                .build(),
            None,
        )
        .expect_err("loopback bind should be rejected for phone dry-run");

        assert!(
            error.to_string().contains("server bind address"),
            "unexpected error: {error}"
        );
        assert!(
            !temp.path().join(".state").exists(),
            "rejected dry-run must not create product harness state"
        );
    }

    #[test]
    fn ios_device_dry_run_requires_udid_and_signing_team_without_creating_state() {
        let temp = tempfile::tempdir().expect("tempdir");

        let missing_udid = ios_product_harness_with_ios_development_team_env(
            temp.path(),
            false,
            false,
            product_harness_args(ProductHarnessPlatform::IosDevice)
                .server_url("http://192.168.1.40:18793")
                .ios_development_team(Some("TEAM123"))
                .dry_run(true)
                .build(),
            None,
        )
        .expect_err("phone dry-run should require udid");
        assert!(
            missing_udid.to_string().contains("requires --udid"),
            "unexpected error: {missing_udid}"
        );

        let missing_team = ios_product_harness_with_ios_development_team_env(
            temp.path(),
            false,
            false,
            product_harness_args(ProductHarnessPlatform::IosDevice)
                .server_url("http://192.168.1.40:18793")
                .udid(Some("PHONE-UDID"))
                .dry_run(true)
                .build(),
            None,
        )
        .expect_err("phone dry-run should require signing team");
        assert!(
            missing_team
                .to_string()
                .contains("requires --ios-development-team"),
            "unexpected error: {missing_team}"
        );
        assert!(
            !temp.path().join(".state").exists(),
            "rejected dry-run must not create product harness state"
        );
    }

    #[test]
    fn product_harness_platform_labels_are_stable() {
        assert_eq!(
            product_harness_platform_label(ProductHarnessPlatform::IosSimulator),
            "ios-simulator"
        );
        assert_eq!(
            product_harness_platform_label(ProductHarnessPlatform::IosDevice),
            "ios-device"
        );
    }

    #[test]
    fn ios_device_product_harness_requires_udid_before_dry_run_or_build() {
        for value in [None, Some("   ".to_owned())] {
            let error = checked_product_harness_udid(ProductHarnessPlatform::IosDevice, value)
                .expect_err("physical phone harness requires explicit udid");
            assert!(
                error.to_string().contains("requires --udid"),
                "unexpected error: {error}"
            );
        }

        assert_eq!(
            checked_product_harness_udid(
                ProductHarnessPlatform::IosDevice,
                Some("  PHONE-UDID  ".to_owned()),
            )
            .expect("udid"),
            Some("PHONE-UDID".to_owned())
        );
    }

    #[test]
    fn simulator_product_harness_keeps_udid_optional() {
        assert_eq!(
            checked_product_harness_udid(ProductHarnessPlatform::IosSimulator, None)
                .expect("missing simulator udid is ok"),
            None
        );
        assert_eq!(
            checked_product_harness_udid(
                ProductHarnessPlatform::IosSimulator,
                Some("  SIM-UDID  ".to_owned()),
            )
            .expect("simulator udid"),
            Some("SIM-UDID".to_owned())
        );
    }

    #[test]
    fn no_reset_harness_config_accepts_missing_or_matching_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("finitechat_config.json");

        validate_existing_harness_config(&config, "http://127.0.0.1:8787", "sim-a")
            .expect("missing config is ok");
        write_harness_config(&config, "http://127.0.0.1:8787", "sim-a").expect("write config");
        validate_existing_harness_config(&config, "http://127.0.0.1:8787", "sim-a")
            .expect("matching config is ok");
    }

    #[test]
    fn no_reset_harness_config_rejects_server_or_device_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("finitechat_config.json");
        write_harness_config(&config, "http://127.0.0.1:8787", "sim-a").expect("write config");

        let server_error =
            validate_existing_harness_config(&config, "http://127.0.0.1:8788", "sim-a")
                .expect_err("server URL change should require reset");
        assert!(
            server_error
                .to_string()
                .contains("--no-reset would change harness identity"),
            "unexpected error: {server_error}"
        );

        let device_error =
            validate_existing_harness_config(&config, "http://127.0.0.1:8787", "sim-b")
                .expect_err("device change should require reset");
        assert!(
            device_error
                .to_string()
                .contains("--no-reset would change harness identity"),
            "unexpected error: {device_error}"
        );
    }

    #[test]
    fn no_reset_harness_config_rejects_invalid_existing_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("finitechat_config.json");
        fs::write(&config, b"not json").expect("write invalid json");

        let error = validate_existing_harness_config(&config, "http://127.0.0.1:8787", "sim-a")
            .expect_err("invalid config should fail closed");

        assert!(
            error.to_string().contains("is not valid JSON"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn delivery_snapshot_accepts_exact_same_device_drain_shape() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("server.sqlite3");
        seed_server_delivery_db(&db_path, 2, "simulator-a");

        let snapshot = assert_server_delivery_snapshot(&db_path, 2, "simulator-a", "after restart")
            .expect("asserted snapshot");

        assert_eq!(snapshot.publish_messages, 2);
        assert_eq!(snapshot.application_delivery_effects, 2);
        assert_eq!(snapshot.publish_idempotency_rows, 2);
        assert_eq!(snapshot.rooms, vec!["room-main"]);
        assert_eq!(snapshot.message_ids, vec!["message-0", "message-1"]);
        assert_eq!(snapshot.sender_devices, vec!["simulator-a"]);
        assert_server_snapshot_contains_message(&snapshot, "message-1", "after restart")
            .expect("server log contains drained visible id");
        assert_server_snapshot_excludes_message(&snapshot, "message-2", "after restart")
            .expect("server log excludes undrained visible id");
    }

    #[test]
    fn delivery_snapshot_rejects_duplicate_drain() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("server.sqlite3");
        seed_server_delivery_db(&db_path, 3, "simulator-a");

        let error = assert_server_delivery_snapshot(&db_path, 2, "simulator-a", "after restart")
            .expect_err("duplicate drain should fail");

        assert!(
            error
                .to_string()
                .contains("expected 2 application delivery effect row(s), got 3")
        );
    }

    #[test]
    fn delivery_snapshot_rejects_wrong_sender_device() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("server.sqlite3");
        seed_server_delivery_db(&db_path, 2, "other-device");

        let error = assert_server_delivery_snapshot(&db_path, 2, "simulator-a", "after restart")
            .expect_err("wrong sender should fail");

        assert!(
            error
                .to_string()
                .contains("expected sender device `simulator-a`")
        );
    }

    #[test]
    fn delivery_snapshot_rejects_missing_visible_message_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("server.sqlite3");
        seed_server_delivery_db(&db_path, 1, "simulator-a");
        let snapshot = read_server_delivery_snapshot(&db_path).expect("snapshot");

        let missing =
            assert_server_snapshot_contains_message(&snapshot, "offline-message", "after restart")
                .expect_err("missing visible server id should fail");
        assert!(
            missing
                .to_string()
                .contains("expected server delivery log to contain visible message id"),
            "unexpected error: {missing}"
        );

        let premature =
            assert_server_snapshot_excludes_message(&snapshot, "message-0", "after offline")
                .expect_err("premature visible server id should fail");
        assert!(
            premature
                .to_string()
                .contains("already contains offline visible message id"),
            "unexpected error: {premature}"
        );
    }

    #[test]
    fn local_projection_counts_accept_force_close_and_drain_identity() {
        let offline = local_projection_snapshot_fixture(
            2,
            1,
            1,
            &["online-message"],
            &["offline-message"],
            &["online-message", "offline-message"],
        );
        assert_local_projection_counts(
            &offline,
            LocalProjectionExpectation {
                expected_messages: 2,
                expected_delivered: 1,
                expected_undelivered: 1,
                required_delivered_message_ids: &["online-message"],
            },
            "after offline",
        )
        .expect("offline projection shape");

        let drained = local_projection_snapshot_fixture(
            2,
            2,
            0,
            &["online-message", "offline-message"],
            &[],
            &["online-message", "offline-message"],
        );
        assert_local_projection_counts(
            &drained,
            LocalProjectionExpectation {
                expected_messages: 2,
                expected_delivered: 2,
                expected_undelivered: 0,
                required_delivered_message_ids: &["online-message", "offline-message"],
            },
            "after restart",
        )
        .expect("same visible offline message id promoted to delivered");
    }

    #[test]
    fn local_projection_counts_reject_missing_online_identity_after_force_close() {
        let offline = local_projection_snapshot_fixture(
            2,
            1,
            1,
            &["replacement-online-message"],
            &["offline-message"],
            &["replacement-online-message", "offline-message"],
        );

        let error = assert_local_projection_counts(
            &offline,
            LocalProjectionExpectation {
                expected_messages: 2,
                expected_delivered: 1,
                expected_undelivered: 1,
                required_delivered_message_ids: &["online-message"],
            },
            "after offline",
        )
        .expect_err("changed online visible identity should fail");

        assert!(
            error
                .to_string()
                .contains("expected message id `online-message`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn local_projection_counts_reject_missing_drained_identity() {
        let drained = local_projection_snapshot_fixture(
            2,
            2,
            0,
            &["online-message", "replacement-message"],
            &[],
            &["online-message", "replacement-message"],
        );

        let error = assert_local_projection_counts(
            &drained,
            LocalProjectionExpectation {
                expected_messages: 2,
                expected_delivered: 2,
                expected_undelivered: 0,
                required_delivered_message_ids: &["online-message", "offline-message"],
            },
            "after restart",
        )
        .expect_err("changed local identity should fail");

        assert!(
            error
                .to_string()
                .contains("expected message id `offline-message`")
        );
    }

    #[test]
    fn local_projection_counts_reject_duplicate_visible_ids() {
        let duplicate = local_projection_snapshot_fixture(
            2,
            2,
            0,
            &["same-message", "same-message"],
            &[],
            &["same-message", "same-message"],
        );

        let error = assert_local_projection_counts(
            &duplicate,
            LocalProjectionExpectation {
                expected_messages: 2,
                expected_delivered: 2,
                expected_undelivered: 0,
                required_delivered_message_ids: &["same-message"],
            },
            "after restart",
        )
        .expect_err("duplicate visible ids should fail");

        assert!(
            error
                .to_string()
                .contains("duplicate visible outbound message id")
        );
    }

    #[test]
    fn local_projection_same_visible_outbound_accepts_unchanged_attachment_fail_fast() {
        let before = local_projection_snapshot_fixture(
            2,
            1,
            1,
            &["online-message"],
            &["offline-message"],
            &["online-message", "offline-message"],
        );
        let after = local_projection_snapshot_fixture(
            2,
            1,
            1,
            &["online-message"],
            &["offline-message"],
            &["online-message", "offline-message"],
        );

        assert_local_projection_same_visible_outbound(&after, &before, "after offline attachment")
            .expect("attachment fail-fast should leave visible outbound ids unchanged");
    }

    #[test]
    fn local_projection_same_visible_outbound_rejects_attachment_bubble() {
        let before = local_projection_snapshot_fixture(
            2,
            1,
            1,
            &["online-message"],
            &["offline-message"],
            &["online-message", "offline-message"],
        );
        let after = local_projection_snapshot_fixture(
            3,
            1,
            2,
            &["online-message"],
            &["offline-message", "attachment-message"],
            &["online-message", "offline-message", "attachment-message"],
        );

        let error = assert_local_projection_same_visible_outbound(
            &after,
            &before,
            "after offline attachment",
        )
        .expect_err("attachment bubble should fail");
        assert!(
            error
                .to_string()
                .contains("leave visible outbound ids unchanged"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn local_outbox_counts_accept_offline_row_and_empty_drain() {
        let offline = local_outbox_snapshot_fixture(&[("simulator-a", "room-main", "offline")]);
        assert_local_outbox_counts(
            &offline,
            "simulator-a",
            OutboxExpectation {
                expected_rows: 1,
                required_room_id: Some("room-main"),
                required_message_id: Some("offline"),
                expected_local_state: Some("sent"),
                expected_server_delivery_state: Some("undelivered"),
            },
            "after offline",
        )
        .expect("offline durable outbox row");

        let drained = local_outbox_snapshot_fixture(&[]);
        assert_local_outbox_counts(
            &drained,
            "simulator-a",
            OutboxExpectation {
                expected_rows: 0,
                required_room_id: None,
                required_message_id: None,
                expected_local_state: None,
                expected_server_delivery_state: None,
            },
            "after restart",
        )
        .expect("drained outbox");
    }

    #[test]
    fn local_outbox_counts_rejects_underspecified_expectations() {
        let offline = local_outbox_snapshot_fixture(&[("simulator-a", "room-main", "offline")]);
        let missing_state = assert_local_outbox_counts(
            &offline,
            "simulator-a",
            OutboxExpectation {
                expected_rows: 1,
                required_room_id: Some("room-main"),
                required_message_id: Some("offline"),
                expected_local_state: None,
                expected_server_delivery_state: Some("undelivered"),
            },
            "after offline",
        )
        .expect_err("non-empty outbox expectation should require exact state");
        assert!(
            missing_state
                .to_string()
                .contains("must specify room id plus local and server delivery state"),
            "unexpected error: {missing_state}"
        );

        let missing_room = assert_local_outbox_counts(
            &offline,
            "simulator-a",
            OutboxExpectation {
                expected_rows: 1,
                required_room_id: None,
                required_message_id: Some("offline"),
                expected_local_state: Some("sent"),
                expected_server_delivery_state: Some("undelivered"),
            },
            "after offline",
        )
        .expect_err("non-empty outbox expectation should require room identity");
        assert!(
            missing_room
                .to_string()
                .contains("must specify room id plus local and server delivery state"),
            "unexpected error: {missing_room}"
        );

        let drained = local_outbox_snapshot_fixture(&[]);
        let stale_expectation = assert_local_outbox_counts(
            &drained,
            "simulator-a",
            OutboxExpectation {
                expected_rows: 0,
                required_room_id: Some("room-main"),
                required_message_id: Some("offline"),
                expected_local_state: Some("sent"),
                expected_server_delivery_state: Some("undelivered"),
            },
            "after restart",
        )
        .expect_err("empty outbox expectation should not carry stale row identity");
        assert!(
            stale_expectation
                .to_string()
                .contains("must not specify row identity or delivery state"),
            "unexpected error: {stale_expectation}"
        );
    }

    #[test]
    fn local_outbox_counts_reject_missing_visible_message_id() {
        let snapshot = local_outbox_snapshot_fixture(&[("simulator-a", "room-main", "other")]);

        let error = assert_local_outbox_counts(
            &snapshot,
            "simulator-a",
            OutboxExpectation {
                expected_rows: 1,
                required_room_id: Some("room-main"),
                required_message_id: Some("offline"),
                expected_local_state: Some("sent"),
                expected_server_delivery_state: Some("undelivered"),
            },
            "after offline",
        )
        .expect_err("missing visible outbox identity should fail");

        assert!(
            error
                .to_string()
                .contains("expected durable outbox row for visible message id `offline`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn local_outbox_counts_reject_wrong_room_id() {
        let snapshot = local_outbox_snapshot_fixture(&[("simulator-a", "other-room", "offline")]);

        let error = assert_local_outbox_counts(
            &snapshot,
            "simulator-a",
            OutboxExpectation {
                expected_rows: 1,
                required_room_id: Some("room-main"),
                required_message_id: Some("offline"),
                expected_local_state: Some("sent"),
                expected_server_delivery_state: Some("undelivered"),
            },
            "after offline",
        )
        .expect_err("wrong outbox room identity should fail");

        assert!(
            error
                .to_string()
                .contains("expected durable outbox row in room `room-main`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn local_outbox_counts_reject_metadata_that_changes_visible_identity() {
        let mut snapshot =
            local_outbox_snapshot_fixture(&[("simulator-a", "room-main", "offline")]);
        snapshot.append_request_message_ids = vec!["replacement".to_owned()];

        let error = assert_local_outbox_counts(
            &snapshot,
            "simulator-a",
            OutboxExpectation {
                expected_rows: 1,
                required_room_id: Some("room-main"),
                required_message_id: Some("offline"),
                expected_local_state: Some("sent"),
                expected_server_delivery_state: Some("undelivered"),
            },
            "after offline",
        )
        .expect_err("changed append identity should fail");

        assert!(
            error.to_string().contains("append request message ids"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn local_outbox_counts_reject_missing_idempotency_material() {
        let mut snapshot =
            local_outbox_snapshot_fixture(&[("simulator-a", "room-main", "offline")]);
        snapshot.idempotency_material_rows = 0;

        let error = assert_local_outbox_counts(
            &snapshot,
            "simulator-a",
            OutboxExpectation {
                expected_rows: 1,
                required_room_id: Some("room-main"),
                required_message_id: Some("offline"),
                expected_local_state: Some("sent"),
                expected_server_delivery_state: Some("undelivered"),
            },
            "after offline",
        )
        .expect_err("missing idempotency material should fail");

        assert!(
            error.to_string().contains("expected idempotency material"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn local_outbox_counts_reject_wrong_durable_delivery_state() {
        let mut snapshot =
            local_outbox_snapshot_fixture(&[("simulator-a", "room-main", "offline")]);
        snapshot.local_states = vec!["sending".to_owned()];
        snapshot.server_delivery_states = vec!["failed".to_owned()];

        let error = assert_local_outbox_counts(
            &snapshot,
            "simulator-a",
            OutboxExpectation {
                expected_rows: 1,
                required_room_id: Some("room-main"),
                required_message_id: Some("offline"),
                expected_local_state: Some("sent"),
                expected_server_delivery_state: Some("undelivered"),
            },
            "after offline",
        )
        .expect_err("wrong outbox state should fail");

        assert!(
            error
                .to_string()
                .contains("expected durable outbox local state `sent`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn local_outbox_snapshot_reads_client_sqlite_keys() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store_path = temp.path().join("FiniteChatStore");
        fs::create_dir_all(&store_path).expect("store path");
        let conn = Connection::open(store_path.join("client.sqlite3")).expect("open sqlite");
        conn.execute_batch(
            r#"
            CREATE TABLE client_app_outbox (
              account_id TEXT NOT NULL,
              device_id TEXT NOT NULL,
              room_id TEXT NOT NULL,
              message_id TEXT NOT NULL,
              nonce BLOB NOT NULL,
              ciphertext BLOB NOT NULL,
              PRIMARY KEY (account_id, device_id, room_id, message_id)
            );
            "#,
        )
        .expect("schema");
        conn.execute(
            r#"
            INSERT INTO client_app_outbox (
              account_id,
              device_id,
              room_id,
              message_id,
              nonce,
              ciphertext
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                "account",
                "simulator-a",
                "room-main",
                "offline-message",
                b"nonce",
                b"ciphertext"
            ],
        )
        .expect("insert outbox row");

        let rows = read_local_outbox_key_rows(&store_path).expect("outbox keys");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].device_id, "simulator-a");
        assert_eq!(rows[0].room_id, "room-main");
        assert_eq!(rows[0].message_id, "offline-message");
    }

    #[test]
    fn peer_receipt_counts_accept_exactly_once_inbound_message() {
        let snapshot = peer_receipt_snapshot_fixture(1, 1, 0, 0, &["simulator-a"]);

        assert_peer_receipt_counts(&snapshot, "simulator-a", "after restart")
            .expect("exactly once inbound peer receipt");
    }

    #[test]
    fn peer_receipt_counts_reject_duplicate_message() {
        let snapshot = peer_receipt_snapshot_fixture(2, 2, 0, 0, &["simulator-a"]);

        let error = assert_peer_receipt_counts(&snapshot, "simulator-a", "after restart")
            .expect_err("duplicate peer receipt should fail");

        assert!(
            error
                .to_string()
                .contains("to receive message `offline-message` exactly once")
        );
    }

    #[test]
    fn peer_receipt_counts_reject_outbound_delivery_on_peer_message() {
        let snapshot = peer_receipt_snapshot_fixture(1, 0, 0, 1, &["simulator-a"]);

        let error = assert_peer_receipt_counts(&snapshot, "simulator-a", "after restart")
            .expect_err("peer inbound message must not carry outbound_delivery");

        assert!(
            error
                .to_string()
                .contains("one inbound row with no outbound_delivery")
        );
    }

    fn peer_receipt_snapshot_fixture(
        matching: usize,
        inbound: usize,
        local: usize,
        with_outbound_delivery: usize,
        sender_devices: &[&str],
    ) -> PeerReceiptSnapshot {
        PeerReceiptSnapshot {
            room_id: "room-main".to_owned(),
            peer_device: "simulator-a-peer".to_owned(),
            message_id: "offline-message".to_owned(),
            total_peer_messages: matching,
            matching_messages: matching,
            inbound_matching_messages: inbound,
            local_matching_messages: local,
            matching_with_outbound_delivery: with_outbound_delivery,
            sender_devices: sender_devices
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }

    fn local_projection_snapshot_fixture(
        messages: usize,
        delivered: usize,
        undelivered: usize,
        delivered_ids: &[&str],
        undelivered_ids: &[&str],
        visible_ids: &[&str],
    ) -> LocalProjectionSnapshot {
        LocalProjectionSnapshot {
            rooms: 1,
            connected_rooms: 1,
            unavailable_on_device_rooms: 0,
            selected_room_id: Some("room-main".to_owned()),
            messages,
            local_outbound_messages: messages,
            delivered_outbound_messages: delivered,
            undelivered_outbound_messages: undelivered,
            failed_outbound_messages: 0,
            sending_outbound_messages: 0,
            nonlocal_outbound_delivery_messages: 0,
            delivered_message_ids: delivered_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            undelivered_message_ids: undelivered_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            visible_outbound_message_ids: visible_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }

    fn local_outbox_snapshot_fixture(rows: &[(&str, &str, &str)]) -> LocalOutboxSnapshot {
        let device_ids = rows
            .iter()
            .map(|(device, _room, _message)| (*device).to_owned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let room_ids = rows
            .iter()
            .map(|(_device, room, _message)| (*room).to_owned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let message_ids: Vec<String> = rows
            .iter()
            .map(|(_device, _room, message)| (*message).to_owned())
            .collect();
        LocalOutboxSnapshot {
            rows: rows.len(),
            device_ids,
            room_ids,
            local_states: vec!["sent".to_owned(); message_ids.len()],
            server_delivery_states: vec!["undelivered".to_owned(); message_ids.len()],
            append_request_message_ids: message_ids.clone(),
            idempotency_material_rows: message_ids.len(),
            message_ids,
        }
    }

    fn seed_server_delivery_db(path: &Path, publish_messages: i64, sender_device: &str) {
        let conn = Connection::open(path).expect("open sqlite");
        conn.execute_batch(
            r#"
            CREATE TABLE http_delivery_ops (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                body_json TEXT NOT NULL
            );
            CREATE TABLE http_application_delivery_effects (
                message_id TEXT PRIMARY KEY,
                room_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                sender_json TEXT NOT NULL,
                delivery_policy_json TEXT NOT NULL
            );
            CREATE TABLE http_publish_idempotency (
                idempotency_key TEXT PRIMARY KEY,
                fingerprint_json TEXT NOT NULL,
                receipt_json TEXT NOT NULL
            );
            "#,
        )
        .expect("create tables");
        for index in 0..publish_messages {
            conn.execute(
                "INSERT INTO http_delivery_ops (kind, body_json) VALUES ('publish_message', ?1)",
                [format!(r#"{{"message":{index}}}"#)],
            )
            .expect("insert delivery op");
            conn.execute(
                "INSERT INTO http_application_delivery_effects (
                    message_id,
                    room_id,
                    seq,
                    sender_json,
                    delivery_policy_json
                ) VALUES (?1, 'room-main', ?2, ?3, '{}')",
                (
                    format!("message-{index}"),
                    index + 1,
                    serde_json::json!({
                        "account_id": "account-main",
                        "device_id": sender_device,
                    })
                    .to_string(),
                ),
            )
            .expect("insert app effect");
            conn.execute(
                "INSERT INTO http_publish_idempotency (
                    idempotency_key,
                    fingerprint_json,
                    receipt_json
                ) VALUES (?1, '{}', '{}')",
                [format!("idempotency-{index}")],
            )
            .expect("insert idempotency");
        }
    }
}
