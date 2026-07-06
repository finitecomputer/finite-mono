use std::path::{Component, Path, PathBuf};

use crate::cli::{
    CliError, JsonOk, ResetProductStoreArgs, ResetProductStorePlatform, human_log, json_print,
};

pub(crate) const PRODUCT_HARNESS_ROOT: &str = ".state/product-harness";

#[derive(Debug)]
pub(crate) struct ResetProductStoreResult {
    pub platform: String,
    pub scenario: String,
    pub device: String,
    pub root: PathBuf,
    pub deleted: bool,
}

pub fn reset_product_store(
    root: &Path,
    json: bool,
    verbose: bool,
    args: ResetProductStoreArgs,
) -> Result<(), CliError> {
    let platform = match args.platform {
        ResetProductStorePlatform::Ios => "ios",
    };
    let result = reset_product_store_root(root, platform, &args.scenario, &args.device, verbose)?;

    if json {
        json_print(&JsonOk {
            ok: true,
            data: serde_json::json!({
                "platform": result.platform,
                "scenario": result.scenario,
                "device": result.device,
                "root": result.root.display().to_string(),
                "deleted": result.deleted,
            }),
        });
    } else {
        eprintln!(
            "ok: reset product store root {}{}",
            result.root.display(),
            if result.deleted {
                ""
            } else {
                " (did not exist)"
            }
        );
    }
    Ok(())
}

pub(crate) fn reset_product_store_root(
    root: &Path,
    platform: &str,
    scenario: &str,
    device: &str,
    verbose: bool,
) -> Result<ResetProductStoreResult, CliError> {
    let scenario = checked_path_component("scenario", scenario)?;
    let device = checked_path_component("device", device)?;
    let target = product_store_root(root, platform, &scenario, &device)?;

    let deleted = target.exists();
    if deleted {
        std::fs::remove_dir_all(&target).map_err(|error| {
            CliError::operational(format!(
                "failed to delete product store root {}: {error}",
                target.display()
            ))
        })?;
    }

    human_log(
        verbose,
        format!("reset product store root: {}", target.display()),
    );
    Ok(ResetProductStoreResult {
        platform: platform.to_owned(),
        scenario,
        device,
        root: target,
        deleted,
    })
}

pub(crate) fn product_store_root(
    root: &Path,
    platform: &str,
    scenario: &str,
    device: &str,
) -> Result<PathBuf, CliError> {
    resolve_product_store_root(
        root,
        platform,
        scenario,
        device,
        ProductStoreRootMode::CreateHarnessRoot,
    )
}

pub(crate) fn product_store_root_dry_run(
    root: &Path,
    platform: &str,
    scenario: &str,
    device: &str,
) -> Result<PathBuf, CliError> {
    resolve_product_store_root(
        root,
        platform,
        scenario,
        device,
        ProductStoreRootMode::ResolveOnly,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProductStoreRootMode {
    CreateHarnessRoot,
    ResolveOnly,
}

fn resolve_product_store_root(
    root: &Path,
    platform: &str,
    scenario: &str,
    device: &str,
    mode: ProductStoreRootMode,
) -> Result<PathBuf, CliError> {
    let scenario = checked_path_component("scenario", scenario)?;
    let device = checked_path_component("device", device)?;
    let platform = checked_path_component("platform", platform)?;
    let workspace_root = root.canonicalize().map_err(|error| {
        CliError::operational(format!(
            "failed to resolve workspace root {}: {error}",
            root.display()
        ))
    })?;
    let state_root = workspace_root.join(".state");
    let product_harness_root = workspace_root.join(PRODUCT_HARNESS_ROOT);
    let harness_root = product_harness_root.join(platform);

    reject_existing_symlink(&state_root, "product harness .state root")?;
    reject_existing_symlink(&product_harness_root, "product harness root")?;
    reject_existing_symlink(&harness_root, "platform product harness root")?;
    if mode == ProductStoreRootMode::CreateHarnessRoot {
        std::fs::create_dir_all(&harness_root).map_err(|error| {
            CliError::operational(format!(
                "failed to create product harness root {}: {error}",
                harness_root.display()
            ))
        })?;
    }
    let canonical_harness_root = if harness_root.exists() {
        harness_root.canonicalize().map_err(|error| {
            CliError::operational(format!(
                "failed to resolve product harness root {}: {error}",
                harness_root.display()
            ))
        })?
    } else {
        harness_root
    };
    if !canonical_harness_root.starts_with(&workspace_root) {
        return Err(CliError::user(format!(
            "refusing to use product harness root outside workspace: {}",
            canonical_harness_root.display()
        )));
    }
    let scenario_root = canonical_harness_root.join(&scenario);
    reject_existing_symlink(&scenario_root, "scenario product harness root")?;
    let target = scenario_root.join(&device);
    reject_existing_symlink(&target, "device product store root")?;
    if !target.starts_with(&canonical_harness_root) {
        return Err(CliError::user(format!(
            "refusing to reset product store outside harness root: {}",
            target.display()
        )));
    }
    if target == default_macos_application_support()? {
        return Err(CliError::user(
            "refusing to reset the default product Application Support path",
        ));
    }

    let deleted = target.exists();
    if deleted {
        let resolved_target = target.canonicalize().map_err(|error| {
            CliError::operational(format!(
                "failed to resolve product store target {}: {error}",
                target.display()
            ))
        })?;
        if !resolved_target.starts_with(&canonical_harness_root) {
            return Err(CliError::user(format!(
                "refusing to reset resolved product store outside harness root: {}",
                resolved_target.display()
            )));
        }
        if resolved_target == default_macos_application_support()? {
            return Err(CliError::user(
                "refusing to reset the default product Application Support path",
            ));
        }
    }
    Ok(target)
}

fn reject_existing_symlink(path: &Path, label: &str) -> Result<(), CliError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(CliError::user(format!(
            "refusing to use symlinked {label}: {}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliError::operational(format!(
            "failed to inspect {label} {}: {error}",
            path.display()
        ))),
    }
}

