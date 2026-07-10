# Hermes Integration

Finite Chat owns the Hermes platform plugin and the Rust bridge contract that
the plugin speaks. finitecomputer can import the plugin or vendor it into its
runtime image, but it should not fork the transport semantics.

## Bridge Commands

The supported plugin distribution path is:

```text
finitechat hermes --agent-home DIR install
```

The command installs the embedded `finitechat` Hermes plugin and writes a
colocated `finitechat.env` with `FINITECHAT_HOME` and `FINITECHAT_BIN`
defaults (plus `FINITE_HOME` when set at install time, so hosted runtimes pin
the shared Finite identity location). It refuses to install from an Agent
Home that has not been initialized with `finitechat hermes init`. The agent's
account key is the shared Finite identity at
`$FINITE_HOME/identity/identity.json` (else `~/.finite/identity/`) — see
`finitechat auth status` / `finitechat auth import`; no key material lives in
the Agent Home. `--service-url URL` also writes
`FINITECHAT_HERMES_SERVICE_URL` for a supervisor-managed service.

The supervised service entrypoint is:

```text
finitechat hermes --agent-home DIR serve --addr 127.0.0.1:0 --ready-file PATH
```

`serve` currently establishes the Rust-owned loopback process boundary and
exposes `GET /healthz` and `POST /v1/hermes/{action}`. It requires an
initialized Agent Home and reports the agent account, device id, server URL,
bound URL, and process id through the ready file or startup JSON.

When `FINITECHAT_HERMES_SERVICE_URL` or platform `extra.service_url` is set,
the Python plugin posts bridge actions to that service first. Without a
configured URL, the plugin starts `finitechat hermes serve` itself, reads the
ready file, and uses that loopback URL. If a service is unreachable it falls
back to the CLI bridge. Action errors returned by the service are not retried
through the CLI, avoiding duplicate sends.

Hermes home channel is persisted in Agent Home with:

```text
finitechat hermes --agent-home DIR home-channel show
finitechat hermes --agent-home DIR home-channel set --room-id ROOM_ID
finitechat hermes --agent-home DIR home-channel clear
```

The loopback service exposes matching `home-channel-show`,
`home-channel-set`, and `home-channel-clear` actions. The setting is only a
Hermes routing preference; it does not add, remove, or reinterpret Finite Chat
room membership. A room must already be available to the agent before it can
be stored as home.

The plugin calls a Finite Chat CLI/daemon boundary:

```text
finitechat hermes poll --json
finitechat hermes ack --json
finitechat hermes send --json
finitechat hermes edit --json
finitechat hermes recover --json
finitechat hermes activity --json
```

Requests are JSON on stdin. Responses are one JSON object on stdout. The Rust
contract lives in `crates/finitechat-hermes`.

`poll`

- Input: `HermesPollOptionsV1`.
- Output: `HermesPollResponseV1`.
- Syncs room logs, stores decrypted Hermes-ready inbound events in the agent
  home's durable inbox, and returns unacked events for one room or all rooms.
  Events redeliver until `ack`.

`ack`

- Input: `HermesAckRequestV1`.
- Acks `(room_id, seq, message_id)` only after Hermes `handle_message` returns.

`send`

- Input: `HermesSendRequestV1`.
- Appends a durable user-visible reply, tool output, or media message.

`edit`

- Input: `HermesEditRequestV1`.
- Updates a previously sent message. `finalize=true` marks stream completion.

`recover`

- Finalizes locally tracked `running` Hermes messages after a gateway restart
  by appending an explicit recovery edit on the same visible message id.

`activity`

- Input: `HermesActivityRequestV1`.
- Sets or clears non-notifying ephemeral state such as `working`.

## Mapping

Finite Chat room maps to Hermes `source.chat_id`.

Finite Chat conversation/topic maps to Hermes `source.thread_id`.

