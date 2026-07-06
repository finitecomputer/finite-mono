use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::bindings;
use crate::bindings::BuildProfile;
use crate::cli::{CliError, JsonOk, human_log, json_print};
use crate::config::load_rmp_toml;
use crate::util::{discover_xcode_dev_dir, run_capture};

const IOS_DEVICE_STORE_DIR: &str = "FiniteChatStore";
const IOS_DEVICE_STORE_SOURCE: &str = "Library/Application Support/FiniteChatStore";
const IOS_DEVICE_STORE_DESTINATION: &str = "Library/Application Support";

pub fn run(
    root: &Path,
    json: bool,
    verbose: bool,
    args: crate::cli::RunArgs,
) -> Result<(), CliError> {
    match args.platform {
        crate::cli::RunPlatform::Ios => run_ios(root, json, verbose, args.ios, args.release),
        crate::cli::RunPlatform::Android => {
            run_android(root, json, verbose, args.android, args.release)
        }
        crate::cli::RunPlatform::Iced => run_iced(root, json, verbose, args.release),
    }
}

fn default_app_relay_csv() -> String {
    std::env::var("FINITECHAT_SERVER_URL").unwrap_or_default()
}

fn default_app_kp_relay_csv() -> String {
    String::new()
}

fn csv_override_from_env_with<F>(
    get: F,
    primary_key: &str,
    secondary_key: &str,
    default_csv: fn() -> String,
) -> String
where
    F: Fn(&str) -> Option<String>,
{
    get(primary_key)
        .or_else(|| get(secondary_key))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(default_csv)
}

fn relay_csv_from_env() -> String {
    csv_override_from_env_with(
        |key| std::env::var(key).ok(),
        "FINITECHAT_RELAY_URLS",
        "FINITECHAT_SERVER_URL",
        default_app_relay_csv,
    )
}

fn kp_relay_csv_from_env() -> String {
    csv_override_from_env_with(
        |key| std::env::var(key).ok(),
        "FINITECHAT_KEY_PACKAGE_RELAY_URLS",
        "FINITECHAT_KP_RELAY_URLS",
        default_app_kp_relay_csv,
    )
}

fn run_ios(
    root: &Path,
    json: bool,
    verbose: bool,
    args: crate::cli::RunIosArgs,
    release: bool,
) -> Result<(), CliError> {
    // When an explicit --udid is supplied, decide whether it names a simulator or
    // an attached physical device and route accordingly. Without --udid we keep the
    // historical behavior of provisioning/booting a default simulator.
    if let Some(requested) = args.udid.clone() {
        let dev_dir = discover_xcode_dev_dir()?;
        let simulator_udids = collect_simulator_udids(&dev_dir).unwrap_or_default();
        if classify_ios_udid(&requested, &simulator_udids) == IosTargetKind::PhysicalDevice {
            human_log(
                verbose,
                format!("routing --udid {requested} to physical iOS device install"),
            );
            return run_ios_device(root, json, verbose, args, release);
        }
    }

    let installed = build_install_ios_simulator(root, verbose, args, release)?;

    // Optional local runtime config for simulator runs.
    if std::env::var("FINITECHAT_RELAY_URLS").is_ok()
        || std::env::var("FINITECHAT_SERVER_URL").is_ok()
    {
        maybe_write_ios_relay_config(
            &installed.dev_dir,
            &installed.udid,
            &installed.bundle_id,
            verbose,
        )?;
    }

    launch_ios_simulator_app(
        &installed.dev_dir,
        &installed.udid,
        &installed.bundle_id,
        &[],
        verbose,
    )?;

    let _ = Command::new("open").arg("-a").arg("Simulator").status();

    if json {
        json_print(&JsonOk {
            ok: true,
            data: serde_json::json!({"platform":"ios","kind":"simulator","udid":installed.udid,"bundle_id":installed.bundle_id}),
        });
    } else {
        eprintln!("ok: ios app launched (simulator)");
    }
    Ok(())
}

fn run_ios_device(
    root: &Path,
    json: bool,
    verbose: bool,
    args: crate::cli::RunIosArgs,
    release: bool,
) -> Result<(), CliError> {
    // Build the Rust core + xcframework, xcodebuild for iphoneos with managed
    // signing, and devicectl-install onto the attached device. reset_app=false so a
    // routine "push a new build" preserves the app's on-device data.
    let installed = build_install_ios_device(root, verbose, args, release, false)?;

    let _pid = launch_ios_device_app(
        &installed.dev_dir,
        &installed.udid,
        &installed.bundle_id,
        &[],
        verbose,
    )?;

    if json {
        json_print(&JsonOk {
            ok: true,
            data: serde_json::json!({"platform":"ios","kind":"device","udid":installed.udid,"bundle_id":installed.bundle_id}),
        });
    } else {
        eprintln!("ok: ios app launched (device)");
    }
    Ok(())
}

/// Whether an explicitly requested iOS `--udid` names a booted-or-bootable
/// simulator or an attached physical device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IosTargetKind {
    Simulator,
    PhysicalDevice,
}

/// Classify an explicitly requested iOS `--udid`.
///
/// `simulator_udids` should be the set of UDIDs reported by `simctl list
/// devices`. When that query succeeds the set is authoritative for simulators.
/// When it is empty (query failed) we fall back to a shape heuristic: simulator
/// UDIDs are standard 8-4-4-4-12 UUIDs (four dashes) while modern hardware UDIDs
/// look like `XXXXXXXX-XXXXXXXXXXXXXXXX` (a single dash), so anything that is not
/// simulator-shaped is treated as a physical device.
pub(crate) fn classify_ios_udid(
    udid: &str,
    simulator_udids: &std::collections::HashSet<String>,
) -> IosTargetKind {
    if simulator_udids.contains(udid) {
        return IosTargetKind::Simulator;
    }
    if is_simulator_shaped_udid(udid) {
        IosTargetKind::Simulator
    } else {
        IosTargetKind::PhysicalDevice
    }
}

fn is_simulator_shaped_udid(udid: &str) -> bool {
    let groups: Vec<&str> = udid.split('-').collect();
    let expected = [8usize, 4, 4, 4, 12];
    groups.len() == expected.len()
        && groups
            .iter()
            .zip(expected)
            .all(|(group, len)| group.len() == len && group.chars().all(|c| c.is_ascii_hexdigit()))
}

fn collect_simulator_udids(dev_dir: &Path) -> Result<std::collections::HashSet<String>, CliError> {
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.env("DEVELOPER_DIR", dev_dir)
        .arg("simctl")
        .arg("list")
        .arg("-j")
        .arg("devices");
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational("failed to list simulators"));
    }
    let value: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| CliError::operational(format!("failed to parse simulator JSON: {e}")))?;
    Ok(simulator_udids_from_value(&value))
}

fn simulator_udids_from_value(value: &serde_json::Value) -> std::collections::HashSet<String> {
    let mut udids = std::collections::HashSet::new();
    if let Some(runtimes) = value.get("devices").and_then(|v| v.as_object()) {
        for devices in runtimes.values() {
            for dev in devices.as_array().into_iter().flatten() {
                if let Some(udid) = dev.get("udid").and_then(|v| v.as_str()) {
                    udids.insert(udid.to_string());
                }
            }
        }
    }
    udids
}

pub(crate) struct IosInstalledApp {
    pub dev_dir: PathBuf,
    pub udid: String,
    pub bundle_id: String,
}

pub(crate) struct IosDeviceInstalledApp {
    pub dev_dir: PathBuf,
    pub udid: String,
    pub bundle_id: String,
}

