# FiniteChat in the browser — feasibility spike

Started 2026-09-10 from `origin/main` (`4c924a95`) on `codex/finitechat-wasm-spike`.

## Result and scope

**The actual dashboard chat UI now talks to a real Hermes agent using
browser-owned FiniteChat MLS encryption.** No Hosted Web Device, hosted action
route, plaintext chat proxy, or deterministic echo handler participates.

The existing Rust `FiniteChatDevice` compiles to `wasm32-unknown-unknown` with
`finitechat-mls`, `finitechat-proto`, `finitechat-http`, `finitechat-delivery`,
`finitechat-transport`, `finite-nostr`, and `finitechat-hermes`. The existing
`DashboardShell`, `AgentSidebar`, `HostedWebChat`, and shared `@finite/chat-ui`
render the result. A browser transport provider supplies their usual state and
actions. A small TypeScript projection adapter covers text, tool/status/edit
messages, topics, new chats, selection, rename, and archive; **the full
`finitechat-core` app runtime/projections are not ported yet**.

The counterpart is the real `finitechat hermes serve` daemon and repository-pinned
Hermes gateway (0.21.0), using `glm-5-3-flash` through Finite Private. The supplied
inference key is read from a local file and passed only to the Hermes process.
It is not printed, written to config, passed in argv, or handed to the dashboard,
relay, or browser. Config stores the `${FINITE_PRIVATE_API_KEY}` reference.
The endpoint matches `finitechat/containers/agent/run_hermes_gateway.sh` and
`finitecomputer-v2/docs/service-dependencies.md`.

The **Core/login shortcut remains deliberate**: the local fixture holds a fresh
User Key, and the dashboard hands its nsec to the browser once per document.
The normal dashboard route receives synthetic agent/account navigation metadata.
There is no WorkOS login or Core database migration. The Agent has a separate,
fresh identity and receives only the user's public account ID for both welcome
admission and Hermes' allowed-users list. No existing chat state is opened or migrated.

Acceptance: real dashboard composer/transcript/sidebar; real Hermes inference
and tools; real encrypted browser/Agent round trips; direct signed relay calls;
conversation context and separate chat sessions; no hosted bridge calls.
The persistence extension additionally requires reload and browser-process restart
to reuse one Device/Room/transcript, concurrent tabs to share a single MLS writer,
and interrupted sends to recover without duplicate delivery. Independent browsers
now join the same Agent Room, obtain an encrypted metadata snapshot, and share
future messages. Per-Chat history is fetched on demand from the agent, separately
from live-chat readiness. Migration and mixed-version proof remain out of scope.

## Run it

From this branch's worktree:

```sh
scripts/finitechat-wasm-spike up --key-file /private/tmp/finite-private-test-key.txt
```

Open **http://127.0.0.1:13010/dashboard/machines/wasm-hermes/chat**.
`/wasm-spike` and `/dashboard` redirect to the actual chat screen in local mode.
The local login happens automatically. Chat with Hermes, try a terminal command,
start another chat, and switch back using the actual sidebar. Hermes tools start
in an isolated scratch workspace, not the repository.

Ctrl-C stops only this run's processes. Fresh state lives in `/tmp/fc-wasm-*`,
with a link under `.local-state/wasm-spike/`. Hermes creates Unix sockets beneath
its home; the short path avoids macOS' socket path length limit. Logs and
synthetic state remain for inspection. Each launch refuses occupied ports and
creates a new state directory and fresh identities. **Within a run, the same
browser profile now reopens its encrypted IndexedDB Device, Room, and transcript
across page reloads and browser restarts.** Tabs on the same origin share this
Device using Web Locks. Restarting the entire harness intentionally creates a
new fixture account and therefore a new browser storage namespace.

A different browser/profile or cleared store creates a fresh Device in the same
Agent Room. It receives the Topic/Chat list and future messages; no old transcript
is transferred automatically. Use **Load earlier messages from agent** in each
Chat to request its history. The spike deliberately uses two events per page to
exercise pagination with a few real turns. An nsec alone does not recover MLS
history: the agent supplies decrypted historical records over the new MLS epoch.

```sh
# Build the optimized browser client and native CLI/relay with pinned Nix tools.
scripts/finitechat-wasm-spike build

# Run the real Chromium/inference test, then stop its stack (when up isn't running).
scripts/finitechat-wasm-spike test --key-file /private/tmp/finite-private-test-key.txt

# Reuse already-built artifacts during development.
scripts/finitechat-wasm-spike up --skip-build
```

