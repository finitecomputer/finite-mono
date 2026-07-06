//! Generated `llms.txt` guidance for agent-editable Project Outputs.
//!
//! This is platform guidance, not site content. The serving plane only emits
//! it when the active Version has no user-authored `/llms.txt`.

const FSITE_REPOSITORY_URL: &str = env!("CARGO_PKG_REPOSITORY");
const DEFAULT_API_URL: &str = "https://api.finite.chat";

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

#[allow(clippy::too_many_arguments)]
pub fn generated_project_llms_txt(
    output_name: &str,
    output_url: &str,
    api_url: &str,
    project_slug: &str,
    git_remote_url: &str,
    output_id: &str,
    output_kind: &str,
    branch: &str,
    output_path: &str,
    start_command: Option<&str>,
) -> String {
    assert!(!output_name.is_empty());
    assert!(!output_url.is_empty());
    assert!(!api_url.is_empty());
    assert!(!project_slug.is_empty());
    assert!(!git_remote_url.is_empty());
    let api_configuration = api_configuration_text(api_url);
    let app_contract = app_contract_text(output_kind, start_command);
    format!(
        "\
# Finite Sites Project Editing Instructions

This URL is a Project Output from a Finite Project Repository. Use these instructions when a human asks you to make a change.

Authorized Project Collaborators clone and edit the whole Project Repository source tree. The served output is only the Project Output path selected by finite.toml.

Output name: {output_name}
Output URL: {output_url}
Project: {project_slug}
Output: {output_id}
Output kind: {output_kind}
Deploy branch: {branch}
Deploy path: {output_path}
Git remote: {git_remote_url}
API URL: {api_url}
{app_contract}

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

# inspect finite.toml to confirm the output path and Deploy Branch
# only files under {output_path} are served for this output
# edit source/data/logic as needed; keep shared source in the repository
# run the project's tests and build command when discoverable
# ensure committed deploy bytes exist at {output_path}
git status
git add .
git commit -m \"Update {output_name}\"
git push origin {branch}

Rules:

- Do not reconstruct source from rendered HTML. Use the Project Repository.
- Do not look for a direct upload command; publish by pushing git commits.
- Do commit source/data/build files that collaborators and agents need.
- For site outputs, Finite Sites does not run builds; run builds yourself and commit the resulting deploy bytes.
- For document outputs, commit authored Markdown, not generated HTML.
- For app outputs, commit source, migrations, seed data, and any explicit runtime bundle files; write live mutable state only under DATA_DIR.
- Preserve a user-authored llms.txt if the project contains one.
- Never commit `.finite/`, `.env*`, private keys, or build caches. Commit dependency directories only when they are intentionally required runtime payload for an app output.
- If authentication or authorization fails after registration/linking, ask the human to confirm the Project Collaborator grant for the approved native identity or editor email.
"
    )
}

fn app_contract_text(output_kind: &str, start_command: Option<&str>) -> String {
    if output_kind != "app" {
        return String::new();
    }
    let start = start_command.unwrap_or("START_COMMAND_FROM_finite.toml");
    format!(
        "
This is a stateful app output.

Finite Sites will run the committed files at the Deploy path as a process using:

{start}

Finite Sites sets PORT and DATA_DIR for the process. The app must listen on 0.0.0.0:$PORT. DATA_DIR is the only live mutable state location: it survives deploys, restarts, and wake/sleep. The committed Project Repository is source, migrations, seed data, and explicit runtime payload; deploys must not overwrite existing DATA_DIR contents.
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
            "https://api.finite.chat",
            "demo-project",
            "https://git.finite.chat/demo-project.git",
            "site",
            "site",
            "main",
            "dist",
            None,
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
        assert!(text.contains("only files under dist are served for this output"));
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
            "site",
            "site",
            "main",
            "dist",
            None,
        );
        assert!(text.contains("Configure this non-default API before running fsite"));
        assert!(text.contains("export FINITE_SITES_API=\"http://127.0.0.1:8787\""));
    }

    #[test]
    fn generated_text_documents_stateful_app_contract() {
        let text = generated_project_llms_txt(
            "crm",
            "https://crm.finite.chat/",
            "https://api.finite.chat",
            "crm",
            "https://git.finite.chat/crm.git",
            "web",
            "app",
            "main",
            "app",
            Some("bun server.ts"),
        );

        assert!(text.contains("Output kind: app"));
        assert!(text.contains("This is a stateful app output."));
        assert!(text.contains("bun server.ts"));
        assert!(text.contains("DATA_DIR is the only live mutable state location"));
        assert!(text.contains("listen on 0.0.0.0:$PORT"));
    }
}