pub(crate) fn build_install_ios_simulator(
    root: &Path,
    verbose: bool,
    args: crate::cli::RunIosArgs,
    release: bool,
) -> Result<IosInstalledApp, CliError> {
    let cfg = load_rmp_toml(root)?;
    let ios = cfg
        .ios
        .ok_or_else(|| CliError::user("rmp.toml missing [ios] section"))?;
    let dev_dir = discover_xcode_dev_dir()?;
    let profile = build_profile(release);
    let (rust_target, xcode_arch) = ios_sim_target_for_host()?;

    let udid = ensure_ios_simulator(&dev_dir, args.udid.as_deref(), verbose)?;

    // Build bindings + xcframework for a single simulator arch.
    bindings::build_swift_for_run(root, rust_target, profile, verbose)?;

    // Generate Xcode project.
    human_log(verbose, "xcodegen generate");
    let status = Command::new("xcodegen")
        .current_dir(root.join("ios"))
        .arg("generate")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run xcodegen: {e}")))?;
    if !status.success() {
        return Err(CliError::operational("xcodegen generate failed"));
    }

    let bundle_id = ios.bundle_id;
    let xcode_name = read_xcode_project_name(root).unwrap_or_else(|| "App".to_string());
    let xcode_scheme = ios.scheme.clone().unwrap_or_else(|| xcode_name.clone());
    let xcode_config = if release { "Release" } else { "Debug" };

    let xcode_project_path = root.join(format!("ios/{xcode_name}.xcodeproj"));

    // Build simulator .app.
    human_log(
        verbose,
        format!("xcodebuild ({xcode_config}, iphonesimulator, arch={xcode_arch})"),
    );
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.env("DEVELOPER_DIR", &dev_dir)
        .env_remove("LD")
        .env_remove("CC")
        .env_remove("CXX")
        .arg("xcodebuild")
        .arg("-project")
        .arg(&xcode_project_path)
        .arg("-scheme")
        .arg(&xcode_scheme)
        .arg("-destination")
        .arg(format!("id={udid}"))
        .arg("-configuration")
        .arg(xcode_config)
        .arg("-sdk")
        .arg("iphonesimulator")
        .arg("build")
        .arg(format!("ARCHS={xcode_arch}"))
        .arg("ONLY_ACTIVE_ARCH=YES")
        .arg("CODE_SIGNING_ALLOWED=NO")
        .arg(format!("PRODUCT_BUNDLE_IDENTIFIER={bundle_id}"));

    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run xcodebuild: {e}")))?;
    if !status.success() {
        return Err(CliError::operational("xcodebuild failed"));
    }

    let app_path = resolve_ios_app_path(
        &dev_dir,
        &xcode_project_path,
        &xcode_scheme,
        &udid,
        xcode_config,
        "iphonesimulator",
        xcode_arch,
    )?;
    if !app_path.is_dir() {
        return Err(CliError::operational(format!(
            "missing built app at {}",
            app_path.to_string_lossy()
        )));
    }

    // Install.
    human_log(verbose, format!("simctl install (udid={udid})"));
    let status = Command::new("/usr/bin/xcrun")
        .env("DEVELOPER_DIR", &dev_dir)
        .arg("simctl")
        .arg("install")
        .arg(&udid)
        .arg(&app_path)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run simctl install: {e}")))?;
    if !status.success() {
        return Err(CliError::operational("simctl install failed"));
    }

    Ok(IosInstalledApp {
        dev_dir,
        udid,
        bundle_id,
    })
}

pub(crate) fn build_install_ios_device(
    root: &Path,
    verbose: bool,
    args: crate::cli::RunIosArgs,
    release: bool,
    reset_app: bool,
) -> Result<IosDeviceInstalledApp, CliError> {
    let cfg = load_rmp_toml(root)?;
    let ios = cfg
        .ios
        .ok_or_else(|| CliError::user("rmp.toml missing [ios] section"))?;
    let dev_dir = discover_xcode_dev_dir()?;
    let profile = build_profile(release);
    let crate::cli::RunIosArgs {
        udid,
        development_team,
    } = args;
    let requested_udid = udid.ok_or_else(|| {
        CliError::user("product harness ios-device requires --udid for an attached physical iPhone")
    })?;
    let development_team = resolve_ios_development_team(development_team)?;
    let udid = resolve_ios_device_udid(&dev_dir, &requested_udid)?;

    bindings::build_swift_for_run(root, "aarch64-apple-ios", profile, verbose)?;

    human_log(verbose, "xcodegen generate");
    let status = Command::new("xcodegen")
        .current_dir(root.join("ios"))
        .arg("generate")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run xcodegen: {e}")))?;
    if !status.success() {
        return Err(CliError::operational("xcodegen generate failed"));
    }

    let bundle_id = ios.bundle_id;
    if reset_app {
        uninstall_ios_device_app(&dev_dir, &udid, &bundle_id, verbose)?;
    }

    let xcode_name = read_xcode_project_name(root).unwrap_or_else(|| "App".to_string());
    let xcode_scheme = ios.scheme.clone().unwrap_or_else(|| xcode_name.clone());
    let xcode_config = if release { "Release" } else { "Debug" };
    let xcode_arch = "arm64";
    let xcode_project_path = root.join(format!("ios/{xcode_name}.xcodeproj"));

    human_log(
        verbose,
        format!("xcodebuild ({xcode_config}, iphoneos, arch={xcode_arch})"),
    );
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.env("DEVELOPER_DIR", &dev_dir)
        .env_remove("LD")
        .env_remove("CC")
        .env_remove("CXX")
        .arg("xcodebuild")
        .arg("-project")
        .arg(&xcode_project_path)
        .arg("-scheme")
        .arg(&xcode_scheme)
        .arg("-destination")
        .arg(format!("id={udid}"))
        .arg("-configuration")
        .arg(xcode_config)
        .arg("-sdk")
        .arg("iphoneos");
    if development_team.is_some() {
        cmd.arg("-allowProvisioningUpdates")
            .arg("-allowProvisioningDeviceRegistration");
    }
    cmd.arg("build")
        .arg(format!("ARCHS={xcode_arch}"))
        .arg("ONLY_ACTIVE_ARCH=YES")
        .arg(format!("PRODUCT_BUNDLE_IDENTIFIER={bundle_id}"));
    if let Some(team) = development_team.as_deref() {
        cmd.arg(format!("DEVELOPMENT_TEAM={team}"));
    }

    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run xcodebuild: {e}")))?;
    if !status.success() {
        return Err(CliError::operational("xcodebuild failed"));
    }

    let app_path = resolve_ios_app_path(
        &dev_dir,
        &xcode_project_path,
        &xcode_scheme,
        &udid,
        xcode_config,
        "iphoneos",
        xcode_arch,
    )?;
    if !app_path.is_dir() {
        return Err(CliError::operational(format!(
            "missing built app at {}",
            app_path.to_string_lossy()
        )));
    }

    devicectl_install_app_with_retry(&dev_dir, &udid, &app_path, verbose)?;

    Ok(IosDeviceInstalledApp {
        dev_dir,
        udid,
        bundle_id,
    })
}

/// Run `devicectl device install app`, retrying a few times.
///
/// The first install after the device has been idle frequently fails while
/// CoreDevice mounts the developer disk image (e.g. error 4000, "the device
/// disconnected immediately after connecting"); a plain retry then succeeds. We
/// stream output so the operator still sees progress, and only surface a failure
/// after all attempts are exhausted.
fn devicectl_install_app_with_retry(
    dev_dir: &Path,
    udid: &str,
    app_path: &Path,
    verbose: bool,
) -> Result<(), CliError> {
    const MAX_ATTEMPTS: usize = 3;
    for attempt in 1..=MAX_ATTEMPTS {
        human_log(
            verbose,
            format!("devicectl install app (udid={udid}, attempt {attempt}/{MAX_ATTEMPTS})"),
        );
        let status = Command::new("/usr/bin/xcrun")
            .env("DEVELOPER_DIR", dev_dir)
            .arg("devicectl")
            .arg("device")
            .arg("install")
            .arg("app")
            .arg("--device")
            .arg(udid)
            .arg(app_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| CliError::operational(format!("failed to run devicectl install: {e}")))?;
        if status.success() {
            return Ok(());
        }
        if attempt < MAX_ATTEMPTS {
            human_log(
                verbose,
                "devicectl install app failed (often a transient CoreDevice disconnect); retrying",
            );
            thread::sleep(Duration::from_secs(3));
        }
    }
    Err(CliError::operational(format!(
        "devicectl install app failed after {MAX_ATTEMPTS} attempts"
    )))
}

