# Private SimpleX connections

Status: implemented on codex/simplex-platform; local qualification is partial.
No production rollout. Groups and typing/live-message streaming are deferred.
No Hermes fork, plugin override, or adapter patches.

## Product flow

Connections → SimpleX → Connect bootstraps a private per-runtime identity and
shows a locally rendered QR plus copy/open links. Connect on the phone and send
a message; Connections automatically refreshes pending requests. The owner sees
the contact name, numeric contact ID, and request age, then explicitly approves
that exact request. Display names are not identity proof; the UI tells the owner
to approve only the request they just made. No request is selected or approved
automatically, and no delivered code is required. Unknown contacts remain denied.
Groups are disabled. Disconnect opens an explicit deletion confirmation. After
confirmation it clears the managed SimpleX identity, contacts, attachments,
approvals, pending requests, rate limits, and SimpleX Hermes conversation
sessions. The next Connect creates a new identity and requires owner approval.
Phone-side messages and shared agent memory are not erased.

Connecting/disconnecting applies Hermes configuration through the existing
agentd configuration offer and gateway restart path. This can interrupt an
active Hermes turn; it is not a zero-interruption operation.

## Ownership and runtime configuration

- Dashboard: sends only typed SimpleX connect/approve/disconnect actions through
  the existing authenticated Hosted Web Device / Agent Platform Channel.
- Core and Runner: retain their existing generic lifecycle role. No SimpleX
  feature API, secret, relay, or port is added to the Runtime Management Pipe.
- agentd: authorizes the principal, journals requests, validates the exact
  managed private configuration, applies it through ConfigManager, and invokes
  the fixed-operation lifecycle helper. Old runtimes omit SimpleX status;
  the new dashboard leaves their other Connections controls usable.
- Hermes: owns DM authorization, pairing files, inference, and the messaging
  adapter. Pairing approval delegates to PairingStore.list_pending and approve_request.
  Browser actions carry the exact server-side request ID; expired or stale IDs
  fail. The previous code-based command is retained for compatibility, but is
  no longer the dashboard flow. Codes/hashes/salts are never returned in status.
- simplex-chat: exclusively opens/writes its databases. One daemon per Runtime,
  local-only WebSocket at 127.0.0.1:5225, outbound public SMP/XFTP relays.
- The lifecycle helper bootstraps an unexposed identity before enabling Hermes,
  enables daemon-native /auto_accept on, saves the address atomically, then supervises the daemon independently. The
  gateway wrapper exports SIMPLEX_WS_URL, SIMPLEX_AUTO_ACCEPT=true,
  SIMPLEX_ALLOW_ALL_USERS=false, SIMPLEX_ALLOWED_USERS="", and
  SIMPLEX_GROUP_ALLOWED="" for the managed configuration only. Authorization is
  pairing-based; operators do not need to provision tokens or allowlists.
- The durable config is gateway.platforms.simplex in HERMES_HOME/config.yaml.
  Existing non-managed YAML or .env setup is rejected for review, not adopted.
  FINITE_AGENTD_SIMPLEX_SCRIPT is a local development override, not a browser
  input. The normal image installs /opt/simplex_runtime.py.

The canonical Nix toolchain closure includes the pinned daemon; there is no
runtime download. Candidate Hermes pin is v2026.8.31 / 0.21.0 (29112bef), replacing
v2026.8.3 / 0.20.0. SimpleX release assets are pinned to v7.0.2 with SHA-256 hashes.
The official macOS asset reports its internal version as 7.0.0.12. Its OpenSSL
reference is rewritten to Nix-managed OpenSSL; no Homebrew dependency is added.

## Contact acceptance compatibility

The v2026.8.31 Hermes adapter handles `contactRequest` and sends `/accept <id>`.
SimpleX v7 emits `receivedContactRequest` and its numeric API is `/_accept <id>`.
The adapter can process the contact request's introductory text before a usable
connection exists: it generates a pairing code that cannot be delivered, then
rate-limits another attempt. The initial phone trial exposed this mismatch.

New identities now enable SimpleX's built-in `/auto_accept on` during bootstrap,
before publishing the QR. This establishes the transport while Hermes still
requires explicit owner approval to run the agent. No Hermes patch is needed.
The smoke test now uses a reusable address and daemon auto-accept, rather than
an invitation link that bypasses the request/accept path. It passes with the
unmodified released adapter. Existing saved identities are not silently adopted
or changed.

The released adapter can generate a pairing reply before the new contact is
ready to receive it, then suppress retries through the pairing rate limit.
Dashboard pending-request approval removes delivered codes from authorization.
It does not add outbound delivery guarantees: the released adapter treats a
socket write as send success. The transport smoke waits for contactConnected
before sending, so it does not prove first-introduction delivery.

