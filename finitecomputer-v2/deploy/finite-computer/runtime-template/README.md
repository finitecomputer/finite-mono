# Finite Computer Runtime Template

This directory documents the immutable `/runtime` contract for hosted Hermes
Agent Runtimes. The same runtime contract should be proven under local Docker,
remote Docker, and Phala before a SaaS launch uses it for people.

Provider runners mount per-agent mutable state at `/data`. The OCI image itself
owns `/runtime`.

The `/data` mount contains:

- Finite Chat state at `/data/agent`;
- Hermes state at `/data/agent/hermes-home`;
- workspace state at `/data/workspace`;
- runtime logs, memories, skills, credentials, and generated config under
  those state roots.

The `/runtime` tree is immutable image state. It must not contain provider keys
or user data. Runner-supplied values arrive through container environment
variables or provider-native sealed env files. Generated Hermes config
references those env values; it must not persist raw provider keys in
`config.yaml`.

## Boot Policy

The current first-class runtime image does not implement a separate boot-policy
shim. Normal restart and recover-known-good both restart the same image and let
the Finite Chat owned entrypoint reconcile required config. A stronger recovery
policy should not be reintroduced until it is implemented in the same Docker
image used by local Docker, remote Docker, and Phala.

## Current OCI Runtime Image

`deploy/finite-computer/images/runtime.Dockerfile` is the current first-class
runtime image. It packages:

- Hermes Agent 0.18 in `/runtime/hermes-venv`
- Finite Chat CLI at `/runtime/bin/finitechat`
- Finite Sites CLI at `/runtime/bin/fsite`
- Finite Chat Hermes plugin at `/runtime/hermes-plugin/finite-platform`
- Finite Chat owned runtime entrypoint, health server, and Hermes gateway
  launcher under `/opt`
- `/runtime/healthcheck.sh`

The current image intentionally does not package the legacy `finitec` monolith.
Any future minimal `finitec` must be a narrow Runtime Management Pipe client,
not a compatibility layer for `finitec publish`, `finitec repo`, legacy chat, or
legacy machine operations.

## Template Debt

The runtime image is now the contract. Provider paths that need custom init
logic should change the image entrypoint and prove it through the local Docker,
remote Docker, and Phala ladder instead of adding host-mounted shims.