pub(crate) fn launch_ios_simulator_app(
    dev_dir: &Path,
    udid: &str,
    bundle_id: &str,
    launch_args: &[String],
    verbose: bool,
) -> Result<(), CliError> {
    let _ = terminate_ios_simulator_app(dev_dir, udid, bundle_id, verbose);

    human_log(verbose, "simctl launch");
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.env("DEVELOPER_DIR", dev_dir)
        .arg("simctl")
        .arg("launch")
        .arg(udid)
        .arg(bundle_id);
    for arg in launch_args {
        cmd.arg(arg);
    }
    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run simctl launch: {e}")))?;
    if !status.success() {
        return Err(CliError::operational("simctl launch failed"));
    }
    Ok(())
}

pub(crate) fn launch_ios_device_app(
    dev_dir: &Path,
    udid: &str,
    bundle_id: &str,
    launch_args: &[String],
    verbose: bool,
) -> Result<u64, CliError> {
    human_log(verbose, "devicectl launch");
    let json_output = tempfile::NamedTempFile::new()
        .map_err(|error| CliError::operational(format!("failed to create temp file: {error}")))?;
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.env("DEVELOPER_DIR", dev_dir)
        .arg("devicectl")
        .arg("device")
        .arg("process")
        .arg("launch")
        .arg("--device")
        .arg(udid)
        .arg("--terminate-existing")
        .arg("--json-output")
        .arg(json_output.path())
        .arg(bundle_id);
    for arg in launch_args {
        cmd.arg(arg);
    }
    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run devicectl launch: {e}")))?;
    if !status.success() {
        return Err(CliError::operational("devicectl launch failed"));
    }
    let bytes = fs::read(json_output.path()).map_err(|error| {
        CliError::operational(format!(
            "failed to read devicectl launch JSON {}: {error}",
            json_output.path().display()
        ))
    })?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::operational(format!("failed to parse devicectl launch JSON: {error}"))
    })?;
    find_process_identifier(&value).ok_or_else(|| {
        CliError::operational(
            "devicectl launch did not report a process identifier; cannot force-close app",
        )
    })
}

pub(crate) fn terminate_ios_device_app(
    dev_dir: &Path,
    udid: &str,
    pid: u64,
    verbose: bool,
) -> Result<(), CliError> {
    human_log(verbose, format!("devicectl terminate (pid={pid})"));
    let status = Command::new("/usr/bin/xcrun")
        .env("DEVELOPER_DIR", dev_dir)
        .arg("devicectl")
        .arg("device")
        .arg("process")
        .arg("terminate")
        .arg("--device")
        .arg(udid)
        .arg("--pid")
        .arg(pid.to_string())
        .arg("--kill")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run devicectl terminate: {e}")))?;
    if !status.success() {
        return Err(CliError::operational("devicectl terminate failed"));
    }
    Ok(())
}

pub(crate) fn pull_ios_device_app_store(
    dev_dir: &Path,
    udid: &str,
    bundle_id: &str,
    store_path: &Path,
    verbose: bool,
) -> Result<(), CliError> {
    checked_ios_device_store_path(store_path)?;
    let support_root = store_path.parent().ok_or_else(|| {
        CliError::operational(format!("store path {} has no parent", store_path.display()))
    })?;
    fs::create_dir_all(support_root).map_err(|error| {
        CliError::operational(format!(
            "failed to create support root {}: {error}",
            support_root.display()
        ))
    })?;
    let pull_parent = support_root.join(".device-pull-FiniteChatStore");
    let _ = fs::remove_dir_all(&pull_parent);
    fs::create_dir_all(&pull_parent).map_err(|error| {
        CliError::operational(format!(
            "failed to create device pull directory {}: {error}",
            pull_parent.display()
        ))
    })?;

    human_log(verbose, "devicectl copy app store from device");
    let status = Command::new("/usr/bin/xcrun")
        .env("DEVELOPER_DIR", dev_dir)
        .arg("devicectl")
        .arg("device")
        .arg("copy")
        .arg("from")
        .arg("--device")
        .arg(udid)
        .arg("--domain-type")
        .arg("appDataContainer")
        .arg("--domain-identifier")
        .arg(bundle_id)
        .arg("--source")
        .arg(IOS_DEVICE_STORE_SOURCE)
        .arg("--destination")
        .arg(&pull_parent)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run devicectl copy from: {e}")))?;
    if !status.success() {
        return Err(CliError::operational(
            "devicectl copy from appDataContainer failed",
        ));
    }

    let source_path = pulled_ios_device_store_source(&pull_parent)?;
    let _ = fs::remove_dir_all(store_path);
    fs::rename(&source_path, store_path).map_err(|error| {
        CliError::operational(format!(
            "failed to move pulled device store from {} to {}: {error}",
            source_path.display(),
            store_path.display()
        ))
    })?;
    let _ = fs::remove_dir_all(&pull_parent);
    if !store_path.is_dir() {
        return Err(CliError::operational(format!(
            "pulled device store is missing at {}",
            store_path.display()
        )));
    }
    Ok(())
}

pub(crate) fn push_ios_device_app_store(
    dev_dir: &Path,
    udid: &str,
    bundle_id: &str,
    store_path: &Path,
    verbose: bool,
) -> Result<(), CliError> {
    checked_ios_device_store_path(store_path)?;
    if !store_path.is_dir() {
        return Err(CliError::operational(format!(
            "cannot push missing device store {}",
            store_path.display()
        )));
    }
    human_log(verbose, "devicectl copy app store to device");
    let status = Command::new("/usr/bin/xcrun")
        .env("DEVELOPER_DIR", dev_dir)
        .arg("devicectl")
        .arg("device")
        .arg("copy")
        .arg("to")
        .arg("--device")
        .arg(udid)
        .arg("--domain-type")
        .arg("appDataContainer")
        .arg("--domain-identifier")
        .arg(bundle_id)
        .arg("--source")
        .arg(store_path)
        .arg("--destination")
        .arg(IOS_DEVICE_STORE_DESTINATION)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run devicectl copy to: {e}")))?;
    if !status.success() {
        return Err(CliError::operational(
            "devicectl copy to appDataContainer failed",
        ));
    }
    Ok(())
}

fn checked_ios_device_store_path(store_path: &Path) -> Result<(), CliError> {
    if store_path.file_name().and_then(|name| name.to_str()) != Some(IOS_DEVICE_STORE_DIR) {
        return Err(CliError::operational(format!(
            "device store path must end with {IOS_DEVICE_STORE_DIR}: {}",
            store_path.display()
        )));
    }
    Ok(())
}

fn pulled_ios_device_store_source(pull_parent: &Path) -> Result<PathBuf, CliError> {
    let copied_store = pull_parent.join(IOS_DEVICE_STORE_DIR);
    if copied_store.is_dir() {
        return Ok(copied_store);
    }
    if is_ios_device_store_root(pull_parent) {
        return Ok(pull_parent.to_owned());
    }
    Err(CliError::operational(format!(
        "devicectl copy did not produce expected {IOS_DEVICE_STORE_DIR} directory or direct store contents under {}",
        pull_parent.display()
    )))
}

fn is_ios_device_store_root(path: &Path) -> bool {
    // Stores no longer contain account-secret.hex (the account key moved to
    // the shared Finite identity); the encrypted client store is the marker.
    path.join("client.sqlite3").is_file()
}

pub(crate) fn terminate_ios_simulator_app(
    dev_dir: &Path,
    udid: &str,
    bundle_id: &str,
    verbose: bool,
) -> Result<(), CliError> {
    human_log(verbose, "simctl terminate");
    let status = Command::new("/usr/bin/xcrun")
        .env("DEVELOPER_DIR", dev_dir)
        .arg("simctl")
        .arg("terminate")
        .arg(udid)
        .arg(bundle_id)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run simctl terminate: {e}")))?;
    if !status.success() {
        human_log(
            verbose,
            format!("simctl terminate returned status {status}; continuing"),
        );
    }
    Ok(())
}

