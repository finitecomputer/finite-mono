# Finite Chat to Finite Computer Handoff

Date: 2026-05-24

Audience: the LLM agent working in the normal `finitecomputer` checkout on SaaS work.

Read this before merging the Finite Chat integration. The important rule is to avoid editing or resetting unrelated SaaS work while bringing the chat stack under one roof.

## Where Things Are

Normal finitecomputer checkout:

```text
/Users/futurepaul/dev/finite/finitecomputer
```

Observed state when this handoff was written:

```text
branch: saasification-workos
HEAD: d231709 Add DeepSeek V4 Flash inference option
dirty files:
  modified: .gitignore
  modified: CONTEXT.md
  modified: Cargo.toml
  modified: docs/README.md
  untracked: apps/saas/
  untracked: crates/finite-saas-core/
  untracked: docs/saasification-plan.md
```

Do not assume this checkout is still exactly there. Inspect it first. Do not reset it.

Finitecomputer worktree with the Finite Chat integration:

```text
/Users/futurepaul/dev/finite/finitecomputer-worktrees/finitechat-hermes-plugin
branch: codex/finitechat-hermes-plugin
HEAD: 428ba5e Document hard cut chat archive policy
state: clean, pushed to origin
```

Recent integration commits on that branch:

```text
428ba5e Document hard cut chat archive policy
5d17b30 Prove Hermes activity is non-durable
94f3aaf Prove browser chat over finitechat transport
6754351 Prove runtime profile assets include finitechat plugin
d088aba Prove finitechat Hermes plugin boots
d8d07b4 Switch Hermes bridge to finitechat plugin
d231709 Add DeepSeek V4 Flash inference option
```

Finitechat repo:

```text
/Users/futurepaul/dev/finite/finitechat
branch: codex/blob-store-spike
HEAD: afaa6e4 Add finitechat Hermes bridge contract
state: clean when this handoff started
remote target: finitecomputer/finitechat
```

Recent finitechat commits:

```text
afaa6e4 Add finitechat Hermes bridge contract
d850282 Document editable topic metadata
e303e20 Harden Blossom download retry tests
4bd4a09 Prove bridge command isolation
cb0f151 Reject empty conversation ids
9c940c2 Prove attachments survive gateway failure
30bceee Harden Blossom HTTP boundary tests
df559ca Make product trust boundaries explicit
```

## What Finite Chat Is Now

Finite Chat is intended to be a standalone open source chat transport and protocol. Finite Computer is the first consumer.

Core product model:

```text
Identity    = user or agent identity, not assigned authoritatively by the server.
Device      = one concrete client/runtime endpoint for an identity.
Room        = Finite Chat security and delivery boundary.
Conversation/topic = app-level session inside a room, carried in message payload metadata.
```

Important decisions already made:

- Room membership is stable across the user devices and agent/runtime devices that need to communicate.
- A Hermes or Finite Computer "new chat" maps to a Finite Chat conversation inside a room, not a new room.
- Topics are first-class app metadata and can group conversations without making room topology complicated.
- Topic display names are editable metadata. Stable IDs remain separate from human names.
- Conversation IDs are validated and cannot be empty.
- Capabilities are encrypted payload concepts, not server-authoritative identity concepts.
- Ephemeral activity such as typing, thinking, and working is separate from durable messages.
- Ephemeral activity must not create push notifications and must not enter durable inbox state.
- Dashboard status should come from structured status snapshots, not chat RPC roundtrips on every page load.
- Runtime commands are structured messages, but Finite Chat itself should remain generic transport.
- Old plaintext chats should become archived/read-only after cutover. No dual-write and no long-lived fallback stack.
- Hosted web chat is not advertised as E2EE because decryption is server-side Rust for this cut.
- True E2EE is for local daemon, Electron, and native clients where keys live on the user's device.

## Important Finitechat Docs

Read these in `/Users/futurepaul/dev/finite/finitechat`:

```text
docs/engineering-style.md
docs/protocol-v1.md
docs/finitecomputer-integration.md
docs/hermes-integration.md
docs/scenario-coverage.md
docs/daemon-survival-testing.md
docs/technical-debt-ledger.md
docs/adr/
```

Important implementation areas:

```text
integrations/hermes/finitechat/
crates/finitechat-hermes/
crates/finitechat-proto/
crates/finitechat-client/
crates/finitechat-sim/
crates/finitechat-relay/
crates/finitechat-blob/
```

Tests worth reading first:

```text
crates/finitechat-client/tests/client_state.rs
crates/finitechat-sim/tests/daemon_survival.rs
crates/finitechat-proto/tests/
crates/finitechat-blob/tests/
crates/finitechat-hermes/tests/
```

