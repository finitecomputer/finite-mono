use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use devfinity::DevfinityStack;

#[test]
fn cli_status_uses_library_stack_layout() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("devfinity crate has a parent")
        .to_path_buf();
    let state_dir = std::env::temp_dir().join(format!(
        "devfinity-cli-layout-{}-{}",
        std::process::id(),
        now_millis()
    ));
    let _ = fs::remove_dir_all(&state_dir);

    let stack = DevfinityStack::new_with_repo_root(repo_root.clone(), state_dir.clone())
        .expect("library stack");
    let paths = stack.paths();
    let output = Command::new(env!("CARGO_BIN_EXE_devfinity"))
        .current_dir(&repo_root)
        .arg("--state-dir")
        .arg(&state_dir)
        .arg("status")
        .output()
        .expect("run devfinity status");

    assert!(
        output.status.success(),
        "devfinity status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&paths.run_dir.display().to_string()),
        "status output did not use library run dir {}\n{stdout}",
        paths.run_dir.display()
    );
    assert!(
        stdout.contains(&format!("control: {}", paths.control_dir.display())),
        "status output did not use library control dir {}\n{stdout}",
        paths.control_dir.display()
    );
    assert!(
        stdout.contains("processes:"),
        "status output did not include process section\n{stdout}"
    );

    let _ = fs::remove_dir_all(state_dir);
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}