The test needs an existing Chromium/Chrome or Playwright executable and makes
real Finite Private inference requests. It checks a terminal calculation, a
remembered code, a separate new chat, and return to the prior transcript. A second
test uses a disposable persistent Chromium profile to exercise reload, concurrent
tabs, browser-process restart, delivery/receipt interruption, failed IndexedDB
writes, and an authentic but stale checkpoint. Both use real relay and Hermes
traffic; only browser I/O faults are injected.
No Core, Runner, Hosted Web Device, or devfinity stack is needed. The runner
starts four processes: relay, native FiniteChat daemon, Hermes gateway, Next.

The mode and bootstrap require `FINITECHAT_WASM_SPIKE=1` and refuse production
mode. HTTP listeners bind loopback, CORS permits exactly the dashboard origin,
and fresh random identities are used. Generated WASM/JS, keys, databases, and
logs remain untracked. Attachments/audio and Brain cards are disabled in this
provider because their browser transport isn't implemented. Other product
surfaces (provisioning, connections, Sites, billing, etc.) aren't configured.

## Actual data flow

```text
Disposable Core/login fixture
  -> POST /api/wasm-spike/bootstrap (nsec, relay URL, Agent public identity)
  -> actual dashboard + browser Rust/WASM FiniteChatDevice
       creates its own MLS leaf key and account-signed Device credential
       persists its KeyPackage private state before publishing the public package
       signs a narrow /spike/enroll request to the real agent
       agent adds this exact Device to its canonical Room with MLS Add + Welcome
       browser durably activates Welcome before acknowledging it
       encrypts standard conversation/segment/Hermes application events
  -> browser Fetch directly to FiniteChat relay /events and /sync/group
       relay orders/stores opaque MLS envelopes, knows membership metadata
  -> real native `finitechat hermes serve` + encrypted SQLite client store
       authenticates/decrypts, applies native Core projections, delivers inbox
  -> installed FiniteChat Hermes plugin -> pinned Hermes gateway
       actual model request to Finite Private; actual tool execution
       response/tool/status events through the Agent's FiniteChat daemon
  -> MLS encryption as Agent Principal -> relay -> browser WASM decrypt
  -> BrowserChatProvider -> existing dashboard transcript/sidebars
```

“Direct” removes the Hosted Web Device hop; the usual FiniteChat relay still
provides delivery. This is not peer-to-peer WebRTC. The Agent's loopback Hermes
service retains its ordinary internal adapter. The browser calls only a separate,
feature-gated `/spike/enroll` route for pre-join membership; it cannot fetch chat
content there. Metadata and history use encrypted `/events` and `/sync/group`.
Core custody of nsec means this is not cryptographic operator blindness.

## What blocked compilation, and the spike solution

- SQLite, file locks, blocking Reqwest, native sync workers, and incident file
  diagnostics were compiled unconditionally in the single client source file.
  Native-only items/dependencies now have target gates. `FiniteChatDevice` and
  its MLS implementation remain shared source, not copied or reimplemented.
  Some unused persistence codecs still compile on WASM; the narrowly
  target-scoped dead-code allowance records this unfinished module split.
- OpenMLS needs its `js` feature for browser time/randomness. Transitive
  `getrandom` 0.2 and 0.4 also need their JS backends. Browser entropy comes from
  the browser backend, never fixed seeds or a JavaScript PRNG.
- secp256k1 includes C. Nix's host-wrapped Clang passed Darwin hardening flags
  that wasm32 cannot accept. The WASM shell specifies unwrapped Clang and
  llvm-ar for that target only. No rustup/system dependency installation.
- The default Nix bindgen CLI was 0.2.108 but the original lockfile used
  0.2.117. The already-pinned `nixpkgs-lat3` provides 0.2.121; the new wrapper
  pins that matching Rust dependency and updates its companion locked crates.
  No Nix input was repinned.
- Browser HTTP uses async Reqwest/Fetch and NIP-98 signing from `finite-nostr`.
  The small async wrapper currently mirrors a subset of the existing native
  HTTP adapter's encoding/decoding. Consolidating those codecs is real work.
- Cross-origin browser requests need CORS; the local harness adds it to the
  actual FiniteChat server router, not to a forwarding proxy.
- WASM mutable borrows must not overlap async send/sync. The React adapter
  serializes operations; background polling yields to queued UI work.
- Next's development request URL used `localhost` while the browser origin
  used `127.0.0.1`. The fixture checks the actual Host plus a loopback origin.
  A fresh Next process was required to pick up a route edit in the initial run.

## Evidence and bundle size

The optimized build uses `--release`, `opt-level=s`, LTO, one codegen unit, and
panic abort, then the pinned `wasm-bindgen --target web`. No wasm-opt pass yet.

| Artifact | Raw bytes | gzip (level 9) bytes |
| --- | ---: | ---: |
| WASM | 4,024,665 | 2,019,677 |
| JS loader | 29,893 | 6,546 |
| Total | 4,054,558 | 2,026,223 |