pub(crate) fn checked_path_component(field: &str, value: &str) -> Result<String, CliError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CliError::user(format!("{field} must not be empty")));
    }
    let path = Path::new(trimmed);
    if path.components().count() != 1 {
        return Err(CliError::user(format!(
            "{field} must be a single path component"
        )));
    }
    match path.components().next() {
        Some(Component::Normal(_)) => Ok(trimmed.to_owned()),
        _ => Err(CliError::user(format!(
            "{field} must be a normal path component"
        ))),
    }
}

fn default_macos_application_support() -> Result<PathBuf, CliError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| CliError::operational("HOME is not set"))?;
    Ok(home.join("Library/Application Support"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_path_component_rejects_path_escapes() {
        for value in ["", "   ", ".", "..", "a/b", "/tmp/store"] {
            assert!(
                checked_path_component("scenario", value).is_err(),
                "{value:?} should be rejected"
            );
        }

        assert_eq!(
            checked_path_component("scenario", " text-offline ").expect("component"),
            "text-offline"
        );
    }

    #[test]
    fn reset_product_store_deletes_only_the_explicit_device_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target =
            product_store_root(temp.path(), "ios", "text-offline", "sim-a").expect("target root");
        let sibling =
            product_store_root(temp.path(), "ios", "text-offline", "sim-b").expect("sibling root");
        std::fs::create_dir_all(target.join("FiniteChatStore")).expect("target store");
        std::fs::write(target.join("FiniteChatStore/client.sqlite3"), b"db").expect("target file");
        std::fs::create_dir_all(&sibling).expect("sibling store");
        std::fs::write(sibling.join("keep.txt"), b"keep").expect("sibling file");

        let result = reset_product_store_root(temp.path(), "ios", "text-offline", "sim-a", false)
            .expect("reset");

        assert!(result.deleted);
        assert!(!target.exists());
        assert!(sibling.join("keep.txt").exists());
    }

    #[test]
    fn dry_run_product_store_root_does_not_create_harness_directories() {
        let temp = tempfile::tempdir().expect("tempdir");

        let target = product_store_root_dry_run(temp.path(), "ios", "text-offline", "sim-a")
            .expect("dry-run target root");

        assert_eq!(
            target,
            temp.path()
                .canonicalize()
                .expect("canonical tempdir")
                .join(PRODUCT_HARNESS_ROOT)
                .join("ios")
                .join("text-offline")
                .join("sim-a")
        );
        assert!(!temp.path().join(".state").exists());
    }

    #[test]
    fn dry_run_product_store_root_still_rejects_existing_symlinked_ancestor() {
        let temp = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside");
        let state = temp.path().join(".state");
        std::fs::create_dir_all(&state).expect("state");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), state.join("product-harness"))
                .expect("symlink");

            let error = product_store_root_dry_run(temp.path(), "ios", "text-offline", "sim-a")
                .expect_err("dry-run should reject symlinked harness root");

            assert!(
                error.to_string().contains("symlinked product harness root"),
                "unexpected error: {error}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn dry_run_product_store_root_rejects_symlinked_state_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside");
        std::os::unix::fs::symlink(outside.path(), temp.path().join(".state"))
            .expect("state symlink");

        let error = product_store_root_dry_run(temp.path(), "ios", "text-offline", "sim-a")
            .expect_err("dry-run should reject symlinked .state root");

        assert!(
            error
                .to_string()
                .contains("symlinked product harness .state root"),
            "unexpected error: {error}"
        );
        assert!(outside.path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn product_store_root_rejects_symlinked_harness_ancestor() {
        let temp = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside");
        let state = temp.path().join(".state");
        std::fs::create_dir_all(&state).expect("state");
        std::os::unix::fs::symlink(outside.path(), state.join("product-harness")).expect("symlink");

        let error = product_store_root(temp.path(), "ios", "text-offline", "sim-a")
            .expect_err("symlinked harness root should be rejected");

        assert!(
            error.to_string().contains("symlinked product harness root"),
            "unexpected error: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn product_store_root_rejects_symlinked_platform_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside");
        let harness_root = temp.path().join(PRODUCT_HARNESS_ROOT);
        std::fs::create_dir_all(&harness_root).expect("harness root");
        std::os::unix::fs::symlink(outside.path(), harness_root.join("ios"))
            .expect("platform symlink");

        let error = product_store_root(temp.path(), "ios", "text-offline", "sim-a")
            .expect_err("symlinked platform root should be rejected");

        assert!(
            error
                .to_string()
                .contains("symlinked platform product harness root"),
            "unexpected error: {error}"
        );
        assert!(outside.path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn dry_run_product_store_root_rejects_symlinked_scenario_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside");
        let harness_root = temp.path().join(PRODUCT_HARNESS_ROOT).join("ios");
        std::fs::create_dir_all(&harness_root).expect("harness root");
        std::os::unix::fs::symlink(outside.path(), harness_root.join("text-offline"))
            .expect("scenario symlink");

        let error = product_store_root_dry_run(temp.path(), "ios", "text-offline", "sim-a")
            .expect_err("symlinked scenario root should be rejected");

        assert!(
            error
                .to_string()
                .contains("symlinked scenario product harness root"),
            "unexpected error: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reset_product_store_rejects_symlinked_device_root_even_inside_harness() {
        let temp = tempfile::tempdir().expect("tempdir");
        let harness_root = temp.path().join(PRODUCT_HARNESS_ROOT).join("ios");
        let scenario_root = harness_root.join("text-offline");
        let real_store = harness_root.join("real-store");
        std::fs::create_dir_all(&scenario_root).expect("scenario root");
        std::fs::create_dir_all(&real_store).expect("real store");
        std::os::unix::fs::symlink(&real_store, scenario_root.join("sim-a"))
            .expect("device symlink");

        let error = reset_product_store_root(temp.path(), "ios", "text-offline", "sim-a", false)
            .expect_err("symlinked device root should be rejected");

        assert!(
            error
                .to_string()
                .contains("symlinked device product store root"),
            "unexpected error: {error}"
        );
        assert!(real_store.exists());
    }

    #[cfg(unix)]
    #[test]
    fn reset_product_store_rejects_symlinked_device_root_escape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside");
        let harness_root = temp.path().join(PRODUCT_HARNESS_ROOT).join("ios");
        std::fs::create_dir_all(harness_root.join("text-offline")).expect("scenario root");
        let target = harness_root.join("text-offline").join("sim-a");
        std::os::unix::fs::symlink(outside.path(), &target).expect("device symlink");

        let error = reset_product_store_root(temp.path(), "ios", "text-offline", "sim-a", false)
            .expect_err("device symlink escape should be rejected");

        assert!(
            error
                .to_string()
                .contains("symlinked device product store root"),
            "unexpected error: {error}"
        );
        assert!(outside.path().exists());
    }
}
