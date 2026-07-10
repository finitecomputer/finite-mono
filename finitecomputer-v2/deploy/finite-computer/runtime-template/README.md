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

The `/runtime` tree is immutable image state. It must not contain provider keys
or user data. Runner-supplied values arrive through container environment
variables or provider-native sealed env files. Generated Hermes config
references those env values; it must not persist raw provider keys in
`config.yaml`.

## Boot Policy

The current first-class runtime image does not implement a separate boot-policy
shim. Normal restart and recover-known-good both restart the same image and let
the Finite Chat owned entrypoint reconcile required config. The launcher seeds
Hermes defaults only when `config.yaml` is absent. On later boots it repairs
only the Finite Chat plugin/platform settings and the installed managed-skills
path. Hermes/user-owned model settings and Telegram or other platform config
must survive unchanged. The merge is fail-closed and atomically replaces the
file; an invalid durable config is never replaced with image defaults. A
stronger recovery policy should not be reintroduced until it is implemented in
the same Docker image used by local Docker, Kata, and Phala.

## Current OCI Runtime Image

`deploy/finite-computer/images/runtime.Dockerfile` is the current first-class
runtime image. It packages:

- Hermes Agent 0.18.2 in `/runtime/hermes-venv`
- pinned Google Workspace Python clients in the same Hermes virtualenv
- Finite Chat CLI at `/runtime/bin/finitechat`
- Finite Sites CLI at `/runtime/bin/fsite`
- Finite Brain CLI at `/runtime/bin/fbrain`
- the local `finite skills sync` utility at `/runtime/bin/finite`
- Finite Chat Hermes plugin at `/runtime/hermes-plugin/finitechat`
- the release's Finite Skills baseline at `/runtime/finite-skills`
- Finite Chat owned runtime entrypoint, health server, and Hermes gateway
  launcher under `/opt`
- `/runtime/healthcheck.sh`

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
The narrow `finite` utility owns only explicit local workflows. It is not a
generic control-plane client or a compatibility layer for legacy machine
operations.

## Template Debt

The runtime image is now the contract. Provider paths that need custom init
logic should change the image entrypoint and prove it through the local Docker,
Kata, and Phala ladder instead of adding host-mounted shims.

The current image still runs Hermes as root. `finite skills sync` replaces the
managed directory without restarting Hermes; newly added or removed slash
command names need the existing Hermes `/reload-skills` command. Do not add a
Runner-mounted checkout, automatic fleet updater, Runtime Management
capability, or provider-specific sync daemon.
