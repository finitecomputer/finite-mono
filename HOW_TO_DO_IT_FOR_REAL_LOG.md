# FiniteChat in the browser — feasibility spike

Started 2026-09-10 from `origin/main` (`4c924a95`) on `codex/finitechat-wasm-spike`.

## Result and scope

The existing Rust `FiniteChatDevice` compiles to `wasm32-unknown-unknown` and
exchanges real MLS messages with a native FiniteChat Device from a page in the
Next dashboard. Both sides use the existing FiniteChat credentials, MLS
implementation, wire protocol, and Hermes chat payload. No Hosted Web Device
process, hosted action route, or plaintext web-chat proxy participates.

This ports `finitechat-client`'s Device, with `finitechat-mls`, `finitechat-proto`,
`finitechat-http`, `finitechat-delivery`, `finitechat-transport`, `finite-nostr`,
and `finitechat-hermes`. **It does not yet port `finitechat-core`'s full app
runtime/projections or replace the normal dashboard chat screen.**

The local counterpart is a deterministic **native agent peer**, not a running
Hermes LLM gateway. It decrypts an incoming Hermes-compatible chat event and
returns a new encrypted reply. This proves the browser/native transport boundary;
real Hermes inference and the full Agent Runtime are a follow-up test.

The user's proposed Core custody model is represented by a disposable local
bootstrap fixture. The sign-in button gets an nsec once and hands it to WASM.
There is no real WorkOS sign-in, Core database column, or production key export
in this spike. A fresh, random User Key and separate Agent Principal Key are
created for each harness run. No real account or production state is accessed.

Acceptance for this slice: compile the real client; use a real browser in the
web dashboard; create a Room and admit a native Device; send/decrypt replies;
prove the browser calls the relay directly with ciphertext; never call the
hosted bridge. No migration or mixed-version work, per the spike request.

## Run it

From this branch's worktree:

```sh
scripts/finitechat-wasm-spike up
```

Open http://127.0.0.1:13010/wasm-spike and click **Sign in as local spike user**.
Send any text. The peer responds `Native agent received over MLS: …`.
Ctrl-C stops only this run's processes. Synthetic state, logs, and test
screenshots remain under `.local-state/wasm-spike/run-*` for inspection.
Every launch makes a fresh directory; it refuses to reuse an occupied port or
existing peer state. Reloading the browser creates a new Device and Room;
there is deliberately no browser history persistence in this slice.

```sh
# Build only (the shell supplies Rust, WASM std, Clang, Node and bindgen).
scripts/finitechat-wasm-spike build

# Start an isolated stack, run Chromium assertions, then stop the stack.
scripts/finitechat-wasm-spike test --dashboard-port 13011 --peer-port 28790
```

The runner requires an existing Chromium/Chrome executable or Playwright browser
(the dashboard's existing browser selection helper is reused). It never starts
Core, a Runner, Hosted Web Device, or the normal devfinity stack.
The special page and bootstrap endpoint require `FINITECHAT_WASM_SPIKE=1` and
refuse production mode. The runner binds both servers to loopback. It configures
CORS for exactly the local dashboard origin. Generated WASM/JS, databases, and
logs are ignored; no generated binary or key belongs in git.

## Actual data flow

```text
Local sign-in fixture (stand-in for Core holding the User Key)
  -> dashboard POST /api/wasm-spike/bootstrap (nsec, relay URL, Agent account)
  -> browser's Rust/WASM FiniteChatDevice
       creates its own MLS leaf key and account-signed Device credential
       claims the Agent's KeyPackage
       creates Room + MLS group
       signs /account-rooms/bootstrap and /commits with the User Key
       prepares MLS Commit + encrypted Welcome for the Agent
       encrypts the normal Hermes chat application event
  -> direct browser Fetch to FiniteChat relay /events and /sync/group
       relay stores/orders opaque MLS envelopes; knows membership metadata
  -> native Agent FiniteChatDevice + encrypted SQLite store
       claims/activates Welcome, authenticates/decrypts the user event
       encrypts an independently created reply as the Agent Principal
  -> relay -> direct browser Fetch -> WASM decrypt -> React rendering
```

“Direct” here removes the Hosted Web Device hop. The ordinary FiniteChat relay
still provides ordered delivery; this is not peer-to-peer WebRTC or a new socket
exposed by the Agent Runtime. Core custody of the nsec also means this must not
be marketed as cryptographic operator blindness.

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

## Evidence

- Rust built and bindgen generated a browser WASM module (about 9.9 MiB in the
  initial unoptimized debug build; release size/startup are not measured yet).
- A Chromium test loaded that `.wasm` from the dashboard, created the MLS Room,
  and completed two consecutive native-peer round trips.
- The test checks that `/events` goes to the relay's origin, carries a Nostr
  signature, and contains no plaintext marker even after base64-decoding the
  ciphertext. It aborts any dashboard API call other than the bootstrap.
- The native peer's only reply path follows MLS decryption with the shared
  client; the browser's reply rendering follows MLS decryption too.
- Replaying without authentication returns 401. Flipping a ciphertext byte
  while keeping the original signature returns 401. This last check proves
  HTTP body binding, not a separate exhaustive MLS tamper suite.
- Native client regression suite: 92 passed, one performance benchmark ignored.
  Formatting, dashboard typecheck/lint, and wrapper clippy are checked by the
  spike's finishing validation. No deployment or production compatibility
  claim is made by this evidence.

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
5. **Full Agent Runtime proof.** Replace the deterministic peer with the real
   `finitechat hermes serve` and pinned Hermes gateway in an isolated Agent
   Home. Prove the agent's allowlist/admission, user identity, chat/session
   dispatch, streamed replies, tools, cancellation and restart behavior. Use
   the existing Hermes integration and a legitimate local inference key; do
   not introduce a server-side browser plaintext adapter to pass this test.
6. **Dashboard integration.** Replace the hosted action/state source behind
   the actual chat UI, including Rooms, Topics, Segments, selection, message
   edits, stream finalize, activity, attachments, read receipts, runtime
   commands/state and cards. The spike page is a text-only test surface.
   Project-to-Room binding remains navigation metadata, never membership
   authority; define authenticated creation/admission explicitly.
7. **Browser networking.** Add reviewed CORS to the service-owned public
   router, signed browser SSE/long-poll wakeups, HTTPS/CSP, timeouts, background
   tab suspension/resume, reconnect/backoff, and useful send-failure behavior.
   Audit authentication of every MemberId-keyed route too: the existing
   server's signed-requests flag does not yet bind every such route to a key.
8. **Packaging/performance.** Measure release WASM size, mobile memory and
   startup/crypto cost. Bundle a versioned worker/JS/WASM set with matching
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
