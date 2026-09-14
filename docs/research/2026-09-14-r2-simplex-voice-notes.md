# FIN-68: SimpleX voice-note investigation and proposed fix (2026-09-14)

Tracking: [FIN-68](https://linear.app/finitecomputer/issue/FIN-68/simplex-cant-read-voice-memos).
Status: investigation and proposed implementation plan. No runtime fix or
production repair is included. Synthetic daemon validation and live end-to-end
acceptance remain open.

## Finding

The evidence points to the download approval step as the r2 blocker:

```text
SimpleX XFTP attachment
  -> daemon receives encrypted file description
  -> Hermes sends /freceive <fileId>
  -> daemon either downloads, or rejects unknown XFTP relays
  -> Hermes emits a VOICE event with filePath
  -> shared Hermes gateway runs default local STT
  -> agent turn
```

The reviewed SimpleX daemon source (the `v7.0.2` source commit used by Finite,
`4df04bd`) verifies the boundary. `receiveViaCompleteFD` compares every XFTP
server in the file description against the user’s known XFTP servers. With
`approved_relays=false`, an unknown relay produces `CEFileNotApproved` and no
`xftpReceiveFile` call; a known set, IP-protected relay, or explicit
`approved_relays=true` proceeds to the file-transfer agent
([source](https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Library/Internal.hs)).
The stored `user_approved_relays` bit is persisted per received file
([source](https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Store/Files.hs)).

The pinned Hermes adapter does support voice input. It recognizes a SimpleX
`msgContent.type == "voice"`, waits when the attachment has no `filePath`, and
on `rcvFileComplete` replays the item with the completed path as a `VOICE`
event ([adapter](https://github.com/NousResearch/hermes-agent/blob/29112bef099274229cadff79cdff7bf7b99c4b77/plugins/platforms/simplex/adapter.py)).
During the investigation, an isolated replay against the original checkout
Hermes pin (`3c27eb6`) and reviewed Finite main Hermes pin (`29112be`) reproduced the important distinction: `fileNotApproved` delivered zero agent
events; a successful completion delivered one VOICE event. This does not prove
production end-to-end success.

Hermes’s default transcription is already the expected fallback. Its STT
loader treats absent `stt.enabled` as enabled, defaults `stt.provider` to
`local`, and defaults the local model to `base`
([source](https://github.com/NousResearch/hermes-agent/blob/29112bef099274229cadff79cdff7bf7b99c4b77/tools/transcription_tools.py)).
The gateway calls the configured transcriber and then an installed-local
fallback on failure ([source](https://github.com/NousResearch/hermes-agent/blob/29112bef099274229cadff79cdff7bf7b99c4b77/gateway/run.py)).
The reviewed full Hermes package includes its `voice` extra, which declares
`faster-whisper` ([package](https://github.com/NousResearch/hermes-agent/blob/29112bef099274229cadff79cdff7bf7b99c4b77/nix/packages.nix),
[dependency](https://github.com/NousResearch/hermes-agent/blob/29112bef099274229cadff79cdff7bf7b99c4b77/pyproject.toml)).
Therefore an absent `stt:` block in r2 is not, by itself, the problem.

## Shared default and scope

The missing Flux relays need not be an r2-specific setup mistake. SimpleX
v7.0.2's executable passes `terminalChatConfig` to its CLI/server entry point.
That configuration replaces the general preset list with only the SimpleX
operator. The WebSocket server receives the same configuration. By contrast,
the general `defaultChatConfig` includes both SimpleX and Flux operators
([executable](https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/apps/simplex-chat/Main.hs),
[terminal defaults](https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Terminal.hs),
[server entry point](https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Terminal/Main.hs),
[general defaults](https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat.hs)).

Finite's managed helper inherits these daemon defaults; it does not add Flux
relays during bootstrap. Source inspection therefore predicts the same gap in
fresh profiles using this path. This is not a fleet-wide measurement or a
completed fresh-profile test. Existing profiles, explicit per-file approval,
and IP-protected connections can differ. An attachment using only known relays
does not hit this particular gate. Text chat can continue while attachments
are blocked.

The sending client selects where to upload the attachment; the receiver must
be permitted to contact the servers in its file description. The two clients
do not need identical sending preferences. Making Flux known for downloads
does not require choosing Flux for r2's outgoing files. The approval gate
protects the receiver's IP address when contacting unfamiliar file servers.
No deliberate Finite policy restricting XFTP to one operator was found in the
reviewed startup path; the reason for upstream's narrower CLI defaults is
not established here.

### Corrections to the original ticket

FIN-68 preserves a bot report mentioning `xftp6.simplexonflux.com`, no configured
XFTP servers, and an unavailable relay fingerprint. Treat that as reported
context, not verified r2 state:

- Flux file servers, including xftp6, have published identities in SimpleX's
  [official presets](https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Operators/Presets.hs).
  Their use does not establish a custom-server configuration on the phone.
- The inspected r2 state has SimpleX-operated XFTP servers. Its two inspected
  pending notes reference xftp3/xftp5, not the ticket's xftp6. The evidence
  does not establish that these are the same transfer or runtime as the
  original report.
- Asking the sender to change servers may avoid the symptom, but the proposed
  product fix belongs in Finite's managed receiver setup.

## r2 evidence

A scratch copy passed SQLite quick_check. Its `protocol_servers` contained only
`simplex.im` XFTP servers; `server_operators` contained only the SimpleX operator.
Pending file 1 used `xftp3.simplexonflux.com` and `xftp5.simplexonflux.com`;
pending file 2 used `xftp3.simplexonflux.com`. Their public relay identities
matched the official presets. Both were still new and had
`user_approved_relays=false`, with no agent transfer ID. This is consistent with
the daemon’s approval gate and explains why Hermes never receives a file path.
The temporary database copy was removed after retaining only sanitized relay/status evidence.
This does not establish that the live daemon is currently running: container
`exec` and inspection timed out, so the observed status is stale and no restart
or production mutation was performed.

## Candidate configuration intervention

Subject to the validation gates below, make the Flux XFTP servers known to
the *same SimpleX profile used by Hermes*, preserving existing server records
and their enabled/disabled state. Use the daemon’s structured API:

1. Read the complete current operator/server object with `/_servers <userId>`.
2. Edit only the XFTP server list to contain the official Flux preset records,
   retaining existing fields for any server that must remain known.
3. Run `/_validate_servers <userId> <updated-json>`.
4. After validation passes, apply `/_servers <userId> <updated-json>` and retry
   a new voice note. These are proposed steps, not an executed repair.

`/_servers` and `/_validate_servers` are real parser/API commands, while `/xftp
<server>` is a convenience replacement ([parser](https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Library/Commands.hs)).
The `/xftp` path disables existing preset servers and, when an operator is
present, drops the old custom list before adding the supplied records
([source](https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Library/Commands.hs)).
That is why `_servers` is safer for preserving configuration.

The structured update is an upsert/delete operation: stored rows are updated,
new rows inserted, and only rows explicitly marked `deleted` are removed
([source](https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Store/Profiles.hs)).
A disabled custom server remains in the known-server set because
`agentServerCfgs` preserves it with `enabled=false`; it is known for relay
approval while remaining unavailable for active use
([source](https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Operators.hs)).
This preserves the distinction between “known for accepting an attachment” and
“selected for sending/active connections.”

`/freceive <fileId> approved_relays=on` remains a per-file approval option,
not a persistent fix. Old pending messages may require explicit recovery after
a gateway restart because the adapter's pending map is in memory. Test a fresh
voice note to assess automatic handling.

## Current Finite ownership

The original investigation checkout was `d1401538` (September 2). The reviewed
Finite main commit is
`1568d5a924c8c31a5a0f9e8d47a5379c0900f4c3` (September 14), with Hermes pin
`29112bef099274229cadff79cdff7bf7b99c4b77`. Main packages SimpleX v7.0.2,
whose tag resolves to the exact SimpleX source reviewed above
([package pin](https://github.com/finitecomputer/finite-mono/blob/1568d5a924c8c31a5a0f9e8d47a5379c0900f4c3/finitecomputer-v2/deploy/finite-computer/images/simplex-chat.nix)). Branch state
is not evidence that a particular runtime has been upgraded.

In that main revision, `finite-agentd` supervises `/opt/simplex_runtime.py`, which
runs one daemon at localhost port 5225 and keeps its identity, server state,
and files under `/data/agent/simplex`. Hermes connects to that daemon; Finite
preserves the agent's durable Hermes configuration. The helper does not pass
an XFTP server override at startup. Dashboard controls support connecting,
approving contacts, and disconnecting; they expose no file-relay setting.
See the immutable [Finite SimpleX plan](https://github.com/finitecomputer/finite-mono/blob/1568d5a924c8c31a5a0f9e8d47a5379c0900f4c3/finitecomputer-v2/docs/simplex-platform-plan.md)
and [runtime helper](https://github.com/finitecomputer/finite-mono/blob/1568d5a924c8c31a5a0f9e8d47a5379c0900f4c3/finite-agentd/simplex_runtime.py).

A setup/repair skill could document or perform this bounded configuration task.
An ordinary conversational skill cannot run in response to a voice event that
the adapter has withheld. Also, SimpleX WebSocket clients compete for one event
queue: a second diagnostic connection can consume Hermes's events. Apply any
admin configuration through controlled exclusive access, not a concurrent
background skill socket. This is a small configuration intervention but still
needs a production maintenance boundary.

## Proposed implementation sequence

1. **Prove the configuration change on disposable state.** Launch the exact
   Finite-packaged SimpleX binary through the managed bootstrap path. Record
   its server object, reproduce the Flux rejection, and exercise the validated
   additive update. Prefer making the reviewed official Flux identities known
   without enabling them for outgoing selection. Prove restart persistence and
   idempotence before selecting the final API payload. An AST adapter replay
   does not substitute for this daemon test.
2. **Choose one managed configuration owner.** Put the bounded initialization
   operation in the managed SimpleX lifecycle helper. Source the reviewed relay
   identities from the pinned upstream presets and document their update
   ownership. No hostname-only trust, arbitrary relay approval, or second
   independent policy in Hermes or a conversational skill. Keep the daemon as
   the sole database writer.
3. **Cover fresh and retained profiles separately.** For new profiles, validate
   configuration before exposing the pairing address. For existing profiles,
   read and preserve the full server object, operator fields, custom entries,
   enabled/disabled state, and sending preferences. Skip profiles already
   satisfying the intended policy. Do not silently override an explicit owner
   policy or assume unmanaged profiles belong to the managed helper; ambiguous
   state requires review. Reuse one configuration operation for both paths.
4. **Respect event ownership and failure recovery.** Perform control operations
   only while Hermes is not consuming the daemon event queue. Design an
   interrupted-update/retry boundary; failed validation must not publish an
   address as chat-ready or repeatedly interrupt existing chat. The helper,
   daemon, and gateway coordination still needs implementation review. This
   draft does not authorize a fleet migration.
5. **Validate a scoped production repair after authorization.** Once disposable
   proof and settings rollback pass, identify the exact r2 runtime/profile and
   establish a maintenance window. Run `scripts/finite-status` before and after
   the change; add any missing sanitized incident probe there. Verify a fresh
   voice note through download, transcription, and reply. Handle old pending
   notes as a separate recovery operation with exact transfer identity and
   duplicate-delivery checks. Then assess affected managed profiles for rollout.

### State ownership to preserve

| State or boundary | Writer / authority | Reader / consumer | Required invariant |
| --- | --- | --- | --- |
| SimpleX identity, contacts, server configuration, transfer records under Agent Home | SimpleX daemon; helper requests changes through validated commands | Same daemon on receive and restart | Preserve identity, history, owner policy, and existing server fields |
| Managed enablement and lifecycle | finite-agentd connection controls and lifecycle helper | Helper and Hermes gateway wrapper | Configuration readiness precedes exposing a new connection; unmanaged profiles stay outside this operation |
| Received audio file | SimpleX file-transfer path | Hermes adapter, then shared transcription | Completed file path reaches the normal voice event path |
| Pending attachment map | Hermes SimpleX adapter in memory | Adapter completion handler | Restart does not imply old notes will automatically replay |
| Transcription settings and chat sessions | Existing Hermes configuration and gateway/session APIs | Transcriber and Agent Runtime | Relay repair does not rewrite STT, routing, or session history |

See the pinned [lifecycle helper](https://github.com/finitecomputer/finite-mono/blob/1568d5a924c8c31a5a0f9e8d47a5379c0900f4c3/finite-agentd/simplex_runtime.py),
[agentd connection controls](https://github.com/finitecomputer/finite-mono/blob/1568d5a924c8c31a5a0f9e8d47a5379c0900f4c3/finite-agentd/src/connections.rs),
and adapter sources above. Re-trace these owners against the implementation
base before changing runtime behavior.

### Acceptance and compatibility gates

All unchecked items are required future proof, not completed tests.

- [x] Trace the pinned daemon approval gate and the managed startup defaults.
- [x] Inspect consistent r2 scratch state and retain only sanitized findings.
- [x] Replay actual adapter methods from both reviewed Hermes pins: rejection
  yields zero events; injected completion yields one VOICE event with a path.
- [ ] Fresh profile: reproduce the default mismatch with the packaged daemon;
  candidate bootstrap downloads a Flux attachment before normal transcription.
- [ ] Existing profile: upgrade synthetic SimpleX-only state without changing
  identity, contacts, text history, existing server fields, or send preferences.
- [ ] Policy variants: already configured Flux, custom servers, disabled
  entries, explicit restrictions, and unmanaged profiles are handled without
  loss or unwanted policy changes.
- [ ] Repeat/restart/failure: no duplicate server rows; settings persist; a
  failed validation or interrupted update recovers without chat-state loss.
- [ ] Mixed versions: candidate helper with released daemon/Hermes and retained
  state works; released helper/daemon can reopen candidate-updated state after
  rollback. Record exact versions; an all-candidate test is insufficient.
- [ ] Regression: known-relay downloads and text chat still work; an unrelated
  unknown relay remains subject to approval; no competing event consumer is
  introduced.
- [ ] Recovery: settings-only rollback succeeds on synthetic state; old pending
  notes are recovered separately without duplicate turns or navigation-based
  selection of durable state.
- [ ] Live r2: exact runtime identity and binary confirmed; admin path healthy;
  fresh voice note downloads, transcribes through existing Hermes defaults,
  and produces a normal reply. Fleet applicability is measured before rollout.

## Proof boundary and rollback

Source inspection establishes the relay policy; isolated execution verifies adapter behavior and default STT selection,
and the r2 scratch-state shape. It does not prove that the live r2 daemon has
the expected binary, that its WebSocket is healthy, that the model is cached,
or that a production voice note has completed. The acceptance test is one new
voice note yielding a normal agent reply, with evidence of successful XFTP completion, a file path in the VOICE event,
and successful transcription. The daemon configuration change has not been
executed or validated against a disposable daemon yet.

Before any admin write, export the pre-change `/_servers` JSON and retain it as
the settings backup. Record the IDs of newly inserted rows. Rollback restores
the original settings and explicitly deletes only those new rows through the
validated API: omission alone does not delete an upserted row. Do not restore
an old whole chat database, which would discard intervening messages. No
production configuration, transfer approval, or process state was changed.
