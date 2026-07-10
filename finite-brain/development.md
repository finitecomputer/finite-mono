# FiniteBrain Development Guide

This document is for humans and agents working on the FiniteBrain codebase.
The root `README.md` is intentionally agent-first and focuses on installing
and using `fbrain`.

Read `CONTEXT.md` before changing code. It defines the product vocabulary used
by code, docs, tests, skills, and prompts.

## Product Shape

FiniteBrain is an encrypted knowledge system where trusted clients and agent
runtimes open Folder Keys locally, materialize readable Pages as markdown, and
sync encrypted changes back to the server.

A Vault is a namespace of many Folder-scoped LLM wikis. Folder access is the
wiki boundary: each top-level Folder owns its own `_index.md`, `config.md`,
`log.md`, sources, compiled pages, outputs, and access-safe activity trail.

Current v1 capabilities:

- SQLite-backed Vaults, Folders, Folder Key Grants, invitations, shares, mounts,
  and encrypted sync records.
- Nostr-authenticated protected HTTP routes.
- Product Client at `/client` for browser-based trusted-client workflows.
- Development Smoke UI at `/smoke/ui` for local inspection only.
- `fbrain` CLI for agent-native Vault Working Trees.
- Folder-scoped default vault files for AGENTS/HUMANS guidance and LLM wiki
  conventions.

## Official URLs

Current hosted smoke service:

- API and Product Client origin: `https://brain.smoke.finite.computer`
- Product Client: `https://brain.smoke.finite.computer/client`
- Health check: `https://brain.smoke.finite.computer/health`
- Client config: `https://brain.smoke.finite.computer/client/config.json`

Repository and releases:

- Source: `https://github.com/finitecomputer/finite-brain`
- Release downloads: `https://github.com/finitecomputer/finite-mono/releases/download/fbrain-latest/` (rolling alias; versioned tags are `fbrain/vX.Y.Z`)

No production FiniteBrain URL is canonized in this repository yet. Do not
invent one in docs, skills, tests, or agent instructions.

## Crate Layout

| Crate | Ownership |
| --- | --- |
| `finite-brain-core` | Portable v1 domain model, validation, crypto-adjacent contracts, defaults, OKF, and Vault Working Tree projection |
| `finite-brain-store` | SQLite schema, transactions, persistence, sync records, invitations, shares, and mounts |
| `finite-brain-server` | HTTP router, protected routes, static Product Client assets, Smoke UI, CORS, and API tests |
| `finite-brain-app` | `finite-brain` application server binary |
| `finite-brain-cli` | Agent-facing CLI crate and `fbrain` binary |
| `../finite-nostr` | Reusable Nostr primitives used by FiniteBrain |

## Local Development

Common checks:

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
node --check crates/finite-brain-server/src/product-client.js
node --check crates/finite-brain-server/src/smoke-ui.js
node crates/finite-brain-server/src/product-client.test.js
```

Run the local server:

```sh
FINITE_BRAIN_ADDR=127.0.0.1:3015 \
FINITE_BRAIN_PUBLIC_BASE_URL=http://127.0.0.1:3015 \
FINITE_BRAIN_DB=.dev-data/finite-brain.sqlite3 \
cargo run -p finite-brain-app
```

Open:

```text
http://127.0.0.1:3015/client
http://127.0.0.1:3015/smoke/ui
```

Use the CLI through Cargo during development:

```sh
cargo run -p finite-brain-cli --bin fbrain -- --version
cargo run -p finite-brain-cli --bin fbrain -- doctor --server http://127.0.0.1:3015
cargo run -p finite-brain-cli --bin fbrain -- auth status --json
```

Or build once and use `target/debug/fbrain`.

## `fbrain` Agent Workflow

Use `fbrain` as the agent-facing command. Agents work in a Vault Working Tree
with ordinary file tools; `fbrain` owns identity, server transport, Folder Key
opening, local daemon state, sync, conflicts, access inspection, and safe
administration commands.

Useful commands:

```sh
fbrain doctor --server "$FINITE_BRAIN_SERVER_URL"
fbrain auth status --json
fbrain open <vault-id> ./vault-tree
cd ./vault-tree
fbrain sync now --summary
fbrain conflicts --json
fbrain status --json
fbrain activity
fbrain folder list --vault <vault-id>
fbrain access list --vault <vault-id>
```

Use global `--config-dir <path>` when an agent needs a dedicated signer/config
directory without relying on shell-level environment persistence:

```sh
fbrain --config-dir "$HOME/.config/finitebrain" auth status --json
```

`fbrain` resolves server URLs in this order:

1. explicit `--server`
2. saved Vault Working Tree server URL
3. `FINITE_BRAIN_SERVER_URL`
4. legacy `FINITE_BRAIN_PUBLIC_BASE_URL`

The CLI accepts `https://` endpoints and `http://` only for localhost or
loopback addresses.