With persistence and agent admission included, that's **3.84 MiB raw / 1.93 MiB gzip for WASM**,
or **1.93 MiB gzip including the loader**. Brotli gives 1,779,938 bytes for WASM
plus 5,684 bytes of JS (**1.70 MiB total**). These are compressed file measurements, not a measurement of the
whole Next dashboard or a promise about server compression configuration.
The initial debug artifact was 10,418,033 bytes raw / 3,458,752 bytes gzip,
plus 28,464 / 6,389 bytes of loader JS.

- Chromium loaded the optimized `.wasm` in the actual dashboard and passed
  three real Hermes round trips: a terminal calculation, memory across turns,
  and a new chat. Switching back displayed the prior transcript. Agent logs
  confirmed `tool terminal completed`, the first chat's reused session, and
  a distinct new session with `history=0`.
- The browser test aborts **every `/api/**` call except key bootstrap**. It
  checks that `/events` targets the relay, carries Nostr authorization, and
  has no plaintext marker even after base64-decoding the ciphertext.
- Unsigned replay returns 401. Altering a ciphertext byte while retaining the
  old authorization returns 401 (HTTP body binding, not a full MLS tamper suite).
- The deterministic peer and bespoke echo UI have been deleted. The running
  relay contains no worker that could synthesize a chat reply.
- Dashboard typecheck, changed-file eslint, production Next build, WASM/native
  wrapper clippy (`-D warnings`) pass. The production build rejects both
  `/wasm-spike` and key bootstrap with 404 even with the spike flag set. The
  latest Next build reports 17 file-tracing warnings through the unchanged
  `workspace-paths.ts`/`fc-dashboard.ts` imports (the earlier build reported 12). The prior native client regression run
  passed 92 tests with one performance benchmark ignored. The persistence
  extension reran those tests and adds portable snapshot/sync entry points;
  native storage and transport behavior remain unchanged.
- The real persistence browser matrix passed (165 seconds): the same Device and
  Room survive page reload, two concurrent tabs, and a new browser process. Two
  tabs converge on both Hermes replies. Killing a tab before delivery or after
  actual relay acceptance retries byte-identical ciphertext; the accepted retry
  returns the original receipt and produces one transcript entry. An aborted
  IndexedDB write publishes nothing and reload restores a sendable Device.
  Rolling back to an older authentic checkpoint triggers native sender-currency
  protection with no new send, Room, or membership Commit. Every dashboard API
  except key bootstrap is blocked throughout the test.
- Focused existing tests passed:
  `hosted_device_chats_with_an_agent_and_restarts_with_the_transcript` and
  `startup_finalizes_durable_device_link_staging_and_exposes_exact_receipt`.

## Browser persistence and crash boundaries

The sole writer is `src/lib/browser-chat-store.ts` in the dashboard. It seals one
versioned checkpoint in IndexedDB (`finitechat-wasm` / `devices`) using AES-GCM
with a fresh nonce, a nonextractable WebCrypto key derived from the supplied nsec,
and the account/relay/Agent scope as authenticated data. The nsec itself is not
stored. This protects an offline database copy; it does not protect against
same-origin malicious JavaScript, which also receives the nsec in this design.

`BrowserFiniteChat` holds an exclusive Web Lock over reading the latest revision,
restoring the WASM client, applying each operation, and committing its writes.
A compare-and-swap revision check catches unexpected writers. Other tabs reload
the current checkpoint before operating. IndexedDB transaction completion with
strict durability is the save boundary. Missing capabilities, unreadable state,
and write failures fail closed; the adapter discards any advanced in-memory MLS
state after an operation fails. There is no localStorage or in-memory fallback.

The checkpoint contains the native `FiniteChatDeviceState` codec (Device signer,
OpenMLS storage, pending Commit, membership and sender-currency state), ordered
sync cursor, decrypted event projection inputs, selected chat, admission journal,
and one pending send with its exact ciphertext, idempotency key and plaintext.
The entire checkpoint, including that plaintext, is sealed before persistence.
This deliberately favors a small implementation over storage/memory efficiency.

| Boundary | Durable state and restart behavior |
| --- | --- |
| Before enrollment | Browser Device and KeyPackage private state saved; retry publishes the same public package and signed admission request |
| Before membership Commit | Agent writer saves pending MLS state and journals the exact Commit/Welcome; file/SQLite atomicity remains a documented gap |
| Before Welcome acknowledgement | Browser saves activated MLS state and admission cursor; retry acknowledges the same Welcome |
| Before message publication | Consumed MLS generation and exact outgoing envelope saved together; no send if storage fails |
| Relay accepts, receipt is lost | Resend saved bytes/idempotency key; reconcile the original receipt before reading own log entries |
| Receipt saved | Own-send high-water mark, local event, and cleared pending send saved together |
| Incoming sync page | Native transition updates MLS, cursor, and event inputs; save together before exposing the page |
| Older checkpoint restored | Native ordered-sync/currency checks persist rewind evidence and refuse sends |