Engineering style is deliberately stricter than ordinary Rust glue code. Prefer explicit error enums, asserts on input and output invariants, bounded loops, narrow functions, and tests with valid and invalid data.

## What The Finitecomputer Worktree Changed

The integration worktree moved finitecomputer toward consuming finitechat instead of carrying its own temporary Hermes plugin.

Notable finitecomputer changes:

- `flake.nix` has a finitechat input:

```text
github:finitecomputer/finitechat?ref=codex/blob-store-spike
```

- The old local plugin under `integrations/hermes/finitechat` was removed.
- Runtime profiles now copy the finitechat-owned Hermes plugin from the finitechat flake input.
- Local dev/bootstrap scripts copy the finitechat plugin into the Hermes environment.
- Runtime scripts set the finitechat plugin environment, including `FINITECHAT_ROOM_ID` and `FINITECHAT_BIN`.
- `finitec hermes` grew JSON commands used by the plugin:

```text
finitec hermes poll --json
finitec hermes ack --json
finitec hermes send --json
finitec hermes edit --json
finitec hermes activity --json
```

- Gateway events now expose a stable SQLite-backed sequence for acking.
- Hermes activity is proven non-durable and separate from inbox work.
- Browser chat E2E proves messages travel through the finitechat transport path.
- Legacy plaintext chats are treated as archived/read-only after the hard cut.

Important scripts in the finitecomputer integration worktree:

```text
scripts/hermes_plugin_boot_smoke.sh
scripts/nix_profile_assets_smoke.sh
scripts/relay_e2e.sh
scripts/chat_browser_e2e.sh
scripts/chat_local_bootstrap.sh
scripts/chat_local_up.sh
```

## What Is Proven

Finitecomputer gates passed on the integration branch:

```text
cargo test --workspace
cargo clippy --all-targets -- -D warnings
scripts/hermes_plugin_boot_smoke.sh
scripts/nix_profile_assets_smoke.sh
scripts/relay_e2e.sh
```

Browser E2E passed against local dashboard plus relay:

```text
FC_CHAT_BROWSER_BASE_URL=http://localhost:3120 \
FC_CHAT_BROWSER_MACHINE=branch-proof-finitechat \
scripts/chat_browser_e2e.sh
```

The browser E2E covered:

```text
text send -> agent reply ok
reload replay ok
command menu closes on outside click ok
failed upload restores composer ok
image upload -> attachment render -> agent reply ok
post-upload replay ok
actual agent media creation -> attachment render ok
```

Finitechat gates passed:

```text
cargo test --workspace
```

Daemon survival work in finitechat:

```text
cargo test -p finitechat-sim --test daemon_survival
```

This passed 21 tests, including runtime command progress via ephemeral activity.

Legacy archive proof:

```text
cargo test -p finitechat-proto old_plaintext_chats_render_as_read_only_archive
```

Live restart smoke also passed:

```text
Hermes gateway restarted: 268 -> 1118; dashboard bootstrap reachable
```

## Live Demo State

When this handoff was written, a local demo was still running from the finitecomputer integration worktree:

```text
dashboard: http://localhost:3120/dashboard/chat/machines/branch-proof-finitechat
relay port: 4120
finitecomputer worktree: /Users/futurepaul/dev/finite/finitecomputer-worktrees/finitechat-hermes-plugin
```

Observed listeners:

```text
finited on *:4120
node on *:3120
```

Do not rely on those processes surviving. Verify with:

```text
lsof -nP -iTCP:3120 -iTCP:4120 -sTCP:LISTEN
```

## What Is Not Done

Do not overclaim any of this:

- Hosted web chat is not E2EE in this cut. It is improved chat protocol/transport with server-side Rust handling keys.
- The finitecomputer branch still has some temporary local bridge shape around `finite-core::ChatRuntime`.
- The durable finitechat daemon/server is not fully owning every finitecomputer chat path yet.
- Blossom-style encrypted blob storage is implemented and tested in finitechat, but finitecomputer attachment handling has not fully moved to that store.
- Electron/native true-device E2EE clients are future work.
- Multi-user/group UI is not shipped, but the protocol leaves room for it.
- There may still be browser/dashboard compatibility fallbacks that should be deleted during the hard cut instead of preserved.
- The SaaS checkout has probably moved. Reinspect before merging.

## Recommended Merge Strategy

Prefer a new integration branch or worktree based on the current SaaS checkout state. Do not merge directly into a dirty checkout unless the human explicitly asks.