fn uninstall_ios_device_app(
    dev_dir: &Path,
    udid: &str,
    bundle_id: &str,
    verbose: bool,
) -> Result<(), CliError> {
    human_log(verbose, format!("devicectl uninstall app (udid={udid})"));
    let status = Command::new("/usr/bin/xcrun")
        .env("DEVELOPER_DIR", dev_dir)
        .arg("devicectl")
        .arg("device")
        .arg("uninstall")
        .arg("app")
        .arg("--device")
        .arg(udid)
        .arg(bundle_id)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run devicectl uninstall: {e}")))?;
    if !status.success() {
        human_log(
            verbose,
            format!("devicectl uninstall returned status {status}; continuing"),
        );
    }
    Ok(())
}

fn resolve_ios_device_udid(dev_dir: &Path, requested: &str) -> Result<String, CliError> {
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.env("DEVELOPER_DIR", dev_dir)
        .arg("xctrace")
        .arg("list")
        .arg("devices");
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational("failed to list iOS devices"));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.contains(requested) {
        return Ok(requested.to_owned());
    }
    if let Some(udid) = resolve_ios_device_udid_from_devicectl(dev_dir, requested)?
        && stdout.contains(&udid)
    {
        return Ok(udid);
    }

    Err(CliError::user(format!(
        "requested physical iOS device identifier or UDID not found: {requested}"
    )))
}

fn resolve_ios_device_udid_from_devicectl(
    dev_dir: &Path,
    requested: &str,
) -> Result<Option<String>, CliError> {
    let json_output = tempfile::NamedTempFile::new()
        .map_err(|error| CliError::operational(format!("failed to create temp file: {error}")))?;
    let status = Command::new("/usr/bin/xcrun")
        .env("DEVELOPER_DIR", dev_dir)
        .arg("devicectl")
        .arg("list")
        .arg("devices")
        .arg("--json-output")
        .arg(json_output.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run devicectl list devices: {e}")))?;
    if !status.success() {
        return Ok(None);
    }
    let bytes = fs::read(json_output.path()).map_err(|error| {
        CliError::operational(format!(
            "failed to read devicectl device JSON {}: {error}",
            json_output.path().display()
        ))
    })?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::operational(format!("failed to parse devicectl device JSON: {error}"))
    })?;
    Ok(resolve_ios_device_udid_from_devicectl_value(
        &value, requested,
    ))
}

fn resolve_ios_device_udid_from_devicectl_value(
    value: &serde_json::Value,
    requested: &str,
) -> Option<String> {
    let devices = value.get("result")?.get("devices")?.as_array()?;
    for device in devices {
        let identifier = device.get("identifier").and_then(|value| value.as_str());
        let hardware_udid = device
            .get("hardwareProperties")
            .and_then(|value| value.get("udid"))
            .and_then(|value| value.as_str());
        if identifier == Some(requested) || hardware_udid == Some(requested) {
            return hardware_udid.map(ToOwned::to_owned);
        }
    }
    None
}

pub(crate) fn resolve_ios_development_team(
    explicit: Option<String>,
) -> Result<Option<String>, CliError> {
    resolve_ios_development_team_with(explicit, std::env::var("RMP_IOS_DEVELOPMENT_TEAM").ok())
}

pub(crate) fn require_ios_development_team_with(
    explicit: Option<String>,
    env_value: Option<String>,
) -> Result<String, CliError> {
    resolve_ios_development_team_with(explicit, env_value)?.ok_or_else(|| {
        CliError::user(
            "product harness ios-device requires --ios-development-team or RMP_IOS_DEVELOPMENT_TEAM for physical iOS signing",
        )
    })
}

fn resolve_ios_development_team_with(
    explicit: Option<String>,
    env_value: Option<String>,
) -> Result<Option<String>, CliError> {
    explicit
        .or(env_value)
        .map(|team| checked_development_team(&team))
        .transpose()
}

pub(crate) fn checked_development_team(value: &str) -> Result<String, CliError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CliError::user("iOS development team id must not be empty"));
    }
    if !trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric())
    {
        return Err(CliError::user(
            "iOS development team id must contain only ASCII letters and digits",
        ));
    }
    Ok(trimmed.to_owned())
}

fn find_process_identifier(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let normalized = key
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>();
                if matches!(
                    normalized.as_str(),
                    "pid" | "processid" | "processidentifier"
                ) && let Some(pid) = json_u64(value)
                {
                    return Some(pid);
                }
            }
            map.values().find_map(find_process_identifier)
        }
        serde_json::Value::Array(values) => values.iter().find_map(find_process_identifier),
        _ => None,
    }
}

fn json_u64(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Number(number) => number.as_u64(),
        serde_json::Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

pub(crate) fn ensure_ios_simulator(
    dev_dir: &Path,
    explicit_udid: Option<&str>,
    verbose: bool,
) -> Result<String, CliError> {
    if let Some(u) = explicit_udid {
        // Validate exists.
        let mut cmd = Command::new("/usr/bin/xcrun");
        cmd.env("DEVELOPER_DIR", dev_dir)
            .arg("simctl")
            .arg("list")
            .arg("devices");
        let out = run_capture(cmd)?;
        if !out.status.success() {
            return Err(CliError::operational("failed to list simulators"));
        }
        let s = String::from_utf8_lossy(&out.stdout);
        if !s.contains(u) {
            return Err(CliError::user(format!(
                "requested simulator udid not found: {u}"
            )));
        }
        boot_sim(dev_dir, u, verbose)?;
        return Ok(u.to_string());
    }

    // Ensure at least one runtime exists.
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.env("DEVELOPER_DIR", dev_dir)
        .arg("simctl")
        .arg("list")
        .arg("-j")
        .arg("runtimes");
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational("failed to list simulator runtimes"));
    }
    let j: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| CliError::operational(format!("failed to parse runtimes JSON: {e}")))?;
    let mut runtimes: Vec<(u32, u32, String)> = vec![];
    for rt in j
        .get("runtimes")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let name = rt.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let ident = rt.get("identifier").and_then(|v| v.as_str()).unwrap_or("");
        let avail = rt
            .get("isAvailable")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !name.starts_with("iOS ") || !avail {
            continue;
        }
        // ident ends with iOS-18-6 style; parse.
        if let Some((maj, min)) = parse_ios_runtime_ident(ident) {
            runtimes.push((maj, min, ident.to_string()));
        }
    }
    runtimes.sort();
    let runtime_id = runtimes
        .last()
        .map(|t| t.2.clone())
        .ok_or_else(|| CliError::operational("no iOS simulator runtimes installed"))?;

    let device_type_id = pick_device_type_id(dev_dir, "iPhone 15")?;

    let device_name = "RMP iPhone 15";
    let udid = match find_simulator_udid_by_name_and_runtime(dev_dir, device_name, &runtime_id)? {
        Some(u) => u,
        None => create_simulator(dev_dir, device_name, &device_type_id, &runtime_id)?,
    };

    boot_sim(dev_dir, &udid, verbose)?;
    Ok(udid)
}

fn parse_ios_runtime_ident(ident: &str) -> Option<(u32, u32)> {
    // com.apple.CoreSimulator.SimRuntime.iOS-18-6
    let tail = ident.rsplit('.').next().unwrap_or("");
    let mut it = tail.split('-').skip_while(|s| *s != "iOS").skip(1);
    let maj = it.next()?.parse().ok()?;
    let min = it.next()?.parse().ok()?;
    Some((maj, min))
}

fn pick_device_type_id(dev_dir: &Path, prefer: &str) -> Result<String, CliError> {
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.env("DEVELOPER_DIR", dev_dir)
        .arg("simctl")
        .arg("list")
        .arg("devicetypes");
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational("failed to list sim device types"));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let mut first_iphone: Option<String> = None;
    for ln in s.lines() {
        if let Some((name, rest)) = ln.split_once('(') {
            let name = name.trim();
            let id = rest.trim_end_matches(')').trim();
            if first_iphone.is_none() && name.contains("iPhone") {
                first_iphone = Some(id.to_string());
            }
            if name.contains(prefer) {
                return Ok(id.to_string());
            }
        }
    }
    first_iphone.ok_or_else(|| CliError::operational("no iPhone simulator device types found"))
}

