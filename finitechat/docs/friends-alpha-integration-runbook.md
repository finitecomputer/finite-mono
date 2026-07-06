# Friends Alpha Integration Runbook

This runbook is the Phase 8 gate for `docs/friends-alpha-hardening-plan.md`.
Use it to collect evidence before inviting friends. Do not count dev-only
fixtures, transient stores, or manual database edits as product proof.

## Required Inputs

- Finite Chat branch: `codex/friends-alpha-hardening`.
- Finite Sites branch: `codex/native-viewer-auth`, or equivalent merged code
  with `POST /_finite/auth/native-session`.
- Deployed chat server: `https://chat.finite.computer`. The native app should
  use this by default. Local or staging servers are branch-validation tools,
  not normal phone-test UX.
- A physical iPhone signed with the Friends Alpha bundle identifier and push
  entitlement.
- Friend self-build path: `docs/friends-alpha-self-build.md`.
- APNs token key, key id, team id, bundle topic, and sandbox/production choice.
- One deployed or locally routed private Finite Site shared to the user's
  native npub.
- One clean Agent Home path that has not been used by prior tests.

## Automated Baseline

Run these before manual proof:

```sh
cargo test -p finitechat-mls -p finitechat-core
cargo clippy -p finitechat-mls -p finitechat-core --all-targets -- -D warnings
```

Then run the iOS simulator unit suite for scheme `FiniteChat`. The current
canonical command is:

```sh
cargo run -q -p finitechat-rmp -- test ios-simulator
```

Expected result: all simulator unit tests pass, with only intentionally skipped
live-relay tests skipped. The RMP command erases the dedicated simulator before
launch, writes derived data and the `.xcresult` bundle under `.state`, and shuts
the simulator down after the run so back-to-back unit-suite runs do not inherit
stale app state or a busy SpringBoard.

## Server Sync Gate

Friends Alpha phone testing uses the deployed server by default. Before handing
the app to a tester, run the release gate in
`docs/server-deployment-gate.md`:

```sh
cargo run -q -p finitechat-cli -- http --server https://chat.finite.computer health
```

The production response must include `server_version`, `source_commit`, and
`source_dirty: false`. The `source_commit` must be the finite-chat commit that
the app build expects. If those fields are missing, production is an old server
build and Friends Alpha is blocked until `../finitecomputer-v2` deploys a
compatible commit.

If the app change requires server-side work, do not continue with native app
proof until Paul has the finite-chat commit, the server/worker list to deploy,
and the expected post-deploy `/health` payload for the
`../finitecomputer-v2` chat server deploy lane.

## Clean Chat And Agent Setup

Use the deployed chat server unless this run is explicitly validating an
unmerged server branch. A fresh app install should not require server entry on
the phone.

```sh
export FRIENDS_ALPHA_ROOT="$PWD/.state/friends-alpha"
export FINITECHAT_SERVER_URL="https://chat.finite.computer"
export AGENT_HOME="$FRIENDS_ALPHA_ROOT/agent-home"
rm -rf "$FRIENDS_ALPHA_ROOT"
mkdir -p "$FRIENDS_ALPHA_ROOT"
```

If a local server is genuinely required for branch validation, keep it out of
the human app UX. Start it explicitly and pass the URL through Xcode launch
arguments or harness commands:

```sh
cargo run -p finitechat-server -- serve 0.0.0.0:8787 \
  --sqlite "$FRIENDS_ALPHA_ROOT/server.sqlite3"
```

For a physical phone, expose that local server through a reachable HTTPS
staging URL before using it. Do not ask friends to type server URLs into the
app.

Initialize the agent principal and Hermes runtime:

```sh
cargo run -p finitechat-cli -- identity --agent-home "$AGENT_HOME" init
cargo run -p finitechat-cli -- identity --agent-home "$AGENT_HOME" show
cargo run -p finitechat-cli -- hermes --agent-home "$AGENT_HOME" init \
  --server "$FINITECHAT_SERVER_URL" \
  --device-id friends-alpha-agent
cargo run -p finitechat-cli -- hermes --agent-home "$AGENT_HOME" install --force
```