Recommended sequence:

1. Inspect `/Users/futurepaul/dev/finite/finitecomputer`.
2. Protect the SaaS work by committing it, stashing it, or creating a new worktree from the current branch.
3. Compare the integration worktree against the normal checkout:

```text
git -C /Users/futurepaul/dev/finite/finitecomputer-worktrees/finitechat-hermes-plugin log --oneline --decorate -n 12
git -C /Users/futurepaul/dev/finite/finitecomputer log --oneline --decorate -n 12
```

4. Bring over the finitechat integration commits in order, resolving conflicts deliberately:

```text
d8d07b4 Switch Hermes bridge to finitechat plugin
d088aba Prove finitechat Hermes plugin boots
6754351 Prove runtime profile assets include finitechat plugin
94f3aaf Prove browser chat over finitechat transport
5d17b30 Prove Hermes activity is non-durable
428ba5e Document hard cut chat archive policy
```

5. Re-run the finitecomputer gates.
6. Re-run the browser E2E against a fresh local machine name.
7. Delete temporary fallbacks rather than broadening them.

If cherry-picks conflict with SaaS work, prefer the current SaaS architecture for auth/product boundaries and the finitechat worktree for chat/protocol boundaries.

## Gates To Re-run After Merge

In finitecomputer:

```text
cargo test --workspace
cargo clippy --all-targets -- -D warnings
scripts/hermes_plugin_boot_smoke.sh
scripts/nix_profile_assets_smoke.sh
scripts/relay_e2e.sh
```

For browser transport proof:

```text
just chat-local-bootstrap branch-proof-finitechat
just chat-local-up branch-proof-finitechat paul@finite.vip 3120 4120
FC_CHAT_BROWSER_BASE_URL=http://localhost:3120 \
FC_CHAT_BROWSER_MACHINE=branch-proof-finitechat \
scripts/chat_browser_e2e.sh
```

If Hermes bootstrap is missing a host venv, use the existing bootstrap path before running plugin smoke. Do not patch around failures without recording the technical debt.

In finitechat:

```text
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

Targeted finitechat checks:

```text
cargo test -p finitechat-sim --test daemon_survival
cargo test -p finitechat-proto old_plaintext_chats_render_as_read_only_archive
cargo test -p finitechat-hermes
cargo test -p finitechat-blob
```

## Technical Debt To Track Loudly

Observed debt that should not metastasize:

- Plaintext bridge paths in finitecomputer should be transition scaffolding only.
- Any dashboard fallback that hides protocol failure should be removed or made explicitly temporary.
- Any chat command that exists only to satisfy dashboard load should become a structured status snapshot instead.
- Attachments should move toward finitechat blob references instead of bespoke finitecomputer upload state.
- The Hermes Python plugin should stay thin. Finitechat owns protocol semantics and tests.
- The finitecomputer product should own hosted capabilities such as private inference, web access, repos, and user sites.
- Finite Chat should own generic messaging, topics, activity, attachments, identities/devices, and transport primitives.
- If a bridge has to cross a trust boundary, document the physical/process boundary and what secrets can exist on that side.

## Product Boundary

Finite Chat:

- open source generic transport for humans, agents, devices, rooms, topics, messages, activity, and attachments
- local daemon and future Electron/native clients
- Nostr-rooted identity proof and device separation
- eventual true E2EE where keys live on user devices

Finite Computer:

- hosted product using finitechat primitives
- WorkOS account/product flow
- hosted agent deployment
- private inference endpoint
- web access, user sites, repos, and opencode-like hosted product capabilities
- Finite Computer-specific UI that lights up when the bot/runtime advertises those capabilities

This means Finite Computer should import finitechat primitives instead of reimplementing chat.

## Prompt For The Main SaaS Agent

Use this as the first prompt to the agent working in `/Users/futurepaul/dev/finite/finitecomputer`:

```text
Read /Users/futurepaul/dev/finite/finitechat/docs/finitecomputer-handoff-2026-05-24.md, then inspect the normal finitecomputer checkout. Do not reset or overwrite SaaS work. Create a merge plan for bringing in /Users/futurepaul/dev/finite/finitecomputer-worktrees/finitechat-hermes-plugin branch codex/finitechat-hermes-plugin and consuming /Users/futurepaul/dev/finite/finitechat branch codex/blob-store-spike. Acceptance: finitecomputer imports the finitechat-owned Hermes plugin, old plaintext chat is archived/read-only, dashboard/browser chat uses the finitechat transport path, temporary fallbacks are inventoried or deleted, and all listed gates pass.
```