fn find_simulator_udid_by_name_and_runtime(
    dev_dir: &Path,
    name: &str,
    runtime_id: &str,
) -> Result<Option<String>, CliError> {
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.env("DEVELOPER_DIR", dev_dir)
        .arg("simctl")
        .arg("list")
        .arg("-j")
        .arg("devices");
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational("failed to list simulators"));
    }
    let j: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| CliError::operational(format!("failed to parse simulator JSON: {e}")))?;
    let Some(runtime_devices) = j.get("devices").and_then(|v| v.get(runtime_id)) else {
        return Ok(None);
    };
    for dev in runtime_devices.as_array().into_iter().flatten() {
        let dev_name = dev.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let udid = dev.get("udid").and_then(|v| v.as_str()).unwrap_or("");
        let available = dev
            .get("isAvailable")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if available && dev_name == name && udid.len() >= 25 {
            return Ok(Some(udid.to_string()));
        }
    }
    Ok(None)
}

fn create_simulator(
    dev_dir: &Path,
    name: &str,
    device_type_id: &str,
    runtime_id: &str,
) -> Result<String, CliError> {
    let out = Command::new("/usr/bin/xcrun")
        .env("DEVELOPER_DIR", dev_dir)
        .arg("simctl")
        .arg("create")
        .arg(name)
        .arg(device_type_id)
        .arg(runtime_id)
        .output()
        .map_err(|e| CliError::operational(format!("simctl create failed: {e}")))?;
    if !out.status.success() {
        return Err(CliError::operational("simctl create failed"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn boot_sim(dev_dir: &Path, udid: &str, verbose: bool) -> Result<(), CliError> {
    let _ = Command::new("/usr/bin/xcrun")
        .env("DEVELOPER_DIR", dev_dir)
        .arg("simctl")
        .arg("boot")
        .arg(udid)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // Avoid an unbounded `simctl bootstatus -b` wait in CI; poll with timeout.
    human_log(verbose, "waiting for simulator boot");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(180) {
        if simulator_is_booted(dev_dir, udid)? {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }

    Err(CliError::operational(format!(
        "simulator did not boot in time: {udid}"
    )))
}

fn simulator_is_booted(dev_dir: &Path, udid: &str) -> Result<bool, CliError> {
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.env("DEVELOPER_DIR", dev_dir)
        .arg("simctl")
        .arg("list")
        .arg("devices");
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational("failed to list simulators"));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        if line.contains(udid) && line.contains("(Booted)") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn maybe_write_ios_relay_config(
    dev_dir: &Path,
    udid: &str,
    bundle_id: &str,
    verbose: bool,
) -> Result<(), CliError> {
    if std::env::var("FINITECHAT_NO_RELAY_OVERRIDE")
        .ok()
        .as_deref()
        == Some("1")
    {
        human_log(
            verbose,
            "FINITECHAT_NO_RELAY_OVERRIDE=1; not writing runtime config",
        );
        return Ok(());
    }

    let relays = relay_csv_from_env();
    let kp_relays = kp_relay_csv_from_env();

    let relay_items: Vec<String> = relays
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let kp_items: Vec<String> = kp_relays
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    let json = serde_json::json!({"server_url": default_app_relay_csv(), "relay_urls": relay_items, "key_package_relay_urls": kp_items});

    // container path
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.env("DEVELOPER_DIR", dev_dir)
        .arg("simctl")
        .arg("get_app_container")
        .arg(udid)
        .arg(bundle_id)
        .arg("data");
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational(
            "failed to locate simulator app container (simctl get_app_container)",
        ));
    }
    let container = String::from_utf8_lossy(&out.stdout).replace('\r', "");
    let container = container.lines().last().unwrap_or("").trim().to_string();
    if container.is_empty() {
        return Err(CliError::operational(
            "simctl get_app_container returned empty path",
        ));
    }

    let support_dir = PathBuf::from(container).join("Library/Application Support");
    std::fs::create_dir_all(&support_dir)
        .map_err(|e| CliError::operational(format!("failed to create support dir: {e}")))?;
    let path = support_dir.join("finitechat_config.json");
    std::fs::write(&path, serde_json::to_vec(&json).unwrap())
        .map_err(|e| CliError::operational(format!("failed to write config: {e}")))?;
    human_log(
        verbose,
        format!("wrote relay override to: {}", path.to_string_lossy()),
    );
    Ok(())
}

fn run_android(
    root: &Path,
    json: bool,
    verbose: bool,
    args: crate::cli::RunAndroidArgs,
    release: bool,
) -> Result<(), CliError> {
    let cfg = load_rmp_toml(root)?;
    let android = cfg
        .android
        .ok_or_else(|| CliError::user("rmp.toml missing [android] section"))?;
    let app_id = android.app_id;
    let avd = args
        .avd
        .or(android.avd_name)
        .unwrap_or_else(|| "finitechat_api35".into());

    let serial = ensure_android_emulator(root, &avd, args.serial.as_deref(), verbose)?;
    let abi = detect_android_abi(&serial, verbose)?;
    let profile = build_profile(release);

    // Build bindings (kotlin + .so) for the connected ABI only.
    bindings::build_kotlin_for_run(root, &abi, profile, verbose)?;

    // Assemble debug APK.
    human_log(verbose, "gradle assembleDebug");
    let ci = is_ci();
    let mut cmd = Command::new("./gradlew");
    cmd.current_dir(root.join("android"))
        .arg(":app:assembleDebug");
    if ci {
        // CI stability + debuggability.
        cmd.arg("--no-daemon")
            .arg("--console=plain")
            .arg("--stacktrace");
    } else if verbose {
        cmd.arg("--console=plain");
    }
    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run gradlew: {e}")))?;
    if !status.success() {
        return Err(CliError::operational("gradle assembleDebug failed"));
    }

    let apk = root.join("android/app/build/outputs/apk/debug/app-debug.apk");
    if !apk.is_file() {
        return Err(CliError::operational(format!(
            "expected apk not found: {}",
            apk.to_string_lossy()
        )));
    }

    let pkg = app_id;

    // Install.
    human_log(verbose, format!("adb install (serial={serial})"));
    let status = Command::new("adb")
        .arg("-s")
        .arg(&serial)
        .arg("install")
        .arg("-r")
        .arg(&apk)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run adb install: {e}")))?;
    if !status.success() {
        return Err(CliError::operational("adb install failed"));
    }

    if let Some(rev) = args.adb_reverse {
        setup_adb_reverse(&serial, &rev, verbose)?;
    }

    // Optional local runtime config for emulator runs.
    if std::env::var("FINITECHAT_RELAY_URLS").is_ok()
        || std::env::var("FINITECHAT_SERVER_URL").is_ok()
    {
        maybe_write_android_relay_config(&serial, &pkg, verbose)?;
    }

    launch_android(&serial, &pkg, verbose)?;

    if json {
        json_print(&JsonOk {
            ok: true,
            data: serde_json::json!({"platform":"android","kind":"emulator","serial":serial,"app_id":pkg}),
        });
    } else {
        eprintln!("ok: android app launched");
    }
    Ok(())
}

fn run_iced(root: &Path, json: bool, verbose: bool, release: bool) -> Result<(), CliError> {
    let cfg = load_rmp_toml(root)?;
    let desktop = cfg
        .desktop
        .ok_or_else(|| CliError::user("rmp.toml missing [desktop] section"))?;

    if !desktop
        .targets
        .iter()
        .any(|t| t.eq_ignore_ascii_case("iced"))
    {
        return Err(CliError::user(
            "desktop target `iced` is not enabled in rmp.toml ([desktop].targets)",
        ));
    }

    let package = desktop
        .iced
        .and_then(|i| i.package)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| default_iced_package_name(&cfg.project.name));

    human_log(verbose, format!("cargo run -p {package}"));
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root).arg("run").arg("-p").arg(&package);
    if release {
        cmd.arg("--release");
    }
    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| CliError::operational(format!("failed to run cargo: {e}")))?;
    if !status.success() {
        return Err(CliError::operational(format!(
            "cargo run failed for desktop package `{package}`"
        )));
    }

    if json {
        json_print(&JsonOk {
            ok: true,
            data: serde_json::json!({"platform":"iced","package":package}),
        });
    } else {
        eprintln!("ok: iced app exited");
    }

    Ok(())
}

