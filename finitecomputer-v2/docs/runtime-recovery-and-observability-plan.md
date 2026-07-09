# Runtime Recovery And Observability Plan

Status: planning contract for the hosted-agent recovery ladder.

## Problem Statement

finitecomputer-v2 hosts agents inside confidential runtimes, with Phala CVMs as
the intended production provider. The normal path should maximize privacy: user
chat, runtime state, Finite Private credentials, and workspace data stay inside
the TEE-backed runtime and durable mount.

That privacy goal must not turn Finite Chat into the only lifeline for user
data. If chat, invite admission, Hermes configuration, plugin loading, or client
protocol compatibility breaks, the user must still have a clear path to recover
the agent, create a fresh topic with the same agent, roll back to a known-good
runtime, or export their work. Data availability takes priority over perfect
confidentiality when the alternative is a bricked user workspace.

## Product Principles

- **Privacy by default, recovery by explicit consent.** Normal operation keeps
  logs minimal and secrets/data inside the confidential runtime. Break-glass
  operations may weaken privacy, but they must be explicit, audited, and
  understandable.
- **Runtime image owns mounted-state repair.** Core and the dashboard request
  lifecycle operations; they do not directly edit `/data`, Hermes config, chat
  stores, plugin directories, or workspace files.
- **Recover does not replace identity.** Chat recovery can regenerate generated
  config and restart services, but it must preserve agent cryptographic
  identity, room/topic membership, message state, Hermes memory, workspace
  files, user-installed tools, and skills.
- **A new topic is not a new pairing.** Multiple Finite Chat rooms should be
  understood as multiple topics with the same hosted agent. The first shipped
  UI may display them as multiple groups, but creating a new topic should not
  force the whole invite/pairing ceremony again for an already authorized user.
- **Fail closed before mutating encrypted state.** Incompatible app/server/image
  versions should stop before consuming a single-use invite, admitting a join,
  or touching MLS state.

## Phala Constraints And Opportunities

Phala changes the operating model in ways this plan must account for:

- CVM persistent volumes survive restart and upgrade. All user-recoverable state
  must live under the mounted `/data` contract, not the ephemeral image layer.
- Encrypted environment variables are encrypted client-side and decrypted only
  inside the CVM, but updates are full replacement and restart the CVM. Recovery
  paths should avoid env-var churn unless they truly need a secret rotation.
- Docker/application logs may be public unless Public Logs are disabled or a
  private log viewer is configured. Startup diagnostics must be redacted, and
  verbose ongoing logs must not be the production default.
- CVM lifecycle operations, logs, image updates, resource updates, SSH/copy, and
  attestation are available through Phala dashboard/API/CLI surfaces. These are
  useful recovery tools, but any use that exposes user data must be treated as a
  break-glass action.

## Phase 1: Startup Report And Health Evidence

Goal: make the runtime explain its startup state without requiring public logs
or chat access.

Runtime boot should write a redacted, machine-readable report under durable
state, for example `/data/agent/startup-report.json`, and expose a safe subset
through health/status endpoints. The report should include:

- runtime image digest and source manifest;
- finitecomputer-v2, finitechat, Hermes, `fsite`, and `fbrain` versions or
  source refs;
- mounted state roots present and writable: `/data`, `/data/agent`,
  `/data/agent/hermes-home`, `/data/workspace`;
- identity presence and public identifiers only, never private keys;
- Finite Private configuration mode and key-present boolean, never raw keys;
- generated Hermes config path and whether raw secrets were avoided;
- active Hermes plugin name and enabled platform name;
- startup invite lifecycle state without raw invite token;
- home-topic/home-channel state for both Finite Chat and Hermes gateway;
- last successful boot phase and last recover-chat boot phase;
- redacted error code and remediation hint when startup fails.

Acceptance criteria:

- Local Docker canary captures the startup report as an artifact.
- Dashboard/Core can distinguish at least: runtime down, runtime up but chat not
  ready, chat ready, invite consumed pending admission, paired/admitted.
- The report contains no invite URL/token, API key, plaintext message, workspace
  listing, or user file content.

## Phase 2: Plugin And Generated Config Audit

Goal: prevent another image/app/plugin drift or plugin rename collision from
silently breaking hosted chat.

