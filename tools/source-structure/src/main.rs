use anyhow::Context;
use source_structure::{Config, check_repository};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let config_path = root.join(".config/source-structure.json");
    let config: Config = serde_json::from_str(
        &std::fs::read_to_string(&config_path).context("read source structure configuration")?,
    )
    .context("parse source structure configuration")?;
    let diagnostics = check_repository(&root, &config)?;
    for diagnostic in &diagnostics {
        eprintln!("{diagnostic}");
    }
    anyhow::ensure!(
        diagnostics.is_empty(),
        "source structure check failed ({} violations)",
        diagnostics.len()
    );
    println!("Source structure checks passed.");
    Ok(())
}
