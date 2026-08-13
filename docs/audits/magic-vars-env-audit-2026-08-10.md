# Magic Vars / Env Vars Audit - 2026-08-10

## Scope

This audit looked for process-level "magic vars" across the tracked monorepo,
with environment variables as the primary target. I scanned `git ls-files`,
skipping lock files, media/binary assets, and very large generated files. The
extractor counted direct env reads/writes in Rust, Node/TypeScript, Python,
shell-like config, systemd/Nix/YAML env wiring, and build-time env macros. Broad
ALL_CAPS prose and protocol constants were sampled, but not counted as env vars
unless they matched an env/config pattern.

Inventory from the strict pass:

| Metric | Count |
|---|---:|
| Tracked files | 1,763 |
| Text files read | 1,693 |
| Recognized env names | 408 |
| Recognized env occurrences | 940 |
| Finite-prefixed env names (`FC_`, `FINITE*`, `FBRAIN`, `DEVFINITY`, `HERMES`, `RMP_`) | 321 |
| Direct product-code env names | 338 |
| Direct product-code env names without a docs/config mention in the scan | 189 |

Occurrences by top-level area:

| Area | Env-related occurrences |
|---|---:|
| `finitecomputer-v2` | 229 |
| `finitechat` | 148 |
| `finite-brain` | 54 |
| `finite-sites` | 34 |
| `devfinity` | 18 |
| `finite-agentd` | 18 |
| `finite-skills` | 14 |
| `finite-search` | 8 |
| `finite-identity` | 5 |

The "undocumented" count is intentionally a heuristic. Some names are platform
or toolchain contracts (`REQUEST_METHOD`, `SDKROOT`, `KUBERNETES_SERVICE_HOST`,
`PATH`, `HOME`, etc.) and should be allowlisted, not product-documented.

## Findings

### P1: There is no authoritative env-var registry

The repo has good local documentation, but no single source of truth for env
names, ownership, defaults, required/optional status, and secret handling.
`infra/README.md` says host READMEs document secret variable names and locations
only, but the runtime code is still the most complete inventory.

Good local patterns already exist:

- `finite-brain/crates/finite-brain-cli/src/environment.rs` collects CLI process
  environment into `CliEnvironment`.
- `finite-sites/crates/finitesitesd/src/lib.rs` keeps several env names as local
  constants near validation.
- `infra/nixos/modules/kata-runner-host.nix` renders shared Runner env for the
  Kata role instead of relying entirely on host-edited files.

Those patterns are not enforced across the monorepo. New `FC_` or `FINITE*`
names can be added in product code without a registry update or CI failure.

### P1: Chat and relay identity are controlled by scattered magic env vars

This is the highest-risk class because chat availability and durable history are
repo-level invariants.

Representative sites:

- `finitecomputer-v2/crates/finite-core/src/chat_runtime.rs:2192` reads
  `FINITE_CHAT_MIRROR_ENABLED`.
- `finitecomputer-v2/crates/finite-core/src/chat_runtime.rs:2207` and nearby
  lines read `FINITE_CHAT_USER_ACCOUNT_ID`, `FINITE_CHAT_USER_DEVICE_ID`,
  `FINITE_CHAT_RUNTIME_ACCOUNT_ID`, and `FINITE_CHAT_RUNTIME_DEVICE_ID`.
- `finitecomputer-v2/crates/finite-core/src/chat_runtime.rs:2404` derives
  runtime identity and display fields from `FINITE_MACHINE_ID`,
  `FC_WORKLOAD_ID`, `MACHINE_ID`, `FINITE_AGENT_ID`, `FINITE_USER_EMAIL`,
  `FINITE_USER_NAME`, `FINITE_USER_ID`, `FINITE_AGENT_NAME`,
  `FINITE_AGENT_PURPOSE`, `FINITE_HERMES_PROFILE`, `FINITE_HERMES_API_PORT`,
  and `FINITE_AGENT_WORKSPACE`.
- `finitecomputer-v2/crates/finite-core/src/relay.rs:2734` and nearby lines
  separately read relay chat identity vars with different defaults.
- `finitecomputer-v2/apps/dashboard/src/lib/finite-relay-client.ts:591` still
  falls back to `FC_RELAY_URL`, while docs and Nix primarily mention
  `FC_RELAY_ADMIN_TOKEN` and `FC_RELAY_HOST_ENDPOINTS_JSON`.
- `finitechat/apps/electron-chat/electron/main.cjs:96` reads desktop/runtime
  knobs such as `FINITECHAT_SERVER_URL`, `FINITECHAT_DASHBOARD_URL`,
  `FINITECHAT_DASHBOARD_PATH`, `FINITECHAT_HOME`, and `FINITECHAT_DEVICE_ID`.

Risk: a host, dashboard container, relay path, or local client can silently fork
chat identity or storage behavior by setting an env var that is not part of an
explicit compatibility contract.

### P1: Runner/runtime env surface is large and only partially checked

`finitecomputer-v2/crates/finite-saas-runner/src/main.rs` has a broad
`FC_RUNNER_*` surface. The central helpers (`required_env`, `optional_env`,
`optional_runtime_environment`, and `optional_runtime_secret_environment`) are
good, but the variable names are still raw strings at the call sites.

Representative sites:

- `finitecomputer-v2/crates/finite-saas-runner/src/main.rs:218` selects
  `FC_RUNNER_CLASS`.
- `finitecomputer-v2/crates/finite-saas-runner/src/main.rs:231` and repeated
  adapter branches read `FC_RUNNER_FINITECHAT_SERVER_URL`.