## Environment Variables

- `FINITE_BRAIN_ADDR`: server bind address, default `127.0.0.1:3015`.
- `FINITE_BRAIN_SERVER_URL`: agent/CLI transport base URL.
- `FINITE_BRAIN_PUBLIC_BASE_URL`: browser-visible Product Client origin and
  legacy CLI fallback.
- `FINITE_BRAIN_DB`: SQLite database path, default `finite-brain.sqlite3`.
- `FINITE_IDENTITY_AUTHORITY`: finite-identity Authority base URL used by
  email-targeted Vault Invitation claims to verify current email proof.
- `FINITE_BRAIN_INVITE_MAILER`: optional Brain invite delivery mode: `dev`,
  `resend`, `postmark`, or `none`.
- `FINITE_BRAIN_INVITE_MAIL_FROM`: sender address for `resend` or `postmark`.
- `FBRAIN_CONFIG_DIR`: local `fbrain` config directory for prototype signer
  state. Prefer global `--config-dir` in scripts and agent runtimes.

## Test Expectations

- Protocol, storage, sync, crypto-adjacent, and access-control changes need
  positive tests plus stale/replay/negative tests where relevant.
- SQLite behavior should be tested through persistence/reopen paths when a
  migration or transaction invariant matters.
- Product Client changes need `node --check` plus
  `node crates/finite-brain-server/src/product-client.test.js`.
- Before handoff, run `cargo fmt --all --check`, `cargo test --workspace`, and
  `cargo clippy --workspace --all-targets -- -D warnings`.

For release confidence, also run:

```sh
cargo build --locked --release --package finite-brain-cli --bin fbrain
```

## Release Shape

Tags named `v*` trigger `.github/workflows/release.yml`.

The GitHub release packages `fbrain` binaries for:

- `fbrain-linux-x86_64`
- `fbrain-macos-aarch64`
- `fbrain-macos-x86_64`

Each asset is uploaded as `.tar.gz` with a matching `.sha256` file.

The `finite-brain` server binary is built through Cargo/Nix for hosted
deployments. It is not currently a GitHub release asset.

Before tagging:

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --locked --release --package finite-brain-cli --bin fbrain
node --check crates/finite-brain-server/src/product-client.js
node --check crates/finite-brain-server/src/smoke-ui.js
node crates/finite-brain-server/src/product-client.test.js
```

## Safety And Public Repo Rules

Do not commit:

- Nostr private keys, `nsec` values, auth files, or signer state.
- Folder Keys, grant plaintext, rotation bodies, or decrypted sync internals.
- Live SQLite databases, backups, runtime PVC contents, or smoke/prod user data.
- Tokens, `.env*`, API keys, OAuth secrets, Telegram secrets, or deploy keys.

Do commit:

- Rust source, docs, tests, Product Client source assets, specs, ADRs, and
  reusable agent skill instructions.
- Redacted smoke findings and commands that do not expose live secrets or user
  data.

Public docs may reference the smoke service URL, local loopback development
URLs, and release download URLs. They must not imply that the development Smoke
UI is the production client.

## Documentation Map

- `CONTEXT.md`: glossary and product vocabulary.
- `AGENTS.md`: repo agent guide.
- `README.md`: agent-first install and usage guide.
- `docs/specs/finitebrain-portability-spec.md`: Portable v1 contract.
- `docs/adr/`: decisions and alternatives.
- `docs/runbooks/`: operational smoke and local parity runbooks.
- `skills/finitebrain/SKILL.md`: packaged FiniteBrain agent skill.
