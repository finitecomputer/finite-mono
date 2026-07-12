# Finite Computer Runtime Template

This directory documents the immutable `/runtime` contract for hosted Hermes
Agent Runtimes. The same runtime contract should be proven under local Docker,
Kata, and Phala before a SaaS launch uses it for people.

Provider runners mount per-agent mutable state at `/data`. The OCI image itself
owns `/runtime`.

The `/data` mount contains:

- Finite Chat state at `/data/agent`;
- Hermes state at `/data/agent/hermes-home`;
- the installed Finite-managed skills baseline at
  `/data/agent/managed-skills/finite/current`;
- user-owned skills at `/data/agent/hermes-home/skills`;
- workspace state at `/data/workspace`;
- runtime logs, memories, credentials, and generated config under
  those state roots.

The opt-in Restic mechanism snapshots the complete `FINITE_AGENT_STATE_ROOT`
(default `/data`), not only `/data/agent`. It refuses to run when the Agent
Home or workspace resolves outside that root, so a successful snapshot cannot
silently omit `/data/workspace`. This closes the old path-selection gap only;
application-consistent quiescing, independent Recovery Authority envelopes,
off-provider storage, and an empty-target restore drill remain required before
the mechanism is a product Recovery Snapshot.

The `/runtime` tree is immutable image state. It must not contain provider keys
or user data. Runner-supplied values arrive through container environment
variables or provider-native sealed env files. Generated Hermes config
references those env values; it must not persist raw provider keys in
`config.yaml`.

## Boot Policy

The current first-class runtime image boots `finite-agentd`. It runs the
image-owned preparation hook once, then independently supervises the resident
Finite Chat sidecar, health endpoint, and Hermes. Normal restart and
recover-known-good still restart the same image. The preparation hook seeds
Hermes defaults only when `config.yaml` is absent. On later boots it repairs
only the Finite Chat plugin/platform settings and the installed managed-skills
path. Hermes/user-owned model settings and Telegram or other platform config
survive unchanged. The merge is fail-closed and atomically replaces the file;
an invalid durable config is never replaced with image defaults.

The Finite Chat sidecar owns the one held, reconnecting server sync stream and
writes separate durable local inboxes for Hermes chat delivery and typed
`finite-agentd` commands. Restarting or breaking Hermes therefore does not
break the Agent Platform Channel. `finite-agentd` may restart local processes;
it never starts, stops, replaces, or destroys the Agent Runtime itself.

## Current OCI Runtime Image

`deploy/finite-computer/images/runtime.Dockerfile` is the current first-class
runtime image. It packages:

- Hermes Agent 0.18.2 in `/runtime/hermes-venv`
- pinned Google Workspace Python clients in the same Hermes virtualenv
- Finite Chat CLI at `/runtime/bin/finitechat`
- Finite Agent Daemon at `/runtime/bin/finite-agentd`
- Finite Sites CLI at `/runtime/bin/fsite`
- Finite Brain CLI at `/runtime/bin/fbrain`
- the local `finite skills sync` utility at `/runtime/bin/finite`
- Finite Chat Hermes plugin at `/runtime/hermes-plugin/finitechat`
- the release's Finite Skills baseline at `/runtime/finite-skills`
- Finite Chat owned runtime entrypoint, health server, and Hermes gateway
  launcher under `/opt`
- `/runtime/healthcheck.sh`

The OCI healthcheck performs one bounded loopback request to `/healthz`. That
endpoint is supervised by `finite-agentd` and returns success only when the
durable identity is usable, the Finite Chat bridge is healthy, and the
`finitechat`, health, and Hermes processes are all running. Binary, dependency,
skill, and version validation happens once while building the image; it is not
repeated every 30 seconds as part of runtime liveness.

On a genuinely fresh Agent Home, the gateway launcher atomically seeds the
image baseline into the durable managed-skills directory and exposes that path
to Hermes with `skills.external_dirs`. A restart or image upgrade never
overwrites it. Existing agents update at their own pace by running
`finite skills sync`, which atomically adopts only this image's tested bundle;
Core, Runner, and the Runtime Management Pipe do not push skills.
`$HERMES_HOME/skills` remains user-owned. See the
[Finite Skills runtime contract](../../../../finite-skills/docs/runtime-delivery-contract.md).
The Google Workspace skill carries its scope contract inside that same bundle,
so `finite skills sync` cannot update the OAuth scripts without their matching
scope list. The image preinstalls the exact client-library versions; normal
skill use never performs a runtime `pip install`.

The current image intentionally does not package the legacy `finitec` monolith.
The `finite` utility still owns only explicit local workflows such as
`finite skills sync`. `finite-agentd` is a separate, typed agent-local boundary:
it accepts no arbitrary shell, argv, YAML, paths, or environment edits; it
delegates Finite Sites, Brain, and Chat behavior to their independent tools;
and it has no Runner/provider lifecycle capability.

## Template Debt

The runtime image is now the contract. Provider paths that need custom init
logic should change the image entrypoint and prove it through the local Docker,
Kata, and Phala ladder instead of adding host-mounted shims.

The current image still runs Hermes as root. `finite skills sync` replaces the
managed directory without restarting Hermes; newly added or removed slash
command names need the existing Hermes `/reload-skills` command. Do not add a
Runner-mounted checkout, automatic fleet updater, Runtime Management
capability, or provider-specific sync daemon.
