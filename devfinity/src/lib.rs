use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub mod process;
pub mod stack;
pub mod topology;
pub mod vars;

pub use stack::{RunningDevfinityStack, StackRunMode};
pub use topology::{DevfinityStack, StackProfile};
pub use vars::{DevfinityVars, StackEnv, StackPaths, StackPorts};

/// Backward-compatible alias for older local callers while `DevfinityStack`
/// becomes the primary harness type.
pub type Stack = DevfinityStack;

pub fn run_devfinity_test<F>(f: F) -> Result<()>
where
    F: FnOnce(&DevfinityStack) -> Result<()>,
{
    let repo_root = default_repo_root()?;
    let stack = DevfinityStack::new_fixture_with_repo_root(repo_root, default_test_state_dir())?;
    let mut running = stack.start()?;
    stack.apply_env_to_current_process();

    let test_result = f(&stack);
    if test_result.is_err() {
        let _ = stack.mark_error();
    }

    let shutdown_result = running.shutdown();
    match (test_result, shutdown_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
    }
}

fn default_test_state_dir() -> PathBuf {
    std::env::var_os("DEVFINITY_TEST_STATE_DIR").map_or_else(
        || std::env::temp_dir().join("devfinity"),
        |value| Path::new(&value).to_path_buf(),
    )
}

fn default_repo_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .context("devfinity crate directory did not have a parent repo root")
}