Start the Hermes service boundary:

```sh
cargo run -p finitechat-cli -- hermes --agent-home "$AGENT_HOME" serve \
  --addr 127.0.0.1:0 \
  --ready-file "$FRIENDS_ALPHA_ROOT/hermes.ready" \
  --json
```

Record the service URL from the ready file, then confirm health from
`finitecomputer-v2` or the runtime supervisor once that path is wired into the
run.

## Native App Chat Proof

On the physical iPhone and one simulator or second physical device:

- Sign in with the user's User Key in the native app.
- Join or create a 1:1 room with the agent.
- Send user-to-agent and agent-to-user text.
- Send encrypted media both directions.
- Create a group room with two users plus one directly invited agent.
- Confirm Hermes sender identity distinguishes both humans.
- Set Hermes home channel to the 1:1 room, restart the Hermes service, and
  confirm the setting survives.
- If Hermes permits it, set a group room as home and verify routing remains
  explicit.
- Kill and restart the Hermes service; confirm no duplicated acknowledged
  messages and no lost pending turn.

## Native Finite Sites Proof

Use a Finite Site served by the native viewer auth branch or merged equivalent.

- Share a private site to the user's native npub.
- Disable relay network access for this test environment.
- Send the site URL in a Finite Chat message.
- Tap the link in the native app.
- Confirm the in-app browser opens the site without email login.
- Confirm the site page never receives the nsec or signed NIP-98 event.
- Revoke the share and reload; confirm access is removed on the next request.
- Open a public site; confirm it still loads when native auth is rejected or
  unnecessary.
- Open an unshared private site; confirm it shows the normal unauthorized/login
  surface.

## Wake-Only Push Proof

Follow `docs/push-notifications-apple-runbook.md` from clean Apple state.
Record:

- Bundle id, APNs environment, team id, and key id.
- Device token registration diagnostic from the app.
- `push-drain` command and server URL.
- Claimed/sent/acked counts from the pusher.
- Confirmation that APNs payload contains only wake metadata.
- Locked-phone observation: phone wakes, app syncs after open, and plaintext
  appears only after app sync.

## Finite Blob Proof

- Upload/download a normal encrypted chat attachment.
- Exercise one non-chat caller using the shared Finite Blob capability model
  once finite-brain or finite-sites is wired to that capability path.
- Confirm provider details and direct bucket credentials are not visible to the
  product caller.
- Confirm capability scope rejects wrong principal/product and expired tokens.

## Evidence Log

| Gate | Evidence | Result | Notes |
| --- | --- | --- | --- |
| Rust baseline | `cargo test -p finitechat-mls -p finitechat-core` | Passed 2026-06-24 | 51 core tests, 14 MLS tests |
| Rust lint | `cargo clippy -p finitechat-mls -p finitechat-core --all-targets -- -D warnings` | Passed 2026-06-24 | no warnings |
| iOS simulator | XcodeBuildMCP `test_sim` for `FiniteChat` | Passed 2026-06-24 | 98 passed, 1 live-relay test skipped |
| Deployed server sync | production `/health` with `source_commit`, `source_dirty: false` | Passed 2026-06-24 | `02dff50a97e0`, clean deploy on box1 |
| Clean Agent Home | identity init/show output, no reused state | Pending | |
| Hermes service | ready file, health, restart behavior | Pending | |
| 1:1 agent chat | transcript/video or logs | Pending | |
| two-user + agent group | transcript/video or logs | Pending | |
| Hermes home channel | show/set/restart evidence | Pending | |
| Native fsite private share | no email flow, no relays | Pending | |
| Native fsite revocation | reload loses access | Pending | |
| APNs wake | locked physical iPhone proof | Pending | |
| Blob shared substrate | chat plus one other product caller | Pending | |

Commit final evidence after the first full pass. If a shortcut is used to
unblock Friends Alpha, add it to `docs/technical-debt-ledger.md` with source,
risk, first proof, and delete condition before calling the gate passed.