pub(crate) fn ensure_android_emulator(
    root: &Path,
    avd: &str,
    explicit_serial: Option<&str>,
    verbose: bool,
) -> Result<String, CliError> {
    let allow_headless = android_allow_headless();

    if let Some(s) = explicit_serial {
        if !adb_serial_exists(s)? {
            return Err(CliError::user(format!(
                "requested android serial not connected: {s}"
            )));
        }
        if !allow_headless && s.starts_with("emulator-") {
            let avd_name = emulator_avd_name(s).unwrap_or_else(|| avd.to_string());
            if emulator_is_headless_only(&avd_name)? {
                if avd_exists(&avd_name)? {
                    human_log(
                        verbose,
                        format!("emulator is headless (avd={avd_name}); restarting with GUI"),
                    );
                    kill_emulator(s, verbose)?;
                    return start_emulator_and_wait(root, &avd_name, verbose);
                }
                human_log(
                    verbose,
                    format!(
                        "emulator is headless (avd={avd_name}) but AVD is not available locally; keeping existing emulator"
                    ),
                );
            }
        }
        return Ok(s.to_string());
    }

    if let Some(s) = pick_any_emulator_serial()? {
        if !allow_headless {
            let avd_name = emulator_avd_name(&s).unwrap_or_else(|| avd.to_string());
            if emulator_is_headless_only(&avd_name)? {
                if avd_exists(&avd_name)? {
                    human_log(
                        verbose,
                        format!("emulator is headless (avd={avd_name}); restarting with GUI"),
                    );
                    kill_emulator(&s, verbose)?;
                    return start_emulator_and_wait(root, &avd_name, verbose);
                }
                human_log(
                    verbose,
                    format!(
                        "emulator is headless (avd={avd_name}) but AVD is not available locally; keeping existing emulator"
                    ),
                );
            }
        }
        human_log(
            verbose,
            format!("ok: android emulator already connected ({s})"),
        );
        return Ok(s);
    }

    let _ = Command::new("adb")
        .arg("start-server")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // Ensure AVD exists.
    let mut cmd = Command::new("emulator");
    cmd.arg("-list-avds");
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational(
            "failed to list AVDs (emulator -list-avds)",
        ));
    }
    let list = String::from_utf8_lossy(&out.stdout);
    if !list.lines().any(|l| l.trim() == avd) {
        return Err(CliError::user(format!(
            "android AVD not found: {avd} (create it, then re-run)"
        )));
    }

    start_emulator_and_wait(root, avd, verbose)
}

fn start_emulator_and_wait(root: &Path, avd: &str, verbose: bool) -> Result<String, CliError> {
    human_log(verbose, format!("starting android emulator: {avd}"));
    let allow_headless = android_allow_headless();
    let log_path = root.join("emulator.log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| CliError::operational(format!("failed to open emulator log: {e}")))?;
    let log2 = log
        .try_clone()
        .map_err(|e| CliError::operational(format!("failed to clone emulator log handle: {e}")))?;

    let mut child = Command::new("emulator");
    child
        .arg("-avd")
        .arg(avd)
        .arg("-no-snapshot")
        .arg("-no-audio")
        .arg("-no-boot-anim")
        .arg("-gpu")
        .arg("swiftshader_indirect");
    if allow_headless {
        child.arg("-no-window");
    }
    child.stdout(Stdio::from(log)).stderr(Stdio::from(log2));

    let _ = child
        .spawn()
        .map_err(|e| CliError::operational(format!("failed to start emulator: {e}")))?;

    // Wait for boot.
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(180) {
        if let Some(serial) = pick_any_emulator_serial()? {
            let boot = adb_shell(&serial, &["getprop", "sys.boot_completed"])?;
            if boot.trim() == "1" {
                human_log(verbose, format!("ok: android emulator booted ({serial})"));
                return Ok(serial);
            }
        }
        thread::sleep(Duration::from_secs(1));
    }

    Err(CliError::operational(
        "android emulator did not boot in time (see emulator.log)",
    ))
}

fn kill_emulator(serial: &str, verbose: bool) -> Result<(), CliError> {
    human_log(verbose, format!("killing emulator: {serial}"));
    let _ = Command::new("adb")
        .arg("-s")
        .arg(serial)
        .arg("emu")
        .arg("kill")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        if !adb_serial_exists(serial)? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(500));
    }
    Err(CliError::operational(format!(
        "timed out waiting for emulator {serial} to exit"
    )))
}

fn emulator_avd_name(serial: &str) -> Option<String> {
    let mut cmd = Command::new("adb");
    cmd.arg("-s").arg(serial).arg("emu").arg("avd").arg("name");
    let out = run_capture(cmd).ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let avd = s.lines().last()?.trim();
    if avd.is_empty() || avd.eq_ignore_ascii_case("ok") {
        return None;
    }
    Some(avd.to_string())
}

fn emulator_is_headless_only(avd: &str) -> Result<bool, CliError> {
    // Mirrors previous shell behavior:
    // headless qemu exists AND no emulator frontend process exists for this AVD.
    let mut cmd = Command::new("ps");
    cmd.arg("ax").arg("-o").arg("command=");
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational(
            "failed to inspect emulator processes",
        ));
    }

    let needle = format!("-avd {avd}");
    let mut has_headless_qemu = false;
    let mut has_frontend = false;
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        if !line.contains(&needle) {
            continue;
        }
        if line.contains("qemu-system") && line.contains("headless") {
            has_headless_qemu = true;
        }
        if line.contains("/emulator") || line.starts_with("emulator ") {
            has_frontend = true;
        }
    }
    Ok(has_headless_qemu && !has_frontend)
}

fn android_allow_headless() -> bool {
    if std::env::var("RMP_ANDROID_ALLOW_HEADLESS").ok().as_deref() == Some("1") {
        return true;
    }
    is_ci()
}

fn is_ci() -> bool {
    std::env::var("CI")
        .ok()
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

fn build_profile(release: bool) -> BuildProfile {
    if release {
        BuildProfile::Release
    } else {
        BuildProfile::Debug
    }
}

fn default_iced_package_name(project_name: &str) -> String {
    let mut out = String::new();
    for c in project_name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c.to_ascii_lowercase());
        }
    }
    if out.is_empty() {
        out.push_str("app");
    }
    format!("{out}_desktop_iced")
}

pub(crate) fn ios_sim_target_for_host() -> Result<(&'static str, &'static str), CliError> {
    match std::env::consts::ARCH {
        "aarch64" => Ok(("aarch64-apple-ios-sim", "arm64")),
        arch => Err(CliError::operational(format!(
            "unsupported host arch for iOS simulator builds: {arch}"
        ))),
    }
}

fn avd_exists(avd: &str) -> Result<bool, CliError> {
    let mut cmd = Command::new("emulator");
    cmd.arg("-list-avds");
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational(
            "failed to list AVDs (emulator -list-avds)",
        ));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    Ok(s.lines().any(|l| l.trim() == avd))
}

fn adb_serial_exists(serial: &str) -> Result<bool, CliError> {
    let mut cmd = Command::new("adb");
    cmd.arg("devices");
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational("adb devices failed"));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    Ok(s.lines()
        .skip(1)
        .any(|l| l.split_whitespace().next() == Some(serial)))
}

fn pick_any_emulator_serial() -> Result<Option<String>, CliError> {
    let mut cmd = Command::new("adb");
    cmd.arg("devices");
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational("adb devices failed"));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    for ln in s.lines().skip(1) {
        let parts: Vec<&str> = ln.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        if parts[1] != "device" {
            continue;
        }
        if parts[0].starts_with("emulator-") {
            return Ok(Some(parts[0].to_string()));
        }
    }
    Ok(None)
}