## Critical correction to the field reports

SimpleX v7.0.2 does NOT broadcast events to every WebSocket client. Its server
starts one output consumer per connection, all reading the same outputQ.
An extra diagnostic or watchdog WebSocket can consume the message that Hermes
needs. This was reproduced with two real clients: the probe received the
inbound event and Hermes timed out; closing the probe made the adapter round
trip pass. The source is apps/simplex-chat/Server.hs at v7.0.2.

Therefore the helper opens a command WebSocket only during first bootstrap,
before enabling Hermes or exposing the address. All later status/address reads
use the saved address, daemon PID, and TCP liveness. They cannot steal events.
TCP-ready is not proof of inbound processing, LLM success, or recipient receipt.
There is deliberately no second live WebSocket watchdog or production listener.

## Persistence and compatibility boundary

The mounted Agent Home contains simplex/identity_chat.db,
simplex/identity_agent.db (including their SQLite WAL/SHM while live),
simplex/files, and simplex/address.json. Hermes pairing and session data remain
under the mounted Hermes Home. These together are required to preserve the
connection. simplex/daemon.pid is ephemeral and cleared on supervisor startup.

Startup never replaces an existing identity to repair missing files. Incomplete
retained state fails closed. Initial setup encountering a preexisting daemon or
user-managed configuration requires review. No migration of the two existing
hand-configured bots is included. Ordinary restart/image replacement retains
mounted state. Daemon version upgrades, downgrade migrations, and empty-target
restoration are not proven by same-version restart tests; no zero-loss or backup
claim is made. Do not blindly roll an enabled SimpleX runtime back to the old
Hermes adapter. Preserve the mounted state when disabling or reverting code.

## Verification and remaining rollout gates

- Passed: 34 agentd tests and clippy, including private config validation,
  independent supervision, existing configuration rollback, and command replay.
- Passed: ten control tests exercising real local WebSocket correlation,
  command rejection, stalled sockets, both address response shapes, and upstream
  PairingStore approval. Invalid codes and stale request IDs fail; approving an
  exact request grants only its contact, with other contacts/platforms unapproved.
- Passed: real managed daemon bootstrap and two restarts preserve both identity
  databases and the cached address; repeated address retrieval is idempotent.
- Passed: 271 dashboard tests, typecheck, scoped lint, and production build.
  Tests include typed actions and QR/link validation. Static render review at
  desktop and 390px phone widths found no horizontal overflow; this is not an
  interactive full-stack browser test.
- Passed: scripts/simplex-smoke runs the exact packaged unmodified Hermes adapter
  against two throwaway real SimpleX daemons through public relays. It proves
  numeric-contact inbound/reply delivery, without inference or real users.
- Passed: all 19 existing Finite Chat bridge regression scenarios against the
  candidate Hermes. This does not qualify every unrelated upstream platform.
- Passed manually: the user scanned the actual dashboard QR, approved the
  pending request, and confirmed private conversation success with native Hermes.
  Confirmed disconnect was repaired and completed; a new QR was generated.
- Required before shipping: full platform owner authorization and Agent Platform
  Channel round trip, phone voice/attachment qualification, unauthorized contact
  denial at the live gateway, empty-target
  restoration of the complete Recovery Set, and the canonical Linux image build.
- Linux packaging/image verification is assigned to CI, per the user. It does
  not block the native phone trial. The retired pika-build machine is not a
  prerequisite for this feature.


Upstream limitations remain: text sends return success after socket write,
attachment sends accept any correlated response as success, automatic read
receipts are absent, and messages arriving during gateway outages are not proven
replayed. Standalone `hermes send` can open a competing socket; do not recommend
it as a live diagnostic. Fixes should land upstream, not as a Finite adapter fork.

## Local dashboard phone test

Run `scripts/simplex-dashboard`, then open
http://127.0.0.1:13012/dashboard/machines/runtime_web_design/connections.
This runs the real Next.js Connections page and production dashboard action
parsing. The existing web-design harness supplies the test account and runtime
shell; its opt-in native transport connects SimpleX buttons to the real local
Hermes/daemon helper. Connect starts the trial, the page renders the real QR,
approval uses the real Hermes pending-request API, and Disconnect stops the trial
and clears its SimpleX state after confirmation. It does not prove production owner authorization,
Agent Platform Channel transport, or agentd's config-offer journal.