Initial admission is journaled but its full crash matrix has not been exercised
in Chromium. The tested failures cover application sends and storage. The shared
native currency gate is used rather than skipping own messages on replay.

**Contract deviation:** `finitechat/CONTEXT.md` currently specifies no durable
client outbox and synchronous send acceptance. This spike has one durable pending
send to reconcile uncertain acceptance automatically. This is an experiment,
not a change to that native product contract. Before shipping, define pending,
rejected, retry and cancellation semantics, and reconcile the draft with eventual
acceptance; currently a transport failure can retain a draft even if retry later
succeeds. Do not encourage a manual resend as the recovery path.

## Investigation: where old chats come from

**Hosted web restart recovery is primarily a durable Device reopening, not a
fresh browser acquiring old MLS secrets.** In
`finitechat/crates/finitechat-hosted-device/src/lib.rs`, `user_root`,
`chat_data_dir`, and `runtime_for` reopen the same per-user identity and SQLite
store using the stable `hosted-web` Device. All browser sessions are views onto
that server Device. The hosted HTTP restart test above proves the retained
transcript. ADR 0012 (`finitechat/docs/adr/0012-hosted-agent-room-binding.md`)
also requires replaying the exact journaled Room binding/admission artifacts;
ordinary restarts must not create a new binding.

Agent startup in `finitechat/containers/agent/recover_chat_boot.py` reconciles
known-good configuration/home channels and calls `finitechat hermes recover` to
finalize interrupted turns while preserving the existing client store. It is
not a new-browser history recovery endpoint.

**There is already a real encrypted linked-Device history-transfer protocol.**
ADR 0014 (`finitechat/docs/adr/0014-nip-ab-device-pairing.md`) separates account
key transfer from subsequent resumable Device enrollment. In
`finitechat/crates/finitechat-core/src/lib.rs`, `link_device`,
`advance_link_device_bootstrap_export`, and `send_link_device_bootstrap` first
admit the target's distinct MLS Device, then transfer encrypted, chunked history
at fixed per-Room membership fences. The target stages chunks invisibly and
imports a complete validated manifest with an exact receipt;
`accept_link_device_bootstrap` and `finish_link_device_bootstrap` implement that
side. The focused startup test proves a restart can finalize durable staged
history. These source/export and target/import paths live in native Core/store
code that the WASM dashboard does not yet use.

The important limitation is explicit in `accept_link_device_bootstrap`: the
source account must equal the target user's account, the target Device must
match, and the Room must already be joined. An Agent is a different Principal,
so it cannot simply serve as this history source unchanged. Account nsec custody
can skip the key handoff for this experiment; it cannot skip MLS admission or
recover pre-admission ciphertext.

To reuse the current full-history linking path, admit browser B as a new Device
using online browser A (or an existing native user Device), transfer chunks and
manifests, and prove both continue chatting in the same Room. The lighter
Agent-assisted admission option below avoids requiring A online by deferring
history. In either case Core must supply a stable, authorized Room binding
rather than deriving one from a random new browser Device.

When no other user Device is available, choose and prove a separate recovery
source: a client-encrypted history backup, or explicitly authorized Agent-supplied
history. The latter needs a deliberate protocol/policy extension: proof that the
new Device belongs to the user allowed in that Room, bounded export scope,
source provenance, chunk/manifest integrity, atomic import and crash resumption.
Do not relax the same-account importer check and assume that is sufficient.

Do not copy one live MLS checkpoint into two browsers: Web Locks only coordinate
one browser profile/origin. Independent browsers need distinct Devices, with
history transfer, rather than concurrent writers sharing one MLS ratchet.

## Follow-up exploration: Core authorization, admission, and optional history

Explored 2026-09-10 against this spike's source. The recommendations below are
design exploration. The selected future-messages/metadata and per-Chat history
options are now implemented in the spike as described below; the other options
are not implemented, and none changes an accepted production contract.

### The object model makes admission smaller than it first appears

One **Room** is one MLS group and ordered log. Topics are Conversations inside
that Room; resumable Chats are Segments inside Topics. `newChat` in the browser
adapter publishes `conversation_segment_start` without creating an MLS group.
The Hermes adapter routes using Room plus conversation/segment identifiers and
account identity, not a fresh browser Device as the conversation identity.
Finite Computer's Canonical Agent Room is the durable binding for a Project's
human and Agent Principals. Therefore one new Device admission per actual Room
covers future messages across all of that Room's Chats, including Chats created
later. It is not one membership operation per Hermes session. Separate Projects
or genuine Rooms still require separate admission.

### Core can authorize; group membership is a separate transition

