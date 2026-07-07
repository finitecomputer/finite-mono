use super::*;

/// Candidate `AGENTS.md` files from nearest page directory to Vault root.
pub fn agent_discovery_paths(
    page_path: &SafeRelativePath,
) -> Result<Vec<SafeRelativePath>, CoreError> {
    let mut dirs = page_path.as_str().split('/').collect::<Vec<_>>();
    dirs.pop();

    let mut candidates = Vec::new();
    for depth in (0..=dirs.len()).rev() {
        let candidate = if depth == 0 {
            "AGENTS.md".to_owned()
        } else {
            format!("{}/AGENTS.md", dirs[..depth].join("/"))
        };
        candidates.push(SafeRelativePath::new("agent_path", candidate)?);
    }
    Ok(candidates)
}
