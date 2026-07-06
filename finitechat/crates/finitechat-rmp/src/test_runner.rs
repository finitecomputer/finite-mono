use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::bindings::{self, BuildProfile};
use crate::cli::{
    CliError, JsonOk, TestArgs, TestIosSimulatorArgs, TestPlatform, human_log, json_print,
};
use crate::config::load_rmp_toml;
use crate::run::{
    ensure_ios_simulator, ios_sim_target_for_host, read_xcode_project_name,
    terminate_ios_simulator_app,
};
use crate::util::{discover_xcode_dev_dir, run_capture};

pub fn test(root: &Path, json: bool, verbose: bool, args: TestArgs) -> Result<(), CliError> {
    match args.platform {
        TestPlatform::IosSimulator => test_ios_simulator(root, json, verbose, args.ios_simulator),
    }
}

#[derive(Serialize)]
struct IosSimulatorTestJson {
    platform: &'static str,
    udid: String,
    bundle_id: String,
    derived_data_path: String,
    result_bundle_path: String,
}

struct SimulatorCleanup {
    dev_dir: PathBuf,
    udid: String,
    bundle_id: String,
    verbose: bool,
    armed: bool,
}

impl SimulatorCleanup {
    fn new(dev_dir: PathBuf, udid: String, bundle_id: String, verbose: bool) -> Self {
        Self {
            dev_dir,
            udid,
            bundle_id,
            verbose,
            armed: true,
        }
    }

    fn cleanup_now(&mut self) {
        if !self.armed {
            return;
        }
        let _ =
            terminate_ios_simulator_app(&self.dev_dir, &self.udid, &self.bundle_id, self.verbose);
        let _ = shutdown_ios_simulator(&self.dev_dir, &self.udid, self.verbose);
        self.armed = false;
    }
}

impl Drop for SimulatorCleanup {
    fn drop(&mut self) {
        self.cleanup_now();
    }
}

fn test_ios_simulator(
    root: &Path,
    json: bool,
    verbose: bool,
    args: TestIosSimulatorArgs,
) -> Result<(), CliError> {
    let cfg = load_rmp_toml(root)?;
    let ios = cfg
        .ios
        .ok_or_else(|| CliError::user("rmp.toml missing [ios] section"))?;
    let bundle_id = ios.bundle_id;
    let xcode_name = read_xcode_project_name(root).unwrap_or_else(|| "App".to_owned());
    let xcode_scheme = ios.scheme.unwrap_or_else(|| xcode_name.clone());
    let xcode_project_path = root.join(format!("ios/{xcode_name}.xcodeproj"));
    let derived_data_path = workspace_path(root, &args.derived_data_path);
    let result_bundle_path = workspace_path(root, &args.result_bundle_path);
    let dev_dir = discover_xcode_dev_dir()?;
    let (rust_target, xcode_arch) = ios_sim_target_for_host()?;

    let udid = ensure_ios_simulator(&dev_dir, args.udid.as_deref(), verbose)?;
    let mut cleanup =
        SimulatorCleanup::new(dev_dir.clone(), udid.clone(), bundle_id.clone(), verbose);

    shutdown_ios_simulator(&dev_dir, &udid, verbose)?;
    erase_ios_simulator(&dev_dir, &udid, json, verbose)?;
    ensure_ios_simulator(&dev_dir, Some(&udid), verbose)?;

    if json {
        bindings::build_swift_for_test(root, rust_target, BuildProfile::Debug, verbose)?;
    } else {
        bindings::build_swift_for_run(root, rust_target, BuildProfile::Debug, verbose)?;
    }
    run_xcodegen(root, json, verbose)?;
    prepare_result_bundle_path(&result_bundle_path)?;

    if let Some(parent) = derived_data_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CliError::operational(format!(
                "failed to create derived-data parent {}: {error}",
                parent.display()
            ))
        })?;
    }

    human_log(
        verbose,
        format!("xcodebuild test ({xcode_scheme}, simulator={udid}, arch={xcode_arch})"),
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
        .arg("-configuration")
        .arg("Debug")
        .arg("-sdk")
        .arg("iphonesimulator")
        .arg("-destination")
        .arg(format!("id={udid}"))
        .arg("-derivedDataPath")
        .arg(&derived_data_path)
        .arg("-resultBundlePath")
        .arg(&result_bundle_path)
        .arg("-parallel-testing-enabled")
        .arg("NO")
        .arg("test")
        .arg(format!("ARCHS={xcode_arch}"))
        .arg("ONLY_ACTIVE_ARCH=YES")
        .arg("CODE_SIGNING_ALLOWED=NO")
        .arg(format!("PRODUCT_BUNDLE_IDENTIFIER={bundle_id}"));

    let status = run_logged_status(cmd, json, "failed to run xcodebuild test")?;
    if !status.success() {
        eprintln!(
            "xcodebuild test failed; result bundle: {}",
            result_bundle_path.display()
        );
        return Err(CliError::operational("xcodebuild test failed").with_detail(
            "result_bundle_path",
            serde_json::json!(result_bundle_path.to_string_lossy()),
        ));
    }

    cleanup.cleanup_now();

    let result = IosSimulatorTestJson {
        platform: "ios-simulator",
        udid,
        bundle_id,
        derived_data_path: derived_data_path.to_string_lossy().into_owned(),
        result_bundle_path: result_bundle_path.to_string_lossy().into_owned(),
    };
    if json {
        json_print(&JsonOk {
            ok: true,
            data: result,
        });
    } else {
        eprintln!(
            "ok: iOS simulator tests passed (udid={}, result={})",
            result.udid, result.result_bundle_path
        );
    }
    Ok(())
}

