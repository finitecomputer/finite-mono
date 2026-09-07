# Private SimpleX connections

Private owner DMs only; groups and typing indicators are deferred. No Hermes
fork or adapter patch. The canonical Hermes pin advances from v2026.8.3 / 0.20.0
to v2026.8.31 / 0.21.0. SimpleX v7.0.2 assets are checksum-pinned through Nix.
The upstream Hermes flake brings a Home Manager input for its own module checks;
Finite does not enable a Home Manager module.

## Product and ownership

Connections → Connect creates a managed identity and renders its contact QR.
The user scans it, sends a message, then explicitly approves the exact pending
request in Connections. Hermes PairingStore owns authorization. Names are not
identity proof; the UI also shows contact ID and request age. Approval collapses
the setup UI. Unknown contacts cannot invoke the agent.

Dashboard commands use the existing authenticated Hosted Web Device → Agent
Platform Channel → agentd path. Core and Runner gain no SimpleX-specific API.
The only new product operations are connect, approve-request, and confirmed reset.
There is no code-entry or pause API. Unsupported runtimes fail rather than
silently substituting a different operation.

agentd applies `gateway.platforms.simplex` through its config-offer journal and
restarts Hermes. This can interrupt an active turn. Its fixed-operation Python
helper bootstraps and supervises the daemon; the daemon alone opens its databases.
Existing user-managed YAML or `.env` configurations are rejected for review.

## Environment and transport

- One daemon per runtime, WebSocket at `127.0.0.1:5225`, outbound public SMP/XFTP.
- The canonical toolchain closure includes the daemon; no runtime downloads.
- For managed connections the gateway wrapper supplies `SIMPLEX_WS_URL`,
  `SIMPLEX_AUTO_ACCEPT=true`, `SIMPLEX_ALLOW_ALL_USERS=false`, empty
  `SIMPLEX_ALLOWED_USERS`, and empty `SIMPLEX_GROUP_ALLOWED`.
- No provider credential is needed for SimpleX itself. Inference retains the
  platform's existing configuration and credentials.
- `FINITE_AGENTD_SIMPLEX_SCRIPT` is a local helper-path override, not browser input.

The released adapter's contact acceptance event/command differs from SimpleX v7.
Bootstrap enables daemon-native `/auto_accept on` before publishing the address;
transport acceptance does not grant agent access. The adapter can also lose a
pairing reply sent before contact readiness. Dashboard pending-request approval
removes delivered codes from the authorization flow.

SimpleX WebSocket clients compete for one event queue. A second diagnostic client
can steal Hermes messages. Therefore only initial bootstrap opens a control
WebSocket. Subsequent status uses cached address, PID, and TCP liveness. TCP
readiness does not prove recipient delivery or inference success. General outbound
send acknowledgement and gateway-outage replay remain upstream limitations.

## Persistence and confirmed disconnect

Agent Home stores `simplex/identity_chat.db`, `simplex/identity_agent.db`, their
live WAL/SHM files, attachments, and `simplex/address.json`. Hermes Home stores
pairing records, `state.db`, session transcripts, and routing metadata. Ordinary
restart/image replacement retains these. Missing/incomplete identity state fails
closed; startup never silently replaces it.

Disconnect warns before deleting SimpleX identity, contacts, approvals, requests,
rate limits, attachments, and SimpleX Hermes sessions. Phone messages, other
platforms' data, and shared agent memory remain. Reconnect requires new pairing.

The reset operation disables managed config, writes `simplex-reset.json`, and
restarts Hermes. Before starting the new gateway, the wrapper waits for the daemon
to stop. Cleanup cannot unlink a live daemon database or race the old gateway's
pairing writers. Both legacy/current pairing layouts are cleaned so migration
cannot resurrect grants. Only SimpleX keys are removed from shared rate limits.
Pinned SessionDB APIs delete SimpleX sessions and their delegate children; routing
rows and the legacy JSON mirror are cleared without removing other platforms.

