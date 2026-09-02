# Finite automated PR review runner

Use a trusted Mac self-hosted GitHub Actions runner when automated PR review
must use ChatGPT-managed Codex auth instead of an OpenAI API key.

## Runner contract

- Register the runner on `finitecomputer/finite-mono`.
- Give it the custom label `finite-pr-review`; GitHub also adds `self-hosted`
  and `macOS`.
- Run it as a dedicated macOS user whose home directory persists between jobs.
- Keep Codex CLI signed in for that user with file-backed credentials. The
  workflow expects `codex exec` to reuse the saved ChatGPT-managed auth in
  that user's `CODEX_HOME` or `~/.codex`.
- Keep `gh`, `jq`, `git`, `nix`, and `codex` available on the runner user's
  `PATH`.
- Keep Nix configured for this repository. The workflow also sets
  `accept-flake-config = true`, so the repo flake can provide the Finite
  Cachix substituter and trusted key.

Do not use this runner for public repositories or fork PRs. The workflow skips
fork PRs because the runner has local Codex credentials and may run PR code
during focused validation.

## One-time setup

1. On the Mac Mini, create or choose the dedicated runner user.
2. Install the GitHub CLI, Nix, Codex CLI, and the standard Apple command-line
   tools for the repo.
3. Sign in as the runner user. Configure Codex to keep refreshable
   ChatGPT-managed credentials in `~/.codex/auth.json`, then sign in:

   ```bash
   mkdir -p ~/.codex
   touch ~/.codex/config.toml
   grep -q '^cli_auth_credentials_store' ~/.codex/config.toml \
     || printf '\ncli_auth_credentials_store = "file"\n' >> ~/.codex/config.toml
   codex login
   test -f ~/.codex/auth.json
   jq '{auth_mode, has_tokens: (.tokens != null), has_refresh_token: ((.tokens.refresh_token // "") != "")}' ~/.codex/auth.json
   ```

4. From a checkout of this repository, verify the runner user's tools:

   ```bash
   gh auth login
   codex doctor --summary
   nix --version
   nix develop .#default --command just --version
   ```

5. In GitHub, open the repository settings, then Actions, then Runners, then
   create a new macOS self-hosted runner. Follow GitHub's generated install
   commands on the Mac Mini. Include the custom label when configuring:

   ```bash
   ./config.sh --url https://github.com/finitecomputer/finite-mono --token <token> --labels finite-pr-review
   ```

6. Install and start the runner service from the runner directory:

   ```bash
   ./svc.sh install
   ./svc.sh start
   ./svc.sh status
   ```

7. Confirm the runner appears online and idle in GitHub with the
   `finite-pr-review` label.

## Review identity

By default, the workflow posts reviews with GitHub's `GITHUB_TOKEN`, usually as
`github-actions[bot]`. If reviews should come from a dedicated GitHub account
or a personal account, create a repository secret named
`FINITE_PR_REVIEW_GITHUB_TOKEN` with pull-request review permission. That token
is separate from OpenAI billing and does not change Codex usage.

If `github-actions[bot]` approvals should count for branch protection, enable
the repository or organization Actions setting that allows GitHub Actions to
create and approve pull requests.

## Validation

After the workflow lands on `main`, manually dispatch one run:

```bash
gh workflow run finite-automated-pr-review.yml -f pr_number=<pr-number>
```

The run should:

- Pick the Mac runner with labels `self-hosted`, `macOS`, and
  `finite-pr-review`.
- Record `.codex-review/preflight.log`.
- Run `codex exec` with the runner's saved ChatGPT-managed auth.
- Submit exactly one GitHub PR review on the current PR head, or explain why it
  skipped.