fn workspace_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    }
}

fn run_xcodegen(root: &Path, json: bool, verbose: bool) -> Result<(), CliError> {
    human_log(verbose, "xcodegen generate");
    let mut cmd = Command::new("xcodegen");
    cmd.current_dir(root.join("ios")).arg("generate");
    let status = run_logged_status(cmd, json, "failed to run xcodegen")?;
    if !status.success() {
        return Err(CliError::operational("xcodegen generate failed"));
    }
    Ok(())
}

fn run_logged_status(
    mut cmd: Command,
    stdout_to_stderr: bool,
    spawn_error_prefix: &str,
) -> Result<ExitStatus, CliError> {
    if stdout_to_stderr {
        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|error| CliError::operational(format!("{spawn_error_prefix}: {error}")))?;
        let mut stderr = std::io::stderr().lock();
        stderr.write_all(&output.stdout).map_err(|error| {
            CliError::operational(format!(
                "failed to write captured stdout to stderr: {error}"
            ))
        })?;
        stderr.write_all(&output.stderr).map_err(|error| {
            CliError::operational(format!("failed to write captured stderr: {error}"))
        })?;
        return Ok(output.status);
    }

    cmd.stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| CliError::operational(format!("{spawn_error_prefix}: {error}")))
}

fn prepare_result_bundle_path(path: &Path) -> Result<(), CliError> {
    if path.exists() {
        if path.is_dir() {
            fs::remove_dir_all(path).map_err(|error| {
                CliError::operational(format!(
                    "failed to remove existing result bundle {}: {error}",
                    path.display()
                ))
            })?;
        } else {
            fs::remove_file(path).map_err(|error| {
                CliError::operational(format!(
                    "failed to remove existing result path {}: {error}",
                    path.display()
                ))
            })?;
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CliError::operational(format!(
                "failed to create result bundle parent {}: {error}",
                parent.display()
            ))
        })?;
    }
    Ok(())
}

fn shutdown_ios_simulator(dev_dir: &Path, udid: &str, verbose: bool) -> Result<(), CliError> {
    human_log(verbose, format!("simctl shutdown (udid={udid})"));
    let _ = Command::new("/usr/bin/xcrun")
        .env("DEVELOPER_DIR", dev_dir)
        .arg("simctl")
        .arg("shutdown")
        .arg(udid)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    wait_for_simulator_state(dev_dir, udid, "Shutdown", Duration::from_secs(60))
}

fn erase_ios_simulator(
    dev_dir: &Path,
    udid: &str,
    stdout_to_stderr: bool,
    verbose: bool,
) -> Result<(), CliError> {
    human_log(verbose, format!("simctl erase (udid={udid})"));
    let mut cmd = Command::new("/usr/bin/xcrun");
    cmd.env("DEVELOPER_DIR", dev_dir)
        .arg("simctl")
        .arg("erase")
        .arg(udid);
    let status = run_logged_status(cmd, stdout_to_stderr, "failed to run simctl erase")?;
    if !status.success() {
        return Err(CliError::operational("simctl erase failed"));
    }
    Ok(())
}

fn wait_for_simulator_state(
    dev_dir: &Path,
    udid: &str,
    expected_state: &str,
    timeout: Duration,
) -> Result<(), CliError> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        match simulator_state(dev_dir, udid)? {
            Some(state) if state == expected_state => return Ok(()),
            Some(_) | None => thread::sleep(Duration::from_millis(500)),
        }
    }
    Err(CliError::operational(format!(
        "simulator {udid} did not reach {expected_state} within {} seconds",
        timeout.as_secs()
    )))
}

fn simulator_state(dev_dir: &Path, udid: &str) -> Result<Option<String>, CliError> {
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
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|error| {
        CliError::operational(format!("failed to parse simulator JSON: {error}"))
    })?;
    let devices = value
        .get("devices")
        .and_then(|devices| devices.as_object())
        .into_iter()
        .flat_map(|runtimes| runtimes.values())
        .flat_map(|devices| devices.as_array().into_iter().flatten());
    for device in devices {
        if device.get("udid").and_then(|value| value.as_str()) == Some(udid) {
            return Ok(device
                .get("state")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned));
        }
    }
    Ok(None)
}
