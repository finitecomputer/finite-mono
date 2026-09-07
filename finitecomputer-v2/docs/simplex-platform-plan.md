# Private SimpleX connections

Status: implemented on codex/simplex-platform; local qualification is partial.
No production rollout. Groups and typing/live-message streaming are deferred.
No Hermes fork, plugin override, or adapter patches.

## Product flow

Connections → SimpleX → Connect bootstraps a private per-runtime identity and
shows a locally rendered QR plus copy/open links. The user connects in SimpleX,
sends a message, then enters the eight-character Hermes pairing code in
Connections. Scanning the QR alone does not authorize agent use. Other contacts
remain denied; groups are disabled. Disconnect pauses the integration and keeps
identity, contacts, attachments, and pairing records. Reconnect reuses them.

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
  adapter. Pairing approval delegates to its existing PairingStore API. The CLI returns
  exit zero even for invalid codes, so the helper checks the API result instead.
- simplex-chat: exclusively opens/writes its databases. One daemon per Runtime,
  local-only WebSocket at 127.0.0.1:5225, outbound public SMP/XFTP relays.
- The lifecycle helper bootstraps an unexposed identity before enabling Hermes,
  saves the address atomically, then supervises the daemon independently. The
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
- Passed: eight control tests exercising real local WebSocket correlation,
  command rejection, stalled sockets, both address response shapes, and upstream
  PairingStore approval. Invalid codes fail; a valid code grants only the matching
  SimpleX contact, with a different contact and platform remaining unapproved.
- Passed: real managed daemon bootstrap and two restarts preserve both identity
  databases and the cached address; repeated address retrieval is idempotent.
- Passed: 270 dashboard tests, typecheck, scoped lint, and production build.
  Tests include typed actions and QR/link validation. Static render review at
  desktop and 390px phone widths found no horizontal overflow; this is not an
  interactive full-stack browser test.
- Passed: scripts/simplex-smoke runs the exact packaged unmodified Hermes adapter
  against two throwaway real SimpleX daemons through public relays. It proves
  numeric-contact inbound/reply delivery, without inference or real users.
- Passed: all 19 existing Finite Chat bridge regression scenarios against the
  candidate Hermes. This does not qualify every unrelated upstream platform.
- Required before shipping: local full-stack owner pairing and phone text/voice
  round trip, unauthorized contact denial at the live gateway, empty-target
  restoration of the complete Recovery Set, and the canonical Linux image build.
- Linux packaging/image verification is assigned to CI, per the user. It does
  not block the native phone trial. The retired pika-build machine is not a
  prerequisite for this feature.


Upstream limitations remain: text sends return success after socket write,
attachment sends accept any correlated response as success, automatic read
receipts are absent, and messages arriving during gateway outages are not proven
replayed. Standalone `hermes send` can open a competing socket; do not recommend
it as a live diagnostic. Fixes should land upstream, not as a Finite adapter fork.

## Native local phone test

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
run `scripts/simplex-local approve CODE` with the code received in SimpleX.
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