At boot and during recover-chat, the runtime should audit:

- canonical Hermes plugin directory exists at `plugins/finitechat`;
- `plugin.yaml` name is `finitechat`;
- Hermes `plugins.enabled` contains `finitechat`;
- Hermes `gateway.platforms.finitechat.enabled` is true;
- generated platform `extra.home` points at `/data/agent`;
- generated platform `home_channel` matches the current hosted topic when set;
- legacy plugin directories/config entries such as `finite-platform` and
  ambiguous `finite` are absent or inactive;
- Finite Chat CLI, embedded plugin, and runtime image source refs match the
  release manifest.

The audit should be fail-closed for release incompatibility and warning-level
for harmless stale files that are not active. Hosted startup must not suppress
these facts; they should appear in the startup report and redacted startup logs.

Acceptance criteria:

- A dirty-volume test with legacy `finite-platform`/`finite` config proves the
  runtime either repairs generated config or reports a precise blocked state.
- Runtime image tests assert the generated config uses `finitechat`, not legacy
  plugin names.
- Release/deploy docs use the canonical `finitechat` name everywhere.

## Phase 3: Real Recover-Chat Boot Mode

Goal: make `recover_known_good_chat_runtime` stronger than provider restart
while preserving user state.

Core already has the lifecycle kind and routes. The next version should pass a
runtime-owned boot/recover intent to the image. On boot, the image should run an
idempotent repair before starting Hermes:

Allowed mutations:

- reinstall the canonical Finite Chat Hermes plugin from immutable `/runtime`;
- regenerate generated Hermes config from current env and durable state;
- repair `FINITECHAT_HOME_CHANNEL` and `gateway.platforms.finitechat.home_channel`;
- repair Finite Chat Hermes home-channel/home-topic metadata;
- clear stale service-ready files, pid files, transient health caches, and
  incomplete generated config;
- mint a fresh startup invite only when the prior invite is expired or provably
  unconsumed and unjoinable.

Forbidden mutations:

- replacing or regenerating agent identity;
- deleting or rewriting `client.sqlite3` or room membership state;
- deleting Hermes memory/session state;
- deleting workspace files;
- silently creating a new agent identity or new isolated workspace;
- downgrading Finite Private fail-closed behavior to OpenRouter or another
  fallback when Finite Private is required.

Acceptance criteria:

- Recover-chat preserves the same public agent identity and workspace checksum
  across local Docker, remote Docker, and Phala.
- Recover-chat fixes intentionally broken generated Hermes config/plugin names.
- Recover-chat emits a startup report proving what it changed and what it
  refused to change.
- If identity/client-store corruption is detected, recover-chat stops with an
  explicit escalation state instead of minting a replacement identity.

## Phase 4: New Topic With Same Agent

Goal: give users a legacy-`/new`-like escape hatch without re-pairing the agent.

Finite Chat rooms should be treated as topics with the same agent. The first UI
may expose them as separate groups, but the product model should allow a future
grouped view: one agent, many topics.

For an already authorized user/device, "new topic" should not require the full
single-use invite ceremony. It should be a normal authenticated command/action
inside the existing agent relationship:

- create a fresh topic room for the same agent and authorized user set;
- preserve old topics and their encrypted history;
- optionally make the new topic the Hermes home topic;
- carry the same agent identity, workspace, tools, and memory unless the user
  explicitly chooses a fresh-memory mode;
- make the new topic visible in the app as another chat/group until grouped UI
  exists.

Escalation rule: if the user is not already authorized for this agent, use the
normal invite/pairing flow. If the current topic is corrupted but the app can
still authenticate the user to the agent, use the new-topic path. If agent
identity or client-store state is corrupted, escalate to recover-chat or rescue.

Acceptance criteria:

- Protocol/API design distinguishes new topic, new room invite, and new agent
  identity.
- Native app and CLI can create a second topic with the same agent without
  consuming another onboarding invite.
- Dashboard does not become a second chat configuration plane; it may expose an
  emergency "create fresh topic" control only by delegating to runtime/chat
  contracts.

## Phase 5: Known-Good Image Rollback

Goal: recover from bad runtime releases without touching user data.