Hermes outbound `chat_id` is interpreted as `room_id`. Hermes outbound metadata
`thread_id` or `conversation_id` is interpreted as `conversation_id`. Reserved
adapter metadata such as `_finitechat_kind`, `_finitechat_status`, and
`attachments` is consumed by the bridge and not stored as user metadata.

Attachments are typed as `HermesAttachmentV1`. They may contain a local path,
a URL, or a Finite Chat encrypted blob reference. The Python adapter only
passes the reference through. On outbound sends, the resident Rust bridge
validates and reads every local path within the Finite Chat per-file and batch
limits, encrypts/uploads the bytes through the room's pinned blob service, and
replaces the path with the durable blob reference before appending the MLS
message. Invalid paths fail before a message is appended. On inbound delivery,
Finite Chat verifies and materializes encrypted blob references for Hermes.
Materialization completes before the durable Hermes inbox cursor advances.
Connect failures, timeouts, retryable server errors, and truncated downloads
end the inbound stream without delivering or acknowledging that event; the
streaming adapter reconnects and retries the same durable event after the
server heals. There is no attachment polling path.

Failures that cannot heal by downloading the same encrypted reference again
are handled differently. An exact referenced object returning `404` (including
historical loopback references rerouted through the configured Finite Chat
origin), ciphertext hash failure, or AEAD/plaintext verification failure is
delivered as caption text with a short resend instruction. The encrypted blob
reference remains in the raw event for product recovery, its ciphertext URL is
never passed to the model as media, and later messages continue normally.

Typing, thinking, and working indicators are not durable chat messages. The
plugin uses room-scoped `activity` so these states do not create unread counts
or push notifications. Topic-scoped activity is out of scope for v1; topics can
be represented as rooms.

## Test Strategy

CI runs both sides of the boundary:

- `cargo test --workspace` validates the Rust DTOs, limits, invalid data, JSON
  round trips, and room/conversation mapping.
- `python3 -m unittest discover -s tests -p '*test*.py'` validates the Hermes
  plugin without requiring a Hermes checkout.
- `scripts/hermes-agent-media-e2e.sh` installs the real upstream
  `hermes-agent` package with `uvx`, starts a live `finitechat-server`, pairs a
  CLI user through the agent invite/PIN, sends image media through the Finite
  Chat adapter, and asserts transport/media round trips. It installs an echo
  `set_message_handler` callback, so it is adapter transport coverage, not real
  Hermes model behavior.
- `scripts/ios-hermes-agent-media-e2e.sh` repeats that adapter transport/media
  round trip through the iOS Simulator app. It also uses an echo callback and
  must not be cited as proof that the real Hermes gateway answered.
- `scripts/ios-device-hermes-agent-media-e2e.sh` is the physical-phone version
  of the same echo-handler transport test. It requires an already installed
  `computer.finite.finitechat` build, an unlocked/awake paired iPhone, and a
  Mac LAN server URL so the phone talks to the same configured server instead
  of Mac loopback.
- `scripts/hermes-real-gateway-demo.sh` is the repo-local real Hermes runner:
  it starts a local Finite Chat server, initializes a Hermes agent home, loads
  the finitechat plugin into a prepared Hermes checkout, and runs
  `hermes gateway run` without a test echo callback.

That runner is a low-level local debugging aid. It is not the physical-phone
product canary gate. For the hardened local phone -> remote Docker -> Tinfoil
promotion loop, use `docs/hermes-phone-canary-loop.md`.

The plugin tests prove:

- registration exposes the `finite` platform contract;
- `FINITECHAT_HOME` is required, and `FINITECHAT_ROOM_ID` is an optional room
  filter;
- outbound sends preserve room, topic, reply, attachments, and metadata;
- outbound sends infer Hermes tool/status kind and running/complete status when
  Hermes metadata is missing;
- inbound poll events map room to chat and topic to thread, then ack only after
  dispatch succeeds;
- wrong-room events are not dispatched or acked;
- ephemeral activity is used for working state instead of durable status
  messages.

See `docs/oops-i-faked-it-audit.md` for the current line between echo-handler
transport coverage and real Hermes gateway proof.