fn adb_shell(serial: &str, args: &[&str]) -> Result<String, CliError> {
    let mut cmd = Command::new("adb");
    cmd.arg("-s").arg(serial).arg("shell").args(args);
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational("adb shell failed"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).replace('\r', ""))
}

fn detect_android_abi(serial: &str, verbose: bool) -> Result<String, CliError> {
    let raw = adb_shell(serial, &["getprop", "ro.product.cpu.abi"])?;
    let raw = raw.lines().next().unwrap_or("").trim();
    if raw.is_empty() {
        return Err(CliError::operational(
            "failed to detect Android ABI (getprop ro.product.cpu.abi returned empty)",
        ));
    }
    let abi = normalize_android_abi(raw).unwrap_or(raw).to_string();
    human_log(
        verbose,
        format!("android ABI detected: {raw} (using {abi})"),
    );
    Ok(abi)
}

fn normalize_android_abi(raw: &str) -> Option<&'static str> {
    match raw {
        "arm64-v8a" | "aarch64" => Some("arm64-v8a"),
        "armeabi-v7a" | "armeabi" => Some("armeabi-v7a"),
        "x86_64" => Some("x86_64"),
        "x86" => Some("x86"),
        _ => None,
    }
}

fn setup_adb_reverse(serial: &str, spec: &str, verbose: bool) -> Result<(), CliError> {
    for item in spec.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let (dev_port, host_port) = if let Some((a, b)) = item.split_once(':') {
            (a.trim(), b.trim())
        } else {
            (item, item)
        };
        human_log(
            verbose,
            format!("adb reverse tcp:{dev_port} -> tcp:{host_port}"),
        );
        let status = Command::new("adb")
            .arg("-s")
            .arg(serial)
            .arg("reverse")
            .arg(format!("tcp:{dev_port}"))
            .arg(format!("tcp:{host_port}"))
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| CliError::operational(format!("failed to run adb reverse: {e}")))?;
        if !status.success() {
            return Err(CliError::operational("adb reverse failed"));
        }
    }
    Ok(())
}

fn maybe_write_android_relay_config(
    serial: &str,
    pkg: &str,
    verbose: bool,
) -> Result<(), CliError> {
    if std::env::var("FINITECHAT_NO_RELAY_OVERRIDE")
        .ok()
        .as_deref()
        == Some("1")
    {
        human_log(
            verbose,
            "FINITECHAT_NO_RELAY_OVERRIDE=1; not writing runtime config",
        );
        return Ok(());
    }

    let relays = relay_csv_from_env();
    let kp_relays = kp_relay_csv_from_env();

    let relay_items: Vec<String> = relays
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let kp_items: Vec<String> = kp_relays
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let json = serde_json::json!({"server_url": default_app_relay_csv(), "relay_urls": relay_items, "key_package_relay_urls": kp_items});
    let json_s = serde_json::to_string(&json).unwrap();

    human_log(verbose, "writing runtime config (finitechat_config.json)");
    let _ = Command::new("adb")
        .arg("-s")
        .arg(serial)
        .arg("shell")
        .arg("am")
        .arg("force-stop")
        .arg(pkg)
        .status();

    let mut child = Command::new("adb")
        .arg("-s")
        .arg(serial)
        .arg("shell")
        // NOTE: `adb shell` concatenates argv into a single string; without careful quoting,
        // `sh -c ...` will receive the wrong argv and do the wrong thing.
        .arg(format!(
            "run-as {pkg} sh -c 'mkdir -p files && cat > files/finitechat_config.json'"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| CliError::operational(format!("failed to run adb run-as: {e}")))?;
    {
        use std::io::Write;
        let Some(mut stdin) = child.stdin.take() else {
            return Err(CliError::operational("failed to open stdin for adb run-as"));
        };
        stdin
            .write_all(json_s.as_bytes())
            .map_err(|e| CliError::operational(format!("failed to write config: {e}")))?;
    }
    let status = child
        .wait()
        .map_err(|e| CliError::operational(format!("failed to wait for adb: {e}")))?;
    if !status.success() {
        return Err(CliError::operational(
            "could not write app config via run-as (is this a debuggable build?)",
        ));
    }
    Ok(())
}

fn launch_android(serial: &str, pkg: &str, verbose: bool) -> Result<(), CliError> {
    human_log(verbose, format!("launching {pkg}"));

    let resolved = adb_shell(
        serial,
        &[
            "cmd",
            "package",
            "resolve-activity",
            "--brief",
            "-a",
            "android.intent.action.MAIN",
            "-c",
            "android.intent.category.LAUNCHER",
            pkg,
        ],
    )
    .unwrap_or_default();
    let resolved = resolved.lines().last().unwrap_or("").trim().to_string();

    if resolved.contains('/') {
        let _ = Command::new("adb")
            .arg("-s")
            .arg(serial)
            .arg("shell")
            .arg("am")
            .arg("start")
            .arg("-W")
            .arg("-n")
            .arg(resolved)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| CliError::operational(format!("failed to run adb am start: {e}")))?;
    } else {
        let _ = Command::new("adb")
            .arg("-s")
            .arg(serial)
            .arg("shell")
            .arg("am")
            .arg("start")
            .arg("-W")
            .arg("-a")
            .arg("android.intent.action.MAIN")
            .arg("-c")
            .arg("android.intent.category.LAUNCHER")
            .arg("-n")
            .arg(format!("{pkg}/.MainActivity"))
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| CliError::operational(format!("failed to run adb am start: {e}")))?;
    }

    for _ in 0..20 {
        let out = Command::new("adb")
            .arg("-s")
            .arg(serial)
            .arg("shell")
            .arg("pidof")
            .arg(pkg)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        if let Ok(out) = out
            && out.status.success()
            && !String::from_utf8_lossy(&out.stdout).trim().is_empty()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(500));
    }

    Err(CliError::operational(format!(
        "app did not appear to start (pidof {pkg} empty)"
    )))
}

fn resolve_ios_app_path(
    dev_dir: &Path,
    xcode_project_path: &Path,
    xcode_scheme: &str,
    udid: &str,
    xcode_config: &str,
    sdk: &str,
    xcode_arch: &str,
) -> Result<PathBuf, CliError> {
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.env("DEVELOPER_DIR", dev_dir)
        .env_remove("LD")
        .env_remove("CC")
        .env_remove("CXX")
        .arg("xcodebuild")
        .arg("-project")
        .arg(xcode_project_path)
        .arg("-scheme")
        .arg(xcode_scheme)
        .arg("-destination")
        .arg(format!("id={udid}"))
        .arg("-configuration")
        .arg(xcode_config)
        .arg("-sdk")
        .arg(sdk)
        .arg(format!("ARCHS={xcode_arch}"))
        .arg("ONLY_ACTIVE_ARCH=YES")
        .arg("-showBuildSettings");
    let out = run_capture(cmd)?;
    if !out.status.success() {
        return Err(CliError::operational(
            "xcodebuild -showBuildSettings failed",
        ));
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut target_build_dir: Option<String> = None;
    let mut full_product_name: Option<String> = None;
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("TARGET_BUILD_DIR = ") {
            target_build_dir = Some(v.trim().to_string());
            continue;
        }
        if let Some(v) = line.strip_prefix("FULL_PRODUCT_NAME = ") {
            full_product_name = Some(v.trim().to_string());
        }
    }

    let target_build_dir = target_build_dir.ok_or_else(|| {
        CliError::operational("xcodebuild -showBuildSettings missing TARGET_BUILD_DIR")
    })?;
    let full_product_name = full_product_name.ok_or_else(|| {
        CliError::operational("xcodebuild -showBuildSettings missing FULL_PRODUCT_NAME")
    })?;

    Ok(PathBuf::from(target_build_dir).join(full_product_name))
}