- `finitecomputer-v2/crates/finite-saas-runner/src/main.rs:235` and repeated
  adapter branches read `FC_RUNNER_AGENT_PICTURE_URL`.
- `finitecomputer-v2/crates/finite-saas-runner/src/main.rs:637` reads
  `FC_RUNNER_RUNTIME_ENV_JSON`.
- `finitecomputer-v2/crates/finite-saas-runner/src/main.rs:645` reads
  `FC_RUNNER_RUNTIME_SECRET_ENV_FILE`.
- `infra/hosts/lat1/systemd/runner.env.example` and
  `infra/nixos/hosts/finite-lat-3/runner.env.example` document the active Kata
  shape, but not every Docker/Apple/Enclavia/Phala/debug knob in code.

Risk: active operator config, historical env examples, Nix-rendered shared env,
and provider-specific code can drift unless env names are checked as a contract.

### P2: Dashboard and control-plane secret roots share ambiguous names

`FC_SECRETS_ROOT` is consumed in more than one context with different defaults:

- `finitecomputer-v2/apps/dashboard/src/lib/runtime-secrets.ts:8` defaults to
  `/fc-secrets`.
- `finitecomputer-v2/crates/finite-core/src/control_plane.rs:112` accepts
  `FC_AGENT_CLUSTER_SECRETS_ROOT` or `FC_SECRETS_ROOT`, defaulting to
  `/var/lib/finitecomputer/agent-cluster/secrets`.

This may be intentional container-vs-host separation, but the shared fallback
name makes that contract implicit.

### P2: Debug, test, and local-dev env vars are not clearly separated

There are many non-production knobs such as `FBRAIN_TEST_*`,
`DEVFINITY_BRAIN_*`, `FINITECHAT_CAPTURE_PATH`,
`FINITECHAT_EXIT_AFTER_CAPTURE`, `FINITECHAT_DISABLE_SINGLE_INSTANCE_LOCK`,
`FINITECHAT_DEBUG_*`, and packaging/build vars such as `FINITECHAT_BUILD_*`.

These should not be mixed with operator runtime config in the same mental
namespace. They need a documented class (`test`, `smoke`, `debug`, `build`,
`operator`) so audit tools can be strict on production variables without making
every fixture variable a blocker.

### P2: Platform/third-party env vars need an allowlist

The scan also found valid external contracts: `WORKOS_*`, `STRIPE_*`,
`RESEND_API_KEY`, `POSTMARK_SERVER_TOKEN`, `OPENROUTER_API_KEY`,
`SEARXNG_*`, `GOOGLE_*`, `KUBERNETES_SERVICE_HOST`, `SDKROOT`,
`DEVELOPER_DIR`, `ANDROID_HOME`, `CARGO_*`, `NODE_ENV`, and similar names.

These should be explicitly classified as external/platform variables so the
Finite-owned registry stays clean and CI does not produce noisy false positives.

## Recommended Fixes

1. Add a machine-readable env registry, for example
   `docs/config/env-vars.toml` or `infra/env-vars.toml`.

   Suggested fields: `name`, `owner`, `class` (`operator`, `runtime`,
   `secret`, `build`, `test`, `debug`, `external`, `platform`), `required`,
   `default`, `source_file`, `consumers`, `secret_value_location`, and
   `compatibility_notes`.

2. Add a CI/static check that extracts env names from Rust, TypeScript,
   Python, shell/systemd/Nix/YAML, and fails for new unregistered
   Finite-owned names. Start strict only for `FC_`, `FINITE*`, `FBRAIN*`,
   `DEVFINITY*`, `HERMES*`, and `RMP_*`; allowlist platform/toolchain names.

3. Introduce typed config modules for the risky surfaces:

   - `ChatRuntimeEnv` / `RelayEnv` for `finite-core` and dashboard relay.
   - `RunnerEnv` for `finite-saas-runner`.
   - `DesktopFiniteChatEnv` for Electron/RMP local client knobs.
   - `DashboardRuntimeEnv` for dashboard server-only config.

   After that, ban direct `process.env.*` / `std::env::var("...")` outside the
   module's config boundary, tests, and small scripts.

4. Close the chat/relay documentation gap first. Decide whether `FC_RELAY_URL`
   is a supported fallback or legacy-only. If supported, document it next to
   `FC_RELAY_ADMIN_TOKEN`, `FC_RELAY_HOST_ENDPOINTS_JSON`, and
   `FC_CHAT_RELAY_TIMEOUT_MS`. If legacy-only, remove the fallback or guard it
   behind a clearly named migration variable.

5. Split or document ambiguous secret roots. Prefer separate names when the same
   string currently means different filesystem boundaries, e.g.
   `FC_DASHBOARD_SECRETS_ROOT` and `FC_AGENT_CLUSTER_SECRETS_ROOT`.

6. Classify non-production variables rather than deleting them. Keep
   smoke/test/debug vars in the registry but mark them non-operator so
   production rollout reviews can filter them out.

## Suggested First PR

1. Add the registry with the active production/operator vars only:
   dashboard, core, identity, finitechat server/hosted device, sites, brain,
   runner, search, and Tinfoil limiter.
2. Add the extractor/check in warn-only mode and commit the generated diff
   report as CI output.
3. Move chat/relay env reads behind typed config structs.
4. Update `infra/nixos/README.md`, `infra/nixos/modules/dashboard.nix`, and the
   runner env examples from the registry output.

That first PR should not try to rename variables. The immediate win is making
new magic vars visible and reviewable.
