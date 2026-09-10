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
Migration and mixed-version proof remain out of scope per the spike request.

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
creates a new state directory. **Browser state is in memory: refreshing the page
creates a new Device/Room and loses that browser's access to the old transcript.**
The Agent/relay retain their own state. An nsec alone does not recover MLS history.

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
remembered code, a separate new chat, and return to the prior transcript.
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
       claims Agent KeyPackage; creates Room and MLS group
       signs /account-rooms/bootstrap and /commits with User Key
       creates MLS Commit + encrypted Welcome for the Agent
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
service is its ordinary internal adapter: **the browser never calls it**.
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
| WASM | 3,775,150 | 1,959,686 |
| JS loader | 29,205 | 6,500 |
| Total | 3,804,355 | 1,966,186 |

That's **3.60 MiB raw / 1.87 MiB gzip for WASM**, or **1.88 MiB gzip including
the loader**. Brotli gives 1,742,595 bytes for WASM plus 5,633 bytes of JS
(**1.67 MiB total**). These are compressed file measurements, not a measurement of the
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
  Next build reports 12 file-tracing warnings through the unchanged
  `workspace-paths.ts`/`fc-dashboard.ts` imports. The prior native client regression run
  passed 92 tests with one performance benchmark ignored; this extension did
  not change the native client implementation.

## Work required for a real product

1. **Key custody and login.** Implement an authenticated Core handoff scoped to
   the actual account/session. Specify storage encryption, key access auditing,
   logout/revocation, CSRF/CORS, session expiry, and which processes can recover
   the User Key. A JS string cannot be reliably zeroized. Web XSS or a
   compromised first-party bundle can steal an exported nsec; decide if a
   bounded signer/device enrollment design is preferable. Do not reuse the
   unauthenticated local fixture as an account API.
2. **Browser Device persistence.** Select IndexedDB or OPFS/SQLite-WASM and
   persist the Device signer, MLS storage, pending admission/Commit state,
   cursors and app events atomically. Persist before publishing consumed key
   material. Enforce a single writer across tabs/workers; test crashes between
   encryption, server acceptance and save. The current browser wrapper owns a
   transient cursor separately and is expressly not a durable sync engine.
3. **Recovery and multiple Devices.** Restoring an nsec restores account
   signing authority, not MLS state or old history. Design Room admission,
   history transfer, browser storage loss/eviction, new browsers, revocation,
   quota failure, and empty-target restore. Keep the existing sender currency
   gate and rollback protection through the browser storage path.
4. **Runtime/transport boundaries.** Split portable Device/codec code from
   native filesystem/SQLite workers instead of accumulating target guards in
   a large file. Share HTTP validation/encoding between async and blocking
   adapters. Port the relevant `finitechat-core` state/projections and expose
   a coherent browser API. Use a Web Worker if crypto/sync blocks rendering.
5. **Remaining Agent Runtime proof.** Real Hermes inference, terminal tools,
   allowlist admission, per-chat context and separate sessions now work. Still
   prove transient token streams, cancellation, attachment/tool approval flows,
   process restart/recovery, and interrupted/outstanding work. Native daemon
   durability is not evidence of browser crash safety.
6. **Complete dashboard integration.** The actual UI now works for text, tool
   output, topics and chats. Replace the temporary TypeScript projector with
   shared canonical projections; support runtime snapshots/commands, activity,
   streaming finalize, receipts, attachments, Brain/Sites signing and cards.
   Bound incremental projection work instead of rebuilding an ever-growing
   event array on every poll. Validate incoming payloads and reconcile all
   native policies for titles, archives, edits and selection. Test refresh,
   multi-tab, revocation and all unsupported actions before enabling them.
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