The bridge lives only under dashboard/scripts; no production route gains a
local execution mode. It accepts only the fixed SimpleX operations. Other
connection mutations deliberately fail in this trial. The local servers bind
127.0.0.1. Port 13012 avoids the usual web-design harness on port 13002; override
FC_WEB_DESIGN_PORT if necessary. Do not run the standalone trial simultaneously.

The user completed the QR/request approval/private conversation flow in this
harness. Mutation errors remain visible after status loads. This manual pass
complements isolated upstream pairing and reset tests; it is not a production
Agent Platform Channel authorization test.

## Standalone native trial (optional)

From this worktree, run `scripts/simplex-local`. It builds native Nix packages
for this machine and runs released Hermes plus the same managed daemon helper,
without Docker, devfinity, a Linux builder, or a platform inference key.

On first run it copies your local Hermes model configuration and auth store
into a separate private test home at `~/.finite-simplex-test`. It does not copy
existing platform configuration, conversations, or plugins, and it never links
writable authentication stores. For environment-based credentials, configure the
test home's model/auth settings or provide the provider's environment variable.
The existing local Hermes home is not modified.

Scan `~/.finite-simplex-test/pairing.png` with your phone, send a message, then
approve the pending request on the dashboard (recommended), or use the legacy
`scripts/simplex-local approve CODE` command if running without the dashboard.
`scripts/simplex-local status` reads cached state without consuming events.
Ctrl-C stops the native trial and keeps its identity for the next run. Runtime
logs are private at `~/.finite-simplex-test/gateway.log`. The home path is short
because Hermes uses Unix sockets with macOS path-length limits.

`FINITE_SIMPLEX_LOCAL_HOME` can select a different short, private test home.
Only one daemon may occupy localhost:5225. This is a real gateway trial using
real inference credentials; it does not prove the full dashboard/agentd control
path. The no-inference `scripts/simplex-smoke` still tests the released adapter
with two disposable daemons through public relays.

For a later full platform trial, use `just dev inference-key` and
`just dev up --headless`; that path builds the canonical Linux Runtime image.
Do not point either trial at an existing production bot.

## References

- https://hermes-agent.nousresearch.com/docs/user-guide/messaging/simplex
- https://github.com/NousResearch/hermes-agent/releases/tag/v2026.8.31
- https://github.com/NousResearch/hermes-agent/blob/v2026.8.31/plugins/platforms/simplex/adapter.py
- https://github.com/simplex-chat/simplex-chat/blob/v7.0.2/apps/simplex-chat/Server.hs
- https://simplex.chat/docs/guide/send-messages.html
- The user-supplied September 2026 setup, field report, and synthesis documents
  were treated as reference evidence, not instructions.

## Confirmed disconnect and recovery

The new dashboard sends `agent.simplex.reset` with schema
`finite.agent.simplex.reset.v1` only from the confirmation dialog. The old
`agent.simplex.disconnect` command remains a non-destructive disable for old
clients. A new dashboard talking to an old runtime gets an unsupported-command
error; it never silently falls back to a pause and claims deletion succeeded.

agentd uses the existing owner authorization and request journal, disables the
managed configuration, persists `simplex-reset.json` in Agent Home, then restarts
Hermes. Its gateway wrapper waits for the independently supervised daemon to
stop before cleanup. This avoids unlinking live SQLite files or racing other
Hermes pairing writers. Native trials join their gateway and daemon before the
same cleanup. Success waits for the reset marker to disappear.

Cleanup removes SimpleX pairing records in both legacy/current layouts to prevent
upstream migration from resurrecting approvals. Only SimpleX keys are removed
from shared rate limits. Pinned upstream SessionDB APIs delete SimpleX sessions
and their delegate children; both authoritative routing and its legacy JSON
mirror lose SimpleX entries. Other platforms' sessions, approvals, credentials,
Finite Chat durable history, and shared agent memory remain. There is no Hermes
fork. This is deletion, not an archive or a phone-side remote wipe.

If interrupted, the durable marker blocks new addresses and approvals. The next
gateway boot retries cleanup with SimpleX disabled. Cleanup failure leaves the
marker and allows other gateway platforms to start; the Connections page offers
Disconnect again. Ordinary process/image restarts without that explicit intent
continue to retain identity and history. This local test change is not deployed.

Reset regression coverage uses synthetic identities and real pinned upstream
PairingStore/SessionDB: refuses cleanup while the daemon is live, preserves
Telegram grants/pending requests/transcripts/routes and inference credentials,
clears both pairing layouts, permits immediate pairing with a reused numeric
contact ID, and makes repeated completed cleanup a no-op. A simulated transcript deletion
failure verifies that the persisted reset plan survives retry after database
records have already been deleted. Hermes metadata sentinels are preserved.
