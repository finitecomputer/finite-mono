# Prompting Contract

When a prompt is not a simple question or very small ask, guide the user
toward:

1. Self-contained problem statement
2. Acceptance criteria
3. Constraints: musts, must-nots, preferences, and escalation points
4. Decomposition into clean phases
5. Evaluation design for the tests or checks that prove success

# Working In This Repo

- Read `CONTEXT.md` first and use its vocabulary.
- Follow `docs/engineering-style.md`; it is enforced, not aspirational.
- Decisions live in `docs/adr/`; add an ADR when you change one.
- Shortcuts require an entry in `docs/technical-debt-ledger.md` with a
  delete condition, before you rely on them.

## Commands

```sh
just test          # cargo test --workspace
just lint          # cargo fmt --check + clippy --all-targets -D warnings
just dev           # run finitesitesd against .dev-data
just fmt           # rustfmt
```

Every mutation needs a positive test and at least one negative/replay test.
`cargo clippy --all-targets -- -D warnings` must pass before any handoff.

# Publishing And Editor Handoff

- `fsite` is the supported agent-facing surface. Do not bypass it with raw
  nostr events, direct registry writes, DNS edits, or proxy edits.
- Use `FINITE_SITES_API=https://api.finite.chat` for production unless the
  task is explicitly local development.
- Collaborative Project Outputs use Project Repositories:

```sh
fsite describe workflow publish-static-site --output json
fsite describe workflow publish-stateful-app --output json
fsite describe workflow project-config --output json
fsite auth register --output json
fsite project init --config finite.toml --dry-run --output json
fsite project init --config finite.toml --output json
fsite project grant PROJECT --email editor@example.com --send-invite --output json
fsite project share PROJECT OUTPUT --shared --add-email viewer@example.com --send-invite --output json
fsite auth login editor@example.com
fsite auth redeem editor@example.com TOKEN_FROM_EMAIL
fsite auth git PROJECT --email editor@example.com --store --output json
git clone https://git.finite.chat/PROJECT.git
```

- A `[project]`-only `finite.toml` is a valid Bare Project Repository with no
  served output. Add missing outputs later by adding them to `finite.toml` and
  replaying `fsite project init --config finite.toml`.
- Email is optional. Prefer native auth with `fsite auth register`; use
  `fsite auth redeem EMAIL TOKEN --link-native` when an invite token should
  link that email to the local npub **and both identify the same Principal**.
  Never use `--link-native` to give an agent access to a human's email-shaped
  grants; that requires a revocable Finite Sites Email Access Delegation and
  grants no Finite Brain authority. Use
  `fsite auth link-email EMAIL` only when you need to request a fresh token.
  `auth login/redeem` remains the External Principal fallback.
- Commit deploy bytes and push the configured Deploy Branch. Finite Sites
  does not run builds.
- There is no direct bundle upload command. For static sites and apps, commit
  the selected `finite.toml` output path, then push.
- App outputs use `kind = "app"` with an explicit `start` command. Finite sets
  `PORT` and `DATA_DIR`; the app must listen on `0.0.0.0:$PORT` and write live
  mutable state only under `DATA_DIR`.
- Commit the whole source tree that collaborators and agents need. The
  Project Repository is the shared source; `finite.toml` only selects what is
  served as the website.
- Do not reconstruct source from rendered HTML. Use the Project Repository.
- A generated `/llms.txt` is platform guidance only. If a project publishes
  its own `/llms.txt`, preserve it and treat it as the project's authority.
- Never commit, print, or upload `.finite/`, `.env*`, private keys, or build
  caches. Avoid dependency directories unless they are intentionally committed
  runtime payload for an app output.

# GitHub Release Shape

The public repository is expected to publish `fsite` binaries from tags named
`v*`. Keep README install commands and generated `/llms.txt` instructions in
sync with `.github/workflows/release.yml`.
