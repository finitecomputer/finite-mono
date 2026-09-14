# FIN-68: SimpleX voice-note support across Finite

[Ticket](https://linear.app/finitecomputer/issue/FIN-68/simplex-cant-read-voice-memos)
· [Draft PR](https://github.com/finitecomputer/finite-mono/pull/875)
· Investigation: 2026-09-14

**Finite's managed SimpleX setup can block voice-note downloads from official
Flux file servers.** The reviewed daemon defaults omit that server group.
Hermes needs the downloaded audio before it can transcribe a voice note.

This investigation covers the shared setup for new and existing Agent Runtimes.
r2 was the initial case inspected. Source review shows the broader exposure;
the number of affected profiles remains unmeasured.

The proposed fix is to add the official Flux servers to Finite's managed
SimpleX setup. Keep the existing Hermes transcription settings. The fix still
needs a test with the packaged daemon and a successful live voice-note reply.
No production state has changed.

## What we know

| Evidence | Finding | Limit |
| --- | --- | --- |
| SimpleX v7.0.2 source | The command-line daemon defaults include only the SimpleX operator. Finite's managed startup inherits this list. | Agents using these defaults are exposed to the gap. A fresh-profile test and fleet count remain open. |
| Initial case: r2 database scratch copy; SQLite integrity check passed | Known file servers use `simplex.im`. Two pending notes use `xftp3.simplexonflux.com` and/or `xftp5.simplexonflux.com`. Both have relay approval false and no started transfer. | Stored state does not prove current runtime health. The scratch copy was removed after recording these findings. |
| Actual Hermes adapter methods replayed in isolation | A rejected download produces zero Agent events. An injected completion produces one voice event with a file path. | This does not test a real download or transcription. |
| Hermes configuration and package source | Transcription is enabled by default. The full package includes local voice dependencies. | Source review does not prove live backend or model availability, or successful transcription. |

The normal path is:

```text
Voice note → SimpleX downloads audio → Hermes transcribes audio → Agent replies
```

SimpleX checks the file servers before download. An unknown server requires
approval unless the connection protects the receiver's IP address. This
explains how text can arrive while audio stays blocked. [Approval logic][approval]

The sender chooses where to upload the file. The receiver needs permission to
contact those servers. Adding a server for downloads need not change the
receiver's outgoing server preferences. Profiles with different server or
network settings may already work.

The original ticket mentions xftp6 and says there are no configured file
servers. r2's inspected state differs; the report has not been tied to those
same transfers. Flux has published official server identities, including
xftp6. Its use does not establish a custom setup on the phone. The initial
case also had admin execution timeouts, so live verification remains open.
[Flux presets][flux]

## Fix plan

Complete each step before moving to the next.

1. **Prove the settings change on disposable state.** Use the exact
   Finite-packaged daemon and managed startup path. Reproduce a blocked Flux
   download, then prove that adding the reviewed server identities permits it.
   Completion: the change survives restart, repeat application adds no duplicate
   records, and settings rollback succeeds.
2. **Add one managed setup operation.** Put it in the SimpleX lifecycle helper.
   Use the same operation for new profiles and eligible existing profiles.
   Completion: new profiles receive the settings before their pairing address
   is exposed; existing profiles retain identity, history, server fields,
   disabled entries, and outgoing preferences. Preserve explicit owner policy;
   leave ambiguous or unmanaged profiles for review.
3. **Prove compatibility and recovery.** Run the checks below with recorded
   versions. Completion: each applicable check passes, including reopening
   updated state with the released components after rollback.
4. **Validate affected profiles, then roll out.** Measure which managed
   profiles need the change. After explicit production-repair authorization,
   verify it on selected affected Agent Runtimes, confirming each exact profile.
   Run `scripts/finite-status` before and after changes; add any missing probe
   there. Completion: fresh voice notes download, transcribe, and produce
   normal replies on the selected profiles before a wider rollout.

## Implementation reference

Read this section when implementing the settings operation or repairing a
profile. The API sequence is a candidate; its payload still needs daemon tests.

**Settings API.** Read the full object with `/_servers <userId>`. Add only the
missing reviewed Flux identities, preserving existing fields. Validate with
`/_validate_servers <userId> <updated-json>`, then apply with
`/_servers <userId> <updated-json>`. The `/xftp` shortcut can replace existing
choices. Use the structured API for a preserving update. [Commands][commands]

A disabled custom server can remain known for download approval while excluded
from active selection. Prove that behavior with the packaged daemon before
choosing the final payload. Use full server identities from the pinned presets;
unrelated unknown servers must still require approval. [Server selection][operators]

**State and event ownership.** The Finite Agent Daemon supervises the SimpleX
lifecycle helper. The SimpleX daemon owns its databases and downloaded files
under `/data/agent/simplex` in Agent Home. The helper requests settings changes
through the daemon API. Hermes reads download events, transcribes audio, and
owns its chat sessions. Keep database writes in the daemon and preserve Hermes
configuration and session history. [Lifecycle helper][helper], [connection controls][controls]

Only one WebSocket consumer can safely receive daemon events. A second client
can take events intended for Hermes. Run settings operations with exclusive
control access, and prove that interruption and retry preserve chat service.
Hermes keeps its pending-file map in memory: an ordinary conversational skill
gets no turn for a withheld voice event. [Lifecycle helper][helper], [adapter][adapter]

**Rollback and old notes.** Export the complete pre-change server object and
record inserted row IDs. Rollback restores the old settings and explicitly
deletes only those new rows; omission alone does not delete them. Use a
settings-only rollback, never an old whole chat database that discards later
messages. [Storage API][store]

Test a fresh voice note first. Recover old pending notes separately, using
confirmed transfer IDs and checking for duplicate replies. A gateway restart
can lose the in-memory pending map; a settings change alone does not prove
that old notes will resume.

## Remaining checks

All items below are still open.

- [ ] Fresh and existing profiles: Flux downloads work; identity, contacts,
  history, server fields, and send preferences are preserved.
- [ ] Policy cases: already-configured Flux, custom servers, disabled entries,
  explicit restrictions, and unmanaged profiles retain their intended behavior.
- [ ] Restart and failure: repeat updates, failed validation, interruption, and
  retry cause no duplicate rows, lost history, false readiness, or repeated
  disruption of existing chat.
- [ ] Versions: test the candidate helper with the released daemon and Hermes,
  and test released components reopening updated state after rollback.
- [ ] Regression: text and known-server downloads still work; unrelated unknown
  servers still require approval; Hermes remains the sole event consumer during chat.
- [ ] Recovery: settings rollback passes; old notes can be handled without
  guessing transfer identity or creating duplicate replies.
- [ ] Live results: identify affected managed profiles, confirm the selected
  runtime versions and admin access, and prove download → transcription → reply
  before wider rollout.

## Reviewed sources

These pins describe inspected source, not proof of deployed runtime versions.
Before implementation, check for changes to the relevant code and owners.

- Finite main `1568d5a9`: [managed helper][helper], [connection controls][controls],
  and [SimpleX v7.0.2 package][package].
- SimpleX v7.0.2 (`4df04bdb`): [executable][entry] and [server entry point][server]
  use [terminal defaults][terminal]; [general defaults][general] also include Flux.
- Hermes `29112bef`: [adapter][adapter], [transcription defaults][stt], and
  [gateway transcription path][gateway]. Its [full package][hermes-package]
  includes the voice dependencies declared in [pyproject.toml][dependencies].
  The adapter replay also covered the original checkout pin `3c27eb6`.

[approval]: https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Library/Internal.hs
[flux]: https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Operators/Presets.hs
[commands]: https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Library/Commands.hs
[operators]: https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Operators.hs
[store]: https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Store/Profiles.hs
[helper]: https://github.com/finitecomputer/finite-mono/blob/1568d5a924c8c31a5a0f9e8d47a5379c0900f4c3/finite-agentd/simplex_runtime.py
[controls]: https://github.com/finitecomputer/finite-mono/blob/1568d5a924c8c31a5a0f9e8d47a5379c0900f4c3/finite-agentd/src/connections.rs
[adapter]: https://github.com/NousResearch/hermes-agent/blob/29112bef099274229cadff79cdff7bf7b99c4b77/plugins/platforms/simplex/adapter.py
[package]: https://github.com/finitecomputer/finite-mono/blob/1568d5a924c8c31a5a0f9e8d47a5379c0900f4c3/finitecomputer-v2/deploy/finite-computer/images/simplex-chat.nix
[entry]: https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/apps/simplex-chat/Main.hs
[server]: https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Terminal/Main.hs
[terminal]: https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat/Terminal.hs
[general]: https://github.com/simplex-chat/simplex-chat/blob/4df04bdb3ff94059734ad2da7d2766de6dba2cc7/src/Simplex/Chat.hs
[stt]: https://github.com/NousResearch/hermes-agent/blob/29112bef099274229cadff79cdff7bf7b99c4b77/tools/transcription_tools.py
[gateway]: https://github.com/NousResearch/hermes-agent/blob/29112bef099274229cadff79cdff7bf7b99c4b77/gateway/run.py
[hermes-package]: https://github.com/NousResearch/hermes-agent/blob/29112bef099274229cadff79cdff7bf7b99c4b77/nix/packages.nix
[dependencies]: https://github.com/NousResearch/hermes-agent/blob/29112bef099274229cadff79cdff7bf7b99c4b77/pyproject.toml
