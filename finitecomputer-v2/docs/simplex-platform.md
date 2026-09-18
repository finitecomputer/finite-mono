# Private SimpleX connections

Private owner DMs and agent-created owner-only topic groups. Multi-user groups
and typing indicators are unsupported. Hermes and SimpleX assets are pinned
through Nix; topics use platform/tool registration without a Hermes source patch.
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
Dashboard operations are connect, approve-request, and confirmed reset.
There is no code-entry or pause API. Unsupported runtimes fail rather than
silently substituting a different operation.

agentd applies `gateway.platforms.simplex` through its config-offer journal and
restarts Hermes. This can interrupt an active turn. Its fixed-operation Python
helper bootstraps and supervises the daemon; the daemon alone opens its databases.
Existing user-managed YAML or `.env` configurations are rejected for review.

## Owner-only topics

Ask in the approved SimpleX DM to create a named topic. The registered
`simplex_create_topic` tool resolves the requester from Hermes turn context and
requires exactly one approved numeric SimpleX contact. It never accepts an
invitee ID from the model. Requests from other transports/groups are rejected.
The agent creates the group and invites that contact as a regular member; the
agent is the sole admin. Repeating a topic name reuses the existing group.

The Finite plugin extends the released adapter only for managed SimpleX. It uses
that adapter's existing socket and checks current membership before admission
and outbound text/media. Checks use the local daemon rather than cached
membership; this deliberately adds a local command round-trip per send. Expected
policy refusals return unsuccessful `SendResult`s at public text/media send
boundaries. Routing is enabled before issuing an invitation so an immediate
first message is not lost while the tool awaits the invitation response.
The group member ID must map to the paired contact ID;
Hermes then applies ordinary pairing authorization to that verified contact.
Unregistered groups are ignored. Additional members, owner promotion, or leaving
permanently disable private processing for that topic. Membership read failures
also deny processing. There is no multi-user conversion path.

A blocked topic does not require a connection reset. The create tool explains
that the owner can explicitly request a fresh replacement in their approved DM.
Only then may it set `replace_blocked=true`: the old group stays disabled with
its history intact, and a new group gets a separate session. The registry keeps
the old blocked row under `retired:<group-id>` alongside the new topic row.
Retries reuse the replacement, including reconciliation after a lost creation
response. Other topics, approvals and Home are untouched. The tool never
unblocks an old group or copies its transcript into the replacement.

The plugin alone writes `/data/agent/simplex/topics.json` atomically. It records
creation intent before issuing the command and reconciles uncertain creation
using a unique temporary group description, removed before the invitation.
Unresolved/ambiguous creation refuses to create another group and requires repair;
it never guesses from a title. Reset deletes this registry with the SimpleX state.
Corrupt registry state disables topics while preserving private DM availability.
The canonical wrapper runs `hermes gateway run --replace` under the same durable
`HERMES_HOME`. Pinned Hermes acquires its process-held `gateway.lock` before
starting platform adapters, and releases it after adapter teardown (or on process
exit). Replacement startup must win that lock before loading this adapter;
`--replace` does not bypass it. A two-process contract test verifies exclusion
and release after abrupt exit. No second registry lock is needed on this path.
Pointing independent Hermes profiles at the same SimpleX identity is unsupported.

Each group retains its own Hermes session; creation does not write Home settings.
Shared agent memory/tools remain shared. The stock adapter on rollback ignores
these groups because managed `SIMPLEX_GROUP_ALLOWED` remains empty; it leaves the
additive topic registry and daemon databases intact. Upgrading again restores
recorded group routing. No existing approval or session schema is rewritten.

## Environment and transport

- One daemon per runtime, WebSocket at `127.0.0.1:5225`, outbound public SMP/XFTP.
- The canonical toolchain closure includes the daemon; no runtime downloads.
- For managed connections the gateway wrapper supplies `SIMPLEX_WS_URL`,
  `SIMPLEX_AUTO_ACCEPT=true`, `SIMPLEX_ALLOW_ALL_USERS=false`, empty
  `SIMPLEX_ALLOWED_USERS`, and empty `SIMPLEX_GROUP_ALLOWED`. The managed plugin
  enables exact group IDs from its durable topic registry in memory.
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
live WAL/SHM files, attachments, `simplex/address.json`, and `simplex/topics.json`.
Hermes Home stores pairing records, `state.db`, session transcripts, and routing
metadata. Ordinary
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

- `just computer simplex-test`: offline control, exact-request authorization, and
  reset tests using pinned upstream stores. Includes cross-platform isolation,
  legacy metadata, live-daemon refusal, and interrupted transcript cleanup.
- `just computer simplex-smoke`: opt-in public-relay test with three disposable
  daemons: private DM, registered topic tool, invitation, group reply, restart,
  retry, stock-adapter rollback/re-enable, third-member rejection, and blocked
  topic replacement. No inference or user contacts. Real phone/model approval
  and canonical-image restart remain canary checks. Topic policy, discovery,
  session, and gateway-lock contracts run in the Hermes suite.
- PR CI checks Python formatting/lint, control tests, and Linux daemon packaging,
  alongside the existing Rust, dashboard, and Hermes bridge suites.