/// Read the `name:` field from `ios/project.yml` to derive the Xcode project/target/app name.
pub(crate) fn read_xcode_project_name(root: &Path) -> Option<String> {
    let yml_path = root.join("ios/project.yml");
    let content = std::fs::read_to_string(&yml_path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("name:") {
            let name = trimmed.strip_prefix("name:")?.trim();
            // Strip optional quotes.
            let name = name.trim_matches('"').trim_matches('\'').trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_default_csv_helpers_are_empty_without_env() {
        assert_eq!(default_app_relay_csv(), "");
        assert_eq!(default_app_kp_relay_csv(), "");
    }

    #[test]
    fn csv_override_prefers_primary_then_secondary_then_default() {
        let from_primary = csv_override_from_env_with(
            |key| match key {
                "A" => Some("wss://primary.example".to_string()),
                _ => None,
            },
            "A",
            "B",
            default_app_relay_csv,
        );
        assert_eq!(from_primary, "wss://primary.example");

        let from_secondary = csv_override_from_env_with(
            |key| match key {
                "B" => Some("wss://secondary.example".to_string()),
                _ => None,
            },
            "A",
            "B",
            default_app_relay_csv,
        );
        assert_eq!(from_secondary, "wss://secondary.example");

        let from_default = csv_override_from_env_with(|_| None, "A", "B", default_app_relay_csv);
        assert_eq!(from_default, default_app_relay_csv());
    }

    #[test]
    fn csv_override_ignores_empty_values_and_uses_default() {
        let from_empty_primary = csv_override_from_env_with(
            |key| match key {
                "A" => Some("   ".to_string()),
                _ => None,
            },
            "A",
            "B",
            default_app_kp_relay_csv,
        );
        assert_eq!(from_empty_primary, default_app_kp_relay_csv());
    }

    #[test]
    fn ios_development_team_resolution_prefers_explicit_then_env() {
        assert_eq!(
            resolve_ios_development_team_with(
                Some("  TEAM123  ".to_owned()),
                Some("ENV999".to_owned()),
            )
            .expect("explicit team"),
            Some("TEAM123".to_owned())
        );
        assert_eq!(
            resolve_ios_development_team_with(None, Some("  ENV999  ".to_owned()))
                .expect("env team"),
            Some("ENV999".to_owned())
        );
        assert_eq!(
            resolve_ios_development_team_with(None, None).expect("missing team"),
            None
        );
    }

    #[test]
    fn ios_development_team_resolution_rejects_empty_or_non_alphanumeric() {
        let empty_error = resolve_ios_development_team_with(Some("   ".to_owned()), None)
            .expect_err("empty team should fail");
        assert!(
            empty_error.to_string().contains("must not be empty"),
            "unexpected error: {empty_error}"
        );

        let punctuation_error =
            resolve_ios_development_team_with(None, Some("TEAM-123".to_owned()))
                .expect_err("punctuation should fail");
        assert!(
            punctuation_error
                .to_string()
                .contains("only ASCII letters and digits"),
            "unexpected error: {punctuation_error}"
        );
    }

    #[test]
    fn required_ios_development_team_uses_injected_env_for_harness_preflight() {
        assert_eq!(
            require_ios_development_team_with(None, Some("  ENV999  ".to_owned()))
                .expect("env team"),
            "ENV999"
        );
        let missing =
            require_ios_development_team_with(None, None).expect_err("missing team should fail");
        assert!(
            missing
                .to_string()
                .contains("requires --ios-development-team"),
            "unexpected error: {missing}"
        );
    }

    #[test]
    fn classify_ios_udid_routes_simulator_and_device() {
        use std::collections::HashSet;

        let mut sims = HashSet::new();
        sims.insert("8C10824B-840E-5717-BC9C-55B537D33060".to_string());

        // A UDID present in the queried simulator set is a simulator.
        assert_eq!(
            classify_ios_udid("8C10824B-840E-5717-BC9C-55B537D33060", &sims),
            IosTargetKind::Simulator
        );
        // Paulphone Air's hardware UDID (single dash, not in the set) is a device.
        assert_eq!(
            classify_ios_udid("00008150-0010149A26F0401C", &sims),
            IosTargetKind::PhysicalDevice
        );

        // Fallback heuristic when the simctl query failed (empty set): a
        // UUID-shaped id is still treated as a simulator, a hardware-shaped id as
        // a device.
        let empty = HashSet::new();
        assert_eq!(
            classify_ios_udid("8C10824B-840E-5717-BC9C-55B537D33060", &empty),
            IosTargetKind::Simulator
        );
        assert_eq!(
            classify_ios_udid("00008150-0010149A26F0401C", &empty),
            IosTargetKind::PhysicalDevice
        );
    }

    #[test]
    fn simulator_udids_are_collected_across_runtimes() {
        let value = serde_json::json!({
            "devices": {
                "com.apple.CoreSimulator.SimRuntime.iOS-18-6": [
                    {"udid": "8C10824B-840E-5717-BC9C-55B537D33060", "name": "iPhone 15"}
                ],
                "com.apple.CoreSimulator.SimRuntime.iOS-17-5": [
                    {"udid": "11111111-2222-3333-4444-555555555555", "name": "iPhone 14"}
                ]
            }
        });
        let udids = simulator_udids_from_value(&value);
        assert!(udids.contains("8C10824B-840E-5717-BC9C-55B537D33060"));
        assert!(udids.contains("11111111-2222-3333-4444-555555555555"));
        assert!(!udids.contains("00008150-0010149A26F0401C"));
    }

    #[test]
    fn devicectl_identifier_resolves_to_xcode_hardware_udid() {
        let value = serde_json::json!({
            "result": {
                "devices": [
                    {
                        "identifier": "8C10824B-840E-5717-BC9C-55B537D33060",
                        "hardwareProperties": {
                            "udid": "00008150-0010149A26F0401C"
                        }
                    }
                ]
            }
        });

        assert_eq!(
            resolve_ios_device_udid_from_devicectl_value(
                &value,
                "8C10824B-840E-5717-BC9C-55B537D33060",
            ),
            Some("00008150-0010149A26F0401C".to_owned())
        );
        assert_eq!(
            resolve_ios_device_udid_from_devicectl_value(&value, "00008150-0010149A26F0401C"),
            Some("00008150-0010149A26F0401C".to_owned())
        );
        assert_eq!(
            resolve_ios_device_udid_from_devicectl_value(&value, "missing-device"),
            None
        );
    }

    #[test]
    fn devicectl_process_identifier_is_found_recursively() {
        let value = serde_json::json!({
            "result": {
                "launch": {
                    "process": {
                        "processIdentifier": "4242"
                    }
                }
            }
        });

        assert_eq!(find_process_identifier(&value), Some(4242));
    }

    #[test]
    fn ios_device_store_path_must_end_with_finitechat_store() {
        checked_ios_device_store_path(Path::new("/tmp/harness/FiniteChatStore"))
            .expect("valid store path");

        let error = checked_ios_device_store_path(Path::new("/tmp/harness/store"))
            .expect_err("wrong store directory should be rejected");

        assert!(
            error
                .to_string()
                .contains("device store path must end with FiniteChatStore"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn pulled_ios_device_store_source_requires_exact_store_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pull_parent = temp.path().join("pull");
        std::fs::create_dir_all(pull_parent.join(IOS_DEVICE_STORE_DIR)).expect("store dir");

        assert_eq!(
            pulled_ios_device_store_source(&pull_parent).expect("store source"),
            pull_parent.join(IOS_DEVICE_STORE_DIR)
        );
    }

    #[test]
    fn pulled_ios_device_store_source_accepts_direct_store_contents() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pull_parent = temp.path().join("pull");
        std::fs::create_dir_all(&pull_parent).expect("pull dir");
        std::fs::write(pull_parent.join("client.sqlite3"), "").expect("sqlite");

        assert_eq!(
            pulled_ios_device_store_source(&pull_parent).expect("direct store source"),
            pull_parent
        );
    }

    #[test]
    fn pulled_ios_device_store_source_rejects_ambiguous_copy_shape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pull_parent = temp.path().join("pull");
        std::fs::create_dir_all(pull_parent.join("client.sqlite3")).expect("ambiguous dir");

        let error = pulled_ios_device_store_source(&pull_parent)
            .expect_err("ambiguous copy shape should be rejected");

        assert!(
            error.to_string().contains(
                "did not produce expected FiniteChatStore directory or direct store contents"
            ),
            "unexpected error: {error}"
        );
    }
}
