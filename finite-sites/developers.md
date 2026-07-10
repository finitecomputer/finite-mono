# Finite Sites Developer Guide

This document is for humans and agents working on the Finite Sites codebase.
The root `README.md` is intentionally agent-first and focuses on installing
and using `fsite`.

Read `CONTEXT.md` before changing code. It defines the product vocabulary used
by code, docs, tests, and prompts. Follow `docs/engineering-style.md`; it is
the style contract for this repo.

## Product Shape

Finite Sites replaces self-hosting sites from inside agent machines with a
Finite-owned serving substrate behind wildcard domains. Agents collaborate in
git. A Project Repository is the editable source of truth, and Project Outputs
select committed bytes to serve as immutable Versions.

Current v1 capabilities:

- Static site Project Outputs served from committed deploy bytes.
- Document Outputs served from authored Markdown under the document base
  domain, including clean routes, Markdown companion URLs, `/llms.txt`, and
  `/llms-full.txt`.
- NIP-98-signed registry mutations through a local Publishing Key.
- Self-registration with `fsite auth register`.
- Project Repositories over Git smart HTTP through `git-http-backend`.
- Generated `/llms.txt` for project-backed editable outputs when the project
  did not publish that path itself.
- Per-output visibility: `private`, `shared`, or `public`.
- Email magic links for external viewer and collaborator bootstrap.
- Operator controls for publish grants, output disable/delete, and selected
  public-read Project Repository visibility.

Stateful apps and PDF outputs use or will use the same Project Repository
model. See `docs/roadmap.md` and the ADRs for the current design history.

## Crate Layout

| Crate | Ownership |
| --- | --- |
| `finitesites-proto` | Nostr events, NIP-98, manifests, names, limits, DTOs, `finite.toml` parsing |
| `finitesites-blob` | Content-addressed blob storage |
| `finitesites-store` | SQLite registry: grants, projects, outputs, versions, shares, tokens |
| `finitesites-engine` | Product decisions: project init, git deploy, sharing, viewing, auth |
| `finitesitesd` | HTTP server, wildcard site serving, Git smart HTTP, operator commands |
| `fsite-cli` | Agent-facing CLI binary, `fsite` |

## Local Development

Common commands:

```sh
just dev
just test
just lint
just fmt
```

`just dev` runs `finitesitesd` against `.dev-data`.

Manual local quickstart:

```sh
# Terminal 1: run the server.
cargo run -p finitesitesd -- serve --data .dev-data

# Terminal 2: use the CLI against the local server.
export FINITE_SITES_API=http://127.0.0.1:8787
cargo run -p fsite-cli --bin fsite -- whoami
cargo run -p fsite-cli --bin fsite -- auth register --output json
```

Create a static-site Project from an example:

```sh
cargo run -p fsite-cli --bin fsite -- project init \
  --config examples/finitechat-native-mockup/finite.toml \
  --dry-run \
  --output json

cargo run -p fsite-cli --bin fsite -- project init \
  --config examples/finitechat-native-mockup/finite.toml \
  --output json

cargo run -p fsite-cli --bin fsite -- auth git finitechat-native --store --output json

tmp="$(mktemp -d)"
git clone http://git.sites.localhost:8787/finitechat-native.git "$tmp/finitechat-native"
rsync -a --delete examples/finitechat-native-mockup/ "$tmp/finitechat-native/"
cd "$tmp/finitechat-native"
git add finite.toml index.html
git commit -m "Seed finitechat native mockup"
git push origin main
```

Open the local site:

```sh
FINITE_SITES_API=http://127.0.0.1:8787 fsite view finitechat-native-mockup --output json
open http://finitechat-native-mockup.sites.localhost:8787/
```

The name form of `fsite view` resolves through the configured API for Projects
owned by the local Finite identity. Never infer a `finite.chat` URL from a
local slug; use the server-returned `output_url`.

`*.sites.localhost` resolves to loopback in modern browsers. For curl, pass a
Host header against `127.0.0.1:8787`.

```sh
curl -H "Host: finitechat-native-mockup.sites.localhost:8787" \
  http://127.0.0.1:8787/
```

### Git runtime dependency and Project Init recovery

`finitesitesd` executes the system `git` binary for bare-repository setup and
Git smart HTTP. The daemon preflights `git --version` before it starts serving;
`/api/v1/healthz` also returns 503 with `git_unavailable` if that dependency
disappears. Packaged services must therefore put Git on the daemon's runtime
`PATH` explicitly (the NixOS module uses `path = [ pkgs.git ];`).

Project Init commits registry state before provisioning the corresponding bare
repository. That boundary is intentional so an interrupted repository setup
cannot erase the Project or its claimed outputs. A setup failure returns
`git_repository_setup_failed`; after fixing Git or repository storage, replay
the identical Project Init once. The replay uses the existing Project ID,
repairs a missing or partially initialized bare repository, and returns
`created: false`. Do not delete the row or mint a replacement slug as normal
recovery.

For local guests that can reach only one gateway origin, set `--api-url` and
`--git-url` to that same origin. Requests matching `/{slug}.git` are routed to
Git smart HTTP while `/api/v1/*` remains on the API plane. This is a server
transport feature; no Runner-specific Sites configuration is required.

## Test Expectations

The repo's test bar is in `docs/engineering-style.md`.

- Registry mutations need positive tests plus negative or replay tests.
- Idempotent mutations need replay coverage.
- Storage invariants need restart or migration coverage.
- Git deploy behavior should be covered through the real HTTP e2e path.
- `cargo clippy --all-targets -- -D warnings` must pass before handoff.

Use `just test` and `just lint` before committing. For release confidence,
also run:

```sh
cargo build --locked --release --workspace
```

## Production And Operations

Production runs `finitesitesd` as a systemd service behind Caddy and
Cloudflare. The server owns the control-plane API, Git smart HTTP, and
wildcard site serving. Publishing does not mutate host configuration; it is
registry, blob, and git repository state.

Runbook and deploy files:

- `docs/deploy-finite-lat-2.md`
- `infra/hosts/lat2/` (mono root — unit files, Caddyfile, and env example;
  moved from `deploy/finite-lat-2/`)
- `docs/technical-debt-ledger.md`

Important production rule: use `fsite` for agent-facing publishing and editor
handoff. Do not bypass it with raw Nostr events, direct registry writes, DNS
edits, or proxy edits unless you are doing an explicit operator recovery from
the runbook.

## Release Shape

Tags named `v*` trigger `.github/workflows/release.yml`.

The GitHub release currently packages `fsite` binaries for:

- `fsite-linux-x86_64`
- `fsite-macos-aarch64`
- `fsite-macos-x86_64`

The server binary `finitesitesd` is built locally or on the production host;
it is not currently a GitHub release asset.

Before tagging:

```sh
just test
just lint
cargo build --locked --release --workspace
```

## Documentation Map

- `CONTEXT.md`: glossary; use these words in code and prompts.
- `AGENTS.md`: prompting contract and repo commands.
- `docs/engineering-style.md`: engineering rules and test shape.
- `docs/adr/`: decisions and alternatives.
- `docs/roadmap.md`: planned tiers and future outputs.
- `docs/bare-repos-and-skills-hosting.md`: source-only Project Repository
  requirements and public-read policy for finitecomputer-managed skills.
- `docs/technical-debt-ledger.md`: accepted shortcuts with delete conditions.
- `../finite-skills/skills/software-development/finite-sites-publishing-finite/SKILL.md`:
  canonical managed agent skill for publishing. Finite Sites owns the API and
  CLI contract; `finite-skills` is the only editable deployed skill source.
