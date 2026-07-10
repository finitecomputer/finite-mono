# Runtime Management Contract v1

Status: accepted boundary; the protocol is not fully wired yet.

## Decision

The Runtime Management Pipe (RMP) is a narrow, provider-neutral telemetry
channel from an Agent Runtime to Core. In v1 it reports only generic runtime
health and the release that is actually running. It does not carry product
features or lifecycle commands.

This is the wall that keeps product work from turning the Runtime image into a
remote-controlled appliance:

- Dashboard and Core own account and Project workflows.
- Runner owns generic compute lifecycle.
- Finite Chat owns chat delivery and Device state.
- Finite Sites, Finite Brain, and connection integrations own their protocols.
- RMP observes runtime health and release identity only.

## Wire Contract

The runtime sends one versioned `RuntimeObservation` shape containing:

- runtime id, boot id, monotonic sequence, and observation time;
- lifecycle phase and `ready`, `degraded`, or `unavailable` health;
- observed Finite Product Release id and Runtime image digest;
- component names and versions drawn from that release manifest; and
- bounded health-check states and redacted error codes.

Core may acknowledge the latest accepted sequence. An acknowledgement is not a
command. Unknown fields, arbitrary JSON, and component names outside the
observed release manifest fail closed.

The runtime authenticates with a runtime-scoped credential delivered through
the Runner's generic secret-injection path. It holds an outbound connection,
reconnects with bounded backoff, and sends a fresh complete observation after a
disconnect. Core is never polled for desired state. A Core or RMP outage must
not stop chat, tools, or agent work inside an otherwise healthy Runtime.

## Explicit Exclusions

RMP v1 has no inbound runtime requests and no:

- shell, command argv, process control, filesystem access, or arbitrary relay;
- restart, stop, replace, provider, volume, or networking operation;
- Google, Telegram, Brain, Sites, chat, or skills command or feature status;
- product credential handoff, OAuth payload, chat content, user data, or key
  material;
- skill source, desired revision, sync request, reload request, or artifact
  transport; or
- Recovery Snapshot, key backup, restore, export, or purge orchestration.

A feature request that appears to need RMP must first prove why its owning
service, a stable product API, a runtime-local CLI, or Finite Chat cannot own
it. Convenience is not enough to expand this contract.

## First-Slice Proof

The same protocol tests must pass unchanged for local Docker, Kata, and Phala:

- a runtime reports its boot, readiness, image digest, and component versions;
- reconnect produces one current observation without endpoint polling;
- stale sequences from an old boot cannot replace a newer observation;
- non-telemetry payloads and unredacted secrets are rejected; and
- Core failure does not interrupt the running agent.

## Current Gaps

Core contains generic heartbeat and relay-era scaffolding, but the Runtime does
not yet have this closed telemetry client and the current credential is not
injected through every Runner before launch. Those routes are not permission to
add feature commands. The first slice may use provider health while this pipe
is completed, but it must not claim RMP conformance until the tests above pass.