`FiniteChatDevice::new` already makes a fresh MLS leaf signer and uses the User
Key to sign `FiniteDeviceCredentialV1`, binding account, Device ID, leaf public
key and validity interval. Handing the browser the nsec already grants it this
authorization power. Core could instead sign that same credential after login,
leaving the leaf private key in the browser, but the current construction API
and account-signed HTTP requests also need adapting before removing the browser
nsec. Keep the existing custody shortcut for the smallest next spike.

A User Key alone is not an existing Room's MLS state. There are two MLS-native
admission mechanisms: a current member creates an Add Commit and encrypted
Welcome, or a new client makes an authorized External Commit using current
GroupInfo and public tree material. Identity authorization, group admission,
and history restoration should have separate completion conditions.
[RFC 9420, membership and external joins](https://www.rfc-editor.org/rfc/rfc9420.html#section-12.4.3),
[RFC 9750, multi-device and application policy](https://www.rfc-editor.org/rfc/rfc9750.html#section-6.7).

| Admission option | Fit and remaining work |
| --- | --- |
| Agent FiniteChat runtime performs normal Add/Welcome | Recommended for this spike. It already holds the Room state and is needed for live chat anyway. Add a deterministic authenticated enrollment operation; no LLM/tool-call decision. Old browsers can remain offline. |
| Existing user Device performs normal Add/Welcome | Closest to the native linking implementation. Requires an online user Device with membership in the relevant Rooms. Its current fanout and history helper is same-account-only. |
| Browser joins using External Commit | Standard MLS option when no existing member should need to be online. FiniteChat currently has no exposed GroupInfo publication/discovery or external-join adapter/relay path; the client discards returned GroupInfo. Requires authorization, current-epoch publication, commit conflict handling and persistence. More work here than Add/Welcome. |
| Retain a hosted user Device for enrollment/history | Reuses much of today's implementation but keeps a server MLS state owner and much of the operational machinery this spike is trying to remove. It is optional, not a prerequisite for browser MLS. |

An Agent can be the ordinary MLS member issuing Add/Welcome. The relay's
`validate_commit_room_membership` accepts active-member commits; its admin
metadata does not gate cross-account additions, as explicitly exercised by
`sqlite_room_admin_metadata_does_not_gate_membership_commits_and_survives_restart`.
The lower-level `prepare_add_member_commit` verifies the target's account-signed
KeyPackage. In contrast, `start_link_fanout` and `accept_link_device_bootstrap`
require the source and target to share an account. Do not confuse those helper
restrictions with an MLS prohibition on Agent-assisted Device admission.

The new enrollment operation still needs its own authority boundary: bind the
user account, exact Room, Device ID, leaf/KeyPackage, expiry and request ID to
Core's authenticated approval (under the current custody assumption an
account-signed grant is possible). The Agent checks the requesting human is
already authorized in that exact Room. A browser is not yet a member, so this
request cannot depend on sending inside that Room. Define a bounded pre-join
request/inbox path or authenticated onboarding endpoint. Existing management
telemetry is not a command channel, and the existing Agent Platform Channel
must not be assumed to solve pre-admission access. This is new plumbing.
No chat plaintext or MLS private state needs to pass through Core.

### Sync choices, independently of admission

| Choice | What the new browser gets | Main cost or limitation |
| --- | --- | --- |
| Future messages plus a small metadata snapshot | Existing Topic/Chat IDs, names and archive state; messages from its admission onward | Smallest useful option. Old transcript bodies stay absent. |
| Agent-served history on demand | Above, then older messages for the Chat the user opens | Recommended follow-on. One-way bounded reads from the Agent's durable FiniteChat store; explicit source authorization, stable IDs/cursors, restart and partial-history semantics. |
| Existing linked-Device full history | Complete retained history from another user Device | Reuses native chunks/manifests and atomic import, but needs browser store porting and an available source; full export currently participates in enrollment completion. |
| Encrypted archive backup | Saved history even when all previous Devices are unavailable | Requires backup writer, coverage/freshness and key-recovery contracts; admission remains separate. A history archive is not a cloned live MLS Device checkpoint. |

A metadata snapshot is necessary for a usable existing Chat list: creation,
rename and archive events may predate the new Device. Today's browser projection
does not create a Chat row merely from a new message carrying an unknown Segment
ID. Replaying old organizational events as new global events could also change
other Devices' active Chat or resurrect stale metadata. Prefer a target-scoped,
versioned snapshot with an explicit Room sequence boundary, then ordinary live
events. Preserve messages arriving during admission/snapshot transfer, and test
concurrent rename/archive/new-Chat operations. Sending the snapshot in the joined
Room keeps it encrypted; existing Devices must not reset their projections when
they see another Device's bootstrap.

“Future only” does not mean the browser never syncs: it still catches up on MLS
Commits and messages that arrived while it was offline after joining. All
Devices consume one authoritative ordered Room log. There is no need for
browser-to-browser database reconciliation for that live traffic. Keep history
backfill additive and separate from the live MLS cursor/ratchet. The Agent's
model context can retain old turns even when the browser intentionally shows no
pre-admission transcript; the UI should make that distinction understandable.

The existing native `link_device` computes completion from both membership
fanout and emitted history manifests. Removing history sends without changing
that completion predicate would leave enrollment pending forever. Introduce
per-Room membership readiness independent of optional history progress; do not
make one unavailable Room or a large transcript block chatting in another Room.
Agent-provided history must come from its durable FiniteChat transcript, not an
LLM reconstruction. It can only supply history it actually retained and was
allowed to see, and it does not replace the production Recovery Set.

### Other MLS implementations

- **Wire:** its current multi-device support page documents new Devices without
  old conversation history, with conversations synchronized going forward.
  Its server documentation also records External Commit support. This validates
  the future-only product choice; its support explanation about per-device
  encryption should not be substituted for an MLS wire-format description.
  [Wire multi-device](https://support.wire.com/hc/en-us/articles/115003858445-Using-Wire-on-multiple-devices),
  [Wire server External Commit support](https://docs.wire.com/v0.0.0/changelog/changelog.html).
- **XMTP:** installations have independent keys. Its history feature requests
  an encrypted archive from an online existing installation, coordinated by a
  user-device MLS sync group and archive server. Its backup API explicitly
  separates restored read-only history from later live group admission. Useful
  precedent for treating admission and archive availability separately, rather
  than copying one writable MLS state between installations.
  [XMTP history sync](https://docs.xmtp.org/chat-apps/list-stream-sync/history-sync),
  [XMTP archive backups](https://docs.xmtp.org/chat-apps/list-stream-sync/archive-backups),
  [XMTP identity](https://docs.xmtp.org/protocol/identity).
- **Marmot:** particularly relevant because it also uses Nostr identities and
  MLS. Its current multi-device document is explicitly a branch draft, proposes
  authorized External Commits with distinct device leaves, and leaves historical
  messages out of scope. This is design precedent, not evidence of a shipped,
  solved multi-device history experience. The old MIP document layout has been
  replaced; use the current feature specification.
  [Marmot multi-device draft](https://github.com/marmot-protocol/marmot/blob/master/features/multi-device.md).

### Smallest next experiment and its acceptance criteria

1. Two independent browser profiles, distinct Device keys, one Core-authorized
   human account and the same canonical Agent Room. Start A, then take A offline
   and enroll B through the Agent's normal Add/Welcome path.
2. B receives a minimal encrypted Topic/Chat snapshot and future messages; no
   old transcript transfer. Opening a known Chat uses its existing Segment ID,
   preserving the real Hermes context.
3. Bring A back. Both see each other's new messages and Chats across reloads.
   Existing pre-join transcript remains on A; there is no replacement Room,
   shared live checkpoint or hosted chat bridge.
4. Interrupt enrollment after Commit acceptance, before Welcome acknowledgement,
   and during concurrent sends. Retry the same enrollment safely; do not create
   duplicate membership. Reject wrong-account, wrong-Room, expired and substituted
   KeyPackage grants. Handle an epoch conflict by bounded sync/reconciliation.
5. Create/rename/archive a Chat during the metadata snapshot and verify no lost
   messages or stale sidebar state. An offline Agent leaves a clear pending
   admission, and partial multi-Room progress does not disable ready Rooms.
6. Only after that works, add paginated per-Chat historical reads with duplicate,
   interruption, live-tail and missing-source-history tests. History failure
   must leave already-working live chat working.

## Independent browsers and agent-sourced history (September 10 extension)

The native CLI enables this experiment only with the `browser-spike` Cargo
feature and the harness's explicit local environment. At startup the agent
creates one canonical `wasm-hermes-room` in the disposable relay. Every browser
owns a different MLS leaf; browser tabs in one profile still share one Device.
The retired browser-owned Room/bootstrap/Commit path has been removed.

The pre-join request binds Room, account, Device ID, KeyPackage ID and hash under
the user's NIP-98 signature, including URL, method, body hash and a 60-second
clock window. The agent admits only the fixture's configured public user account
to that one Room. Its existing Core actor is the sole writer for Add/Commit,
metadata responses, history responses and normal Hermes messages. The agent
never receives the User Key. This stands in for a future authenticated Core
Device grant; it is not a new general-purpose agent HTTP chat API.

After Welcome activation the browser emits `finitechat.browser.request.v1` with
operation `metadata`. The agent replies with `finitechat.browser.response.v1`:
request ID, target Device, sequence boundary, Topic/Chat IDs, titles, active Chat
IDs and archive state. Previews and transcript counts are scrubbed. Responses
are non-notifying encrypted application events; other Devices ignore the target's
snapshot. The browser applies organizational events after the boundary and keeps
all transcript events it could decrypt since admission, including overlap during
snapshot creation. It never replays old ChatStart events into the live group.

History uses those same encrypted event kinds with operation `history`, explicit
Topic/Chat, an exclusive `before_seq`, and page limit. The source reads its own
encrypted `client_app_events` store through the existing SQLite store API, filters
the requested Chat, and returns original event IDs, sequences, timestamps,
senders, message/edit kinds and payloads. No LLM reconstructs history. No web
bridge, dashboard history endpoint, copied MLS state or old browser is involved.
History records are **agent-attested copies**, not newly authenticated original
MLS ciphertext. The browser accepts only responses from the Agent Principal,
addressed to its Device and matching a request it actually sent.

Imported history is projected additively and persisted inside the encrypted
browser checkpoint. Original live records win duplicate IDs. History never
advances the browser's MLS cursor, re-enters Hermes' inbox, switches Chat selection
or blocks a send while waiting for a response. A 15-second history timeout leaves
live transport ready and permits retry; late responses still merge. Each Chat has
its own cursor. An empty Chat has the same history control as one with live text.

**Validation:** `/tmp/finitechat-multibrowser-test-5.log` passes the
complete future-only scenario: B enrolled with A's browser process closed,
received metadata but no old transcript, continued the same real Hermes context,
and exchanged new messages with A after it reopened. B reload retained its own
Device and history. The test verified distinct Device IDs, one Room, signed
ciphertext traffic, no dashboard chat API/bridge, and rejected unsigned/tampered
admission.

`/tmp/finitechat-history-test-2.log` passes the complete history extension in
44 seconds with actual Hermes inference. A creates two Chats before B enrolls.
B gets neither transcript automatically. The test suspends only the disposable
native agent, requests history, observes the history-only timeout, and proves A's
new encrypted message still arrives on B. It closes A, resumes the agent and
fetches multiple two-event pages. Original history survives B's reload, live
messages are not duplicated, and the unrelated Chat remains empty until B requests
that Chat separately. `/tmp/finitechat-final-enrollment-negative.log` additionally
proves that **valid** signatures for the wrong account or wrong Room receive 403.

`/tmp/finitechat-final-browser-regressions-2.log` passes both original browser
suites (60 seconds): real terminal tool execution, Hermes context, separate
Chats, reload/browser restart, concurrent tabs, interrupted delivery and lost
receipts, failed IndexedDB save, and stale-checkpoint refusal. These tests now
create their own Chat explicitly because fresh browsers share the existing Room.
`/tmp/finitechat-final-native-checks-2.log` records 107 client/MLS tests passed,
one pre-existing ignored test, and native clippy. WASM clippy and the default
non-spike CLI check passed. Dashboard TypeScript, focused ESLint and production
build passed; the build reports 14 existing file-tracing warnings. These focused
checks do not claim the monorepo-wide Postgres/CI gate or production compatibility.

### Deliberate shortcuts and remaining proof

- **Hermes stream/lease recovery remains a known limitation.** An interrupted
  early outage test left entries in `hermes-inbox.json` marked `leased` after
  the stream disconnected; one later user request remained stored/leased without
  reaching the gateway. Evidence is in `/tmp/fc-wasm-qaa7f8rb/agent/hermes-inbox.json`
  and the associated gateway log (Room sequences 26 and 65). The writer is
  `lease_pending_hermes_inbox_events`; readers/acknowledgers are the native inbound
  stream and Hermes plugin. The later clean outage test passed, and the old
  persistence suites passed on a fresh fixture. This is not proof of reliable
  lease reclamation. Before shipping, reproduce and fix disconnect-before-ack,
  re-offer unacknowledged deliveries without rerunning acknowledged model turns,
  and prove native service/gateway restarts. No durable production state was
  inspected or repaired; the final demo uses a fresh disposable fixture.
- This is one fixture user, one Agent Room, one local agent. Core login, device
  grants, revocation and multi-Room/project fanout are not production features.
- The enrollment Commit journal and encrypted Device SQLite state are separate
  writes. A crash between them is not proven safe. A failed preparation can also
  leave a claimed KeyPackage without a resumable claim journal. Combine claim,
  grant, pending Commit and Device state transactionally; reconcile accepted
  Commits before preparing another. Add full crash and concurrent-Commit tests.
- Metadata currently exports the agent's whole Topic/Chat list in one event.
  Define bounded snapshots/chunks and test create/rename/archive races, large
  lists, conflicting metadata and agent restart at every boundary.
- The responder scans the most recent 10,000 stored events for unanswered
  requests. Replace that scan with a durable indexed request/response ledger and
  idempotent pending response envelopes. Full agent process-crash recovery is
  not proven by a temporary process suspension.
- History reads bounded SQLite pages but scans the Room to find the Chat. Add a
  per-Chat index and bounded asynchronous work so very large histories cannot
  monopolize the actor. Responses cap stored event JSON at 48 KiB; an oversized
  individual message returns a history-only error and needs chunked transfer.
  Metadata/history responses currently consume group bandwidth for all Devices.
  The target field is routing, not private encryption within the group: every
  active Room member can decrypt the response. This fixture has one human
  account; review per-recipient history authorization/encryption before using
  this design in multi-user Rooms.
- Define history retention/completeness and missing-source recovery. An empty
  result currently means only that this agent has no matching stored records;
  it is not proof the original Chat never had older messages. Attachments and
  original sender proof chains are not imported by this text spike.
- Use a production page size, incremental IndexedDB tables, bounded projections,
  and shared native projection code. The two-event page size exists to exercise
  actual pagination in this experiment, not as a proposed product default.

## Work required for a real product

1. **Key custody and login.** Implement an authenticated Core handoff scoped to
   the actual account/session. Specify storage encryption, key access auditing,
   logout/revocation, CSRF/CORS, session expiry, and which processes can recover
   the User Key. A JS string cannot be reliably zeroized. Web XSS or a
   compromised first-party bundle can steal an exported nsec; decide if a
   bounded signer/device enrollment design is preferable. Do not reuse the
   unauthenticated local fixture as an account API.
2. **Browser Device persistence.** The basic encrypted checkpoint, exact-send
   journal, Web Lock and restart/currency proofs now work. Replace full JSON
   snapshot rewrites with bounded storage and incremental projections; define
   schema/WASM upgrades and credential renewal (the spike issues 24-hour Device
   credentials). Test storage corruption, eviction, initial admission crashes,
   worker/tab suspension and all supported browsers. Normalize key/relay scope
   encoding. Persistent-storage permission is best effort, not a backup.
3. **Recovery and multiple Devices.** The selected Agent admission and per-Chat
   import paths now exist as a spike. Harden their authorization and recovery
   contracts, then prove empty-target recovery and missing Agent history. Prove
   revocation, partial/missing/conflicting chunks and retry across restart.
   Native sender currency/rollback protection is already wired and tested.
4. **Runtime/transport boundaries.** Split portable Device/codec code from
   native filesystem/SQLite workers instead of accumulating target guards in
   a large file. Share HTTP validation/encoding between async and blocking
   adapters. Port the relevant `finitechat-core` state/projections and expose
   a coherent browser API. Use a Web Worker if crypto/sync blocks rendering.
5. **Remaining Agent Runtime proof.** Real Hermes inference, terminal tools,
   allowlist admission, per-chat context and separate sessions now work. Still
   prove transient token streams, cancellation, attachment/tool approval flows,
   Agent process restart/recovery, and interrupted/outstanding work. Browser
   crash tests now pass; they do not prove every Agent Runtime recovery edge.
6. **Complete dashboard integration.** The actual UI now works for text, tool
   output, topics and chats. Replace the temporary TypeScript projector with
   shared canonical projections; support runtime snapshots/commands, activity,
   streaming finalize, receipts, attachments, Brain/Sites signing and cards.
   Bound incremental projection work instead of rebuilding an ever-growing
   event array on every poll. Validate incoming payloads and reconcile all
   native policies for titles, archives, edits and selection. Extend the existing
   reload/multi-tab proof to revocation and unsupported actions before enabling them.
   Project-to-Room binding remains navigation metadata, never membership
   authority; implement authenticated creation/admission explicitly.
7. **Browser networking.** Add reviewed CORS to the service-owned public
   router, signed browser SSE/long-poll wakeups, HTTPS/CSP, timeouts, background
   tab suspension/resume, reconnect/backoff, and useful send-failure behavior.
   Audit authentication of every MemberId-keyed route too: the existing
   server's signed-requests flag does not yet bind every such route to a key.
8. **Packaging/performance.** Release size is measured above. Measure mobile
   memory and startup/crypto cost; assess wasm-opt and Brotli delivery. Bundle a versioned worker/JS/WASM set with matching
   bindgen versions and MIME/cache policy. Keep all toolchain pins in Nix and
   the root lockfile. Add actual-browser WASM build/test coverage to CI.
9. **Product/security contracts.** Explicitly revisit the current Hosted Web
   Device custody language in `finitechat/CONTEXT.md` and Finite Computer's
   product boundary. The spike is an experiment alongside that ADR-backed
   design, not a silent change to it. User Key and Agent Principal Key must
   remain distinct; the user key must never be handed to the agent.
10. **Later rollout.** Only after the above works, design existing-account
    enrollment, existing Room/history transfer, deployment and rollback.
    These are intentionally unimplemented and untested in this spike.
