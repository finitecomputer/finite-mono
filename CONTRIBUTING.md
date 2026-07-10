# Contributing to Finite Mono

The 15-minute path from clone to a running stack.

## 1. Toolchain (one-time, ~5 min)

Everything is pinned by Nix — do not install Rust/Node/Postgres yourself.

1. Install Nix (with flakes enabled) — https://nixos.org/download
2. Install direnv — https://direnv.net
3. `git clone https://github.com/finitecomputer/finite-mono && cd finite-mono`
4. `direnv allow` — the flake dev shell provides rustc/cargo, just,
   process-compose, Postgres, and friends.

No direnv? Prefix commands with `scripts/with-dev-env`.

## 2. Boot the local stack (~5 min)

```
just dev up
```

devfinity generates and supervises the whole stack under process-compose:
local Postgres, finite-saas-core, finitechat server, the Hosted Web Device,
finitesitesd, and the dashboard dev server. URLs land in
`.local-state/devfinity/runs/default/urls.txt`.
Quit the TUI (or Ctrl-C) to shut down; `just dev cleanup` recovers from
orphaned state.

## 3. Make a change and see it

Pick a product surface:

- **Dashboard** (Next.js): edit `finitecomputer-v2/apps/dashboard/`, the dev
  server hot-reloads inside the stack.
- **CLI** (`finitechat`, `fsite`, `fbrain`): `cargo run -p finitechat-cli --bin finitechat -- --help`
- **Server** (core / chat / sites): edit, then restart the stack.
- **iOS**: see `finitechat/ios/` + `finitechat-rmp` (XcodeGen; run from
  `finitechat/` so `rmp.toml` resolves).
- **Electron**: see `finitechat/` Electron app docs.

## 4. Gates before you push

```
just check    # cargo check --workspace --locked
just fmt      # rustfmt
just test     # cargo test --workspace --locked
just dev saas-smoke  # real Apple Runtime + Hosted Web chat + restart healing
just dev smoke       # portable services-only integration smoke (Linux CI)
```

CI (`.github/workflows/ci.yml`) runs fmt/clippy/tests against real Postgres,
dashboard lint/test/build, the finitechat Hermes bridge suite, and
skills/search checks on every PR.

## Rules worth repeating

- **This repo is public. Never commit a secret value.** Names and locations
  only (`infra/README.md` explains the secrets model).
- Release asset names and legacy install URLs are product contracts.
- Deploy changes go in `infra/`, not in shell history.
