use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");

    if let Some(commit) = git_output(&["rev-parse", "--short=12", "HEAD"]) {
        println!("cargo:rustc-env=FINITECHAT_BUILD_COMMIT={commit}");
    }
    if let Some(branch) = git_output(&["rev-parse", "--abbrev-ref", "HEAD"]) {
        println!("cargo:rustc-env=FINITECHAT_BUILD_BRANCH={branch}");
    }
    if git_is_dirty() {
        println!("cargo:rustc-env=FINITECHAT_BUILD_DIRTY=true");
    } else {
        println!("cargo:rustc-env=FINITECHAT_BUILD_DIRTY=false");
    }
}

fn git_output(args: &[&str]) -> Option<String> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let output = Command::new("git")
        .args(args)
        .current_dir(manifest_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn git_is_dirty() -> bool {
    let Some(status) = git_output(&["status", "--porcelain"]) else {
        return false;
    };
    !status.trim().is_empty()
}
