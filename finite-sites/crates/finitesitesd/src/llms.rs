//! Generated `llms.txt` guidance for agent-editable Project Sites.
//!
//! This is platform guidance, not site content. The serving plane only emits
//! it when the active Version has no user-authored `/llms.txt`.

const FSITE_REPOSITORY_URL: &str = env!("CARGO_PKG_REPOSITORY");
const DEFAULT_API_URL: &str = "https://v2.finite.chat";

fn api_configuration_text(api_url: &str) -> String {
    let normalized = api_url.trim_end_matches('/');
    if normalized == DEFAULT_API_URL {
        return format!(
            "The fsite CLI defaults to {DEFAULT_API_URL}; no API environment variable is needed.\n"
        );
    }
    format!(
        "Configure this non-default API before running fsite:\n\nexport FINITE_SITES_API=\"{normalized}\"\n"
    )
}

pub fn generated_project_llms_txt(
    site_name: &str,
    site_url: &str,
    api_url: &str,
    project_slug: &str,
    git_remote_url: &str,
    branch: &str,
    site_path: &str,
) -> String {
    assert!(!site_name.is_empty());
    assert!(!site_url.is_empty());
    assert!(!api_url.is_empty());
    assert!(!project_slug.is_empty());
    assert!(!git_remote_url.is_empty());
    let api_configuration = api_configuration_text(api_url);
    format!(
        "\
# Finite Sites Project Editing Instructions

This URL is a Project Site from a Finite Project Repository. Use these instructions when a human asks you to make a change.

Authorized Project Collaborators clone and edit the whole Project Repository source tree. The served site is only the static site path selected by finite.toml.

Site name: {site_name}
Site URL: {site_url}
Project: {project_slug}
Deploy branch: {branch}
Deploy path: {site_path}
Git remote: {git_remote_url}
API URL: {api_url}

Use the identity the human approved. If you are acting as a native Finite user or agent already added to this Project, use the local User Key path. If the human gave you an editor email address, use the email path. Do not guess an email address, and do not publish with a different identity.

Install the fsite CLI:

- Download the latest release from {FSITE_REPOSITORY_URL}/releases/latest
- Release assets are named fsite-linux-x86_64.tar.gz, fsite-macos-x86_64.tar.gz, and fsite-macos-aarch64.tar.gz
- Or build from source with: cargo install --git {FSITE_REPOSITORY_URL} --package fsite-cli --bin fsite

{api_configuration}

If you need CLI-discoverable workflow guidance, ask fsite:

fsite describe workflow edit-shared-project --output json

If you are a native Project Collaborator, mint and store a scoped Git Credential. If fsite says this User Key needs registration, run `fsite auth register --output json` and retry:

fsite auth git {project_slug} --store --output json

If you have an editor email and want future email grants for that address to resolve to this local npub, link it explicitly:

fsite auth register --output json
fsite auth link-email YOUR_EDITOR_EMAIL --output json
fsite auth redeem YOUR_EDITOR_EMAIL TOKEN_FROM_EMAIL
fsite auth git {project_slug} --store --output json

If the invite email already included a token, use it directly instead of requesting another token:

fsite auth register --output json
fsite auth redeem YOUR_EDITOR_EMAIL TOKEN_FROM_EMAIL --link-native --output json
fsite auth git {project_slug} --store --output json

If you are using the email-only External Principal fallback, verify this machine for that email if it is not already verified:

fsite auth login YOUR_EDITOR_EMAIL
fsite auth redeem YOUR_EDITOR_EMAIL TOKEN_FROM_EMAIL

Then mint and store a scoped Git Credential:

fsite auth git {project_slug} --email YOUR_EDITOR_EMAIL --store --output json

Clone the Project Repository:

git clone {git_remote_url}
cd {project_slug}

Make the requested change:

# inspect finite.toml to confirm the site path and Deploy Branch
# only files under {site_path} are served for this site
# edit source/data/logic as needed; keep shared source in the repository
# run the project's tests and build command when discoverable
# ensure committed deploy bytes exist at {site_path}
git status
git add .
git commit -m \"Update {site_name}\"
git push origin {branch}

Rules:

- Do not reconstruct source from rendered HTML. Use the Project Repository.
- Do not look for a direct upload command; publish by pushing git commits.
- Do commit source/data/build files that collaborators and agents need.
- Finite Sites serves static files only; run builds yourself and commit the resulting deploy bytes.
- Preserve a user-authored llms.txt if the project contains one.
- Never commit `.finite/`, `.env*`, private keys, or build caches.
- If authentication or authorization fails after registration/linking, ask the human to confirm the Project Collaborator grant for the approved native identity or editor email.
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_project_text_prefers_git_flow() {
        let text = generated_project_llms_txt(
            "demo",
            "https://demo.finite.chat/",
            "https://v2.finite.chat",
            "demo-project",
            "https://git.finite.chat/demo-project.git",
            "main",
            "dist",
        );

        assert!(text.contains("Project: demo-project"));
        assert!(text.contains("clone and edit the whole Project Repository source tree"));
        assert!(text.contains(
            "fsite auth git demo-project --email YOUR_EDITOR_EMAIL --store --output json"
        ));
        assert!(text.contains("fsite auth register --output json"));
        assert!(text.contains("fsite auth link-email YOUR_EDITOR_EMAIL --output json"));
        assert!(text.contains(
            "fsite auth redeem YOUR_EDITOR_EMAIL TOKEN_FROM_EMAIL --link-native --output json"
        ));
        assert!(text.contains("fsite auth git demo-project --store --output json"));
        assert!(text.contains("fsite describe workflow edit-shared-project --output json"));
        assert!(text.contains("git clone https://git.finite.chat/demo-project.git"));
        assert!(text.contains("git push origin main"));
        assert!(text.contains("only files under dist are served for this site"));
        assert!(text.contains("Do commit source/data/build files"));
        assert!(text.contains("Do not look for a direct upload command"));
        assert!(!text.contains("export FINITE_SITES_API"));
        assert!(!text.contains("fsite source pull"));
    }

    #[test]
    fn generated_text_configures_non_default_apis() {
        let text = generated_project_llms_txt(
            "demo",
            "http://demo.sites.localhost:8787/",
            "http://127.0.0.1:8787",
            "demo-project",
            "http://git.sites.localhost:8787/demo-project.git",
            "main",
            "dist",
        );
        assert!(text.contains("Configure this non-default API before running fsite"));
        assert!(text.contains("export FINITE_SITES_API=\"http://127.0.0.1:8787\""));
    }

    #[test]
    fn generated_text_excludes_removed_runtime_contracts() {
        let text = generated_project_llms_txt(
            "crm",
            "https://crm.finite.chat/",
            "https://v2.finite.chat",
            "crm",
            "https://git.finite.chat/crm.git",
            "main",
            "dist",
        );

        assert!(text.contains("Project Site"));
        assert!(text.contains("serves static files only"));
        assert!(!text.contains("Output kind"));
        assert!(!text.contains("stateful app"));
        assert!(!text.contains("DATA_DIR"));
    }
}