The reset journal records transcript IDs before deletion. A partial filesystem
failure can therefore retry after database rows are gone. Pending reset blocks
new addresses and approvals; failed cleanup leaves SimpleX disabled but allows
other gateway platforms to start. Success is reported only after cleanup ends.
No archive or remote phone wipe is implied.

## Development and qualification

Use the existing `just dev web-design` for UI fixture work and `just dev up` /
`just dev saas-smoke` for the canonical local platform. This PR adds no native
runtime mode or feature-specific dashboard backend. The experimental native phone
trial was removed after validating the UX; its success is not production control
path proof.

- `just computer simplex-test`: offline control, exact-request authorization, and
  reset tests using pinned upstream stores. Includes cross-platform isolation,
  legacy metadata, live-daemon refusal, and interrupted transcript cleanup.
- `just computer simplex-smoke`: opt-in public-relay test with two disposable
  daemons and the released adapter; no inference or user contacts.
- PR CI checks Python formatting/lint, control tests, and Linux daemon packaging,
  alongside the existing Rust, dashboard, and Hermes bridge suites.
- Manual native trial validated QR → pending approval → private conversation and
  confirmed reset. Dashboard screenshots document that UI, not platform auth.

Before rollout: qualify the canonical Runtime image, full owner-authorized
Agent Platform Channel flow, live unauthorized-contact denial, voice/attachments,
and restore of the complete Recovery Set onto an empty target. Hermes upgrade
coverage and persistence migration/downgrade require separate review; the adapter
regression suite alone is not proof of upstream compatibility. Do not assume an
old Hermes image can read state migrated by the new release.

## Hermes upgrade contract review

The exact pinned upgrade changes schema 25 → 26 and private gateway behavior.
These are qualification targets, not confirmed breaking regressions:

| Surface Finite depends on | Existing evidence | Gap before rollout |
| --- | --- | --- |
| `state.db`, sessions, routing, JSON mirror | Candidate-store reset tests; same-image durable-home smoke | Old-written home → candidate → resume; establish rollback using pre-upgrade recovery copy unless downgrade is proven |
| Private adapter lifecycle and inbox settlement | Adapter regression scenarios plus pinned busy/clarification tests | Real-base cancellation, failed delivery/retry, reconstruction, and exactly one durable settlement |
| Session turn leases and restart | Pinned busy-session test | Candidate lease contention must not acknowledge rejected input or strand the Finite inbox after restart |
| Requester identity and terminal tools | Pinned ContextVar/plugin-hook tests | Model-backed terminal turn and restart using existing Finite Private/custom-provider config |
| Config, plugins, other connections | Reconciler tests and pinned imports | Existing YAML/env interpretation, runtime plugin discovery, Telegram approval CLI outcomes, Google setup |
| Media and full image | Opt-in media E2E and dispatch-only runtime smoke | Execute the existing canonical-image media/durability/recovery lanes on the candidate |

The 19 adapter regression scenarios use stand-in gateway/base classes. Running
them with candidate Python is not upstream compatibility proof. The five tests
in `finitechat/tests/hermes/test_pinned_hermes_sender_context.py` use real pinned
Hermes and passed locally after the walkback, but do not exercise full startup
or old-version persisted state. New turn-lease admission has a default five-second
wait; Finite's separate durable inbox lease makes rejection/settlement a contract
to test explicitly. Schema additions appearing additive do not prove downgrade.

Relevant upstream changes: `hermes_state_common.py`, `hermes_state_schema.py`,
`gateway/session.py`, `gateway/turn_lease.py`, `gateway/platforms/base.py`,
`gateway/run.py`, and terminal/provider config code. Retain the upgrade, but keep
this PR draft until its required qualification scope is resolved.

## References

- https://hermes-agent.nousresearch.com/docs/user-guide/messaging/simplex
- https://github.com/NousResearch/hermes-agent/releases/tag/v2026.8.31
- https://github.com/NousResearch/hermes-agent/blob/v2026.8.31/plugins/platforms/simplex/adapter.py
- https://github.com/simplex-chat/simplex-chat/blob/v7.0.2/apps/simplex-chat/Server.hs