Core should know the current runtime artifact and one or more known-good
artifacts that are compatible with the mounted state schema. Rollback should
redeploy the selected image against the same `/data` volume and encrypted env
set, then run the same startup report and chat readiness checks.

Rollback must be gated by a compatibility manifest:

- runtime image digest;
- finitechat source ref and protocol compatibility range;
- Hermes version;
- state schema version;
- required env names;
- supported provider paths: local Docker, remote Docker, Phala;
- migration and rollback safety notes.

Acceptance criteria:

- Local Docker canary proves rollback from a deliberately bad image to a
  known-good image while preserving identity, workspace, and chat state.
- Phala canary proves image update/restart with the same durable volume.
- Core refuses rollback to an artifact whose compatibility manifest says it
  cannot read the mounted state schema.

## Phase 6: Break-Glass Export And Rescue

Goal: make data rescue possible even when it weakens confidentiality.

Break-glass operations are for cases where normal chat, recover-chat, and
rollback do not restore access. They require explicit user or admin consent,
clear privacy warnings, and audit events.

Rescue modes to design:

1. **Read-only rescue image.** Boot a minimal image against the same `/data`,
   keep Hermes/chat stopped, and expose/download selected data such as
   `/data/workspace` and redacted diagnostics.
2. **Operator-assisted Phala rescue.** Use Phala SSH/copy/log facilities only
   after consent, with an audit trail and a visible privacy downgrade warning.
3. **User-owned export bundle.** Package workspace and selected agent metadata
   encrypted to a user recovery key or downloaded directly by the user.

Break-glass must never be implicit. The UI copy should say that this may expose
data to operators or non-TEE tooling depending on the selected rescue mode.

Acceptance criteria:

- Core records who initiated break-glass, when, why, which runtime, and which
  privacy downgrade was accepted.
- Rescue image can export workspace data from a mounted volume without starting
  chat services.
- No break-glass path deletes mounted state as part of diagnosis.

## Phase 7: Logs Policy

Goal: have enough visibility to debug startup without leaking user data.

Canaries and early internal deployments may enable redacted startup logs. User
production should default to either private logs or startup-only logs that go
quiet after readiness.

Allowed startup log facts:

- boot phase names;
- image/source/plugin versions;
- state-root presence booleans;
- public identity prefixes or hashes;
- key-present booleans;
- redacted invite lifecycle state;
- plugin/config audit pass/fail;
- recover-chat actions taken.

Forbidden log facts:

- invite URLs or tokens;
- raw API keys or env values;
- plaintext chat messages;
- user file contents or workspace listings;
- private identity material;
- long-running Hermes conversation logs in public mode.

Acceptance criteria:

- Phala deployment docs explicitly choose public, private, or startup-only logs
  for each environment.
- Startup logs and startup report are checked in the same local/Phala canary.
- A redaction test fails if a raw invite URL/token or Finite Private key appears
  in startup logs.

## Phase 8: User Recovery Material

Goal: handle identity/client-store corruption without silently replacing the
agent or losing the user's work.

Recover-chat must stop before identity replacement. A separate recovery-material
story should cover:

- what user-owned key or platform-secure storage protects recovery material;
- whether iCloud Keychain or manual export is acceptable for native devices;
- how additional devices receive recovery access;
- what can be restored into a new runtime if the old agent identity is lost;
- how to attach an old workspace to a new agent identity with clear loss of old
  encrypted chat continuity;
- how to revoke or retire a compromised agent identity server-side where
  possible.

Acceptance criteria:

- Product copy distinguishes recover chat, new topic, rollback, export, and new
  identity.
- There is a tested path to preserve workspace data even when chat identity is
  unrecoverable.
- There is no silent identity reset in normal restart or recover-chat.

## Evaluation Ladder

Each phase must climb the same provider ladder before it becomes a user-facing
promise:

1. unit tests for contracts and redaction;
2. local Docker runtime image canary;
3. remote Docker or equivalent non-laptop canary;
4. Phala CVM with durable volume and real lifecycle operation;
5. dashboard/Core operation wired to the proven runtime behavior;
6. docs and support runbooks updated with the exact evidence users/operators
   should collect.

Do not claim a recovery level in dashboard copy until that level has passed the
Phala rung with the same runtime image and state-root contract.
