# Identity Rollout: Paul's Test Log

Companion to `docs/identity-rollout-reconciled-plan.md`. Things Paul needs to
test, provide, or eyeball — locally or after deploy — in the order they'll
come up. Check items off as they happen.

## Needed from Paul before hosted Sites validation

- [ ] `WORKOS_STAGING_API_KEY` and `WORKOS_STAGING_CLIENT_ID` values, placed in
      repo-root `.env.local` (never committed). See `docs/local-integration-harness.md`.
- [ ] Confirm the staging AuthKit app has `http://127.0.0.1:13002/callback`
      (and companions listed in `docs/local-integration-harness.md`) registered.

## Phase 1 — PR #172 (brain), local

- [ ] `just dev up` boots clean on the rebased 172 branch.
- [ ] From a chat turn: ask the agent to create a brain; confirm it appears
      and is loadable.
- [ ] Invite a second identity to the brain by email; claim flow works end to
      end (invite email prints via dev mailer).
- [ ] Migration check: open a pre-172 brain DB with the new server; confirm
      all prior access still works and nothing widened.
- [ ] Dashboard: Brain nav still disabled (`aria-disabled`, "Coming soon").

## Phase 1 — PR #172, deployed canary

- [ ] New agent on lat3: create + load a brain from chat.
- [ ] Upgrade Canary 0715: same checks after its image roll; confirm its
      pre-existing brain access is unchanged.
- [ ] (Optional) one lat1 agent: same checks, note any host drift observed.
- [ ] Old-skill agent in the rollout window: brain commands fail LOUDLY with a
      clear error, no corruption, and work again after the image roll.

## Phase 2 — Sites slices, local

- [ ] (a) `fsite --email <managed-agent-nip05>` fails with corrective guidance,
      sends no mail; `fsite auth status` shows the agent's NIP-05.
- [ ] (b) Two test npubs registered under one mailbox; both edit the same
      site; revoking one leaves the other working.
- [ ] (b) `finitesitesd reconcile-identity` previews from a consistent copy
      without changing the source registry; `--apply yes` is the only path
      that performs the one-off durable reconciliation.
- [ ] (c) Agent creates a private site in a hosted chat turn; Paul sees it in
      the dashboard WITHOUT any manual share step.
- [ ] (c) Private-site email-auth page: unshared email gets truthful "not
      shared" + Request Access (no share-status leak before verification).
- [ ] (c) Request Access approval email arrives via the unified mailer;
      approval grants persistent access — no single-use link UX.
- [ ] (c) First-publication notification arrives exactly once per site.

### Phase 2(b) environment-repair decision

Use the existing provider-neutral Runtime Spec environment reconciliation; do
not add a production-only repair script. Core already persists the desired
`FINITE_BRAIN_SERVER_URL` and `FINITE_BRAIN_PUBLIC_BASE_URL`, and Kata upgrade
merges those explicitly desired keys while retaining unrelated Runtime
contract values and secrets. The 21 stale lat1 Kata agents therefore repair as
part of the reviewed, digest-pinned runtime upgrade cohort, with the normal
prepare/hash/execute and rollback boundary. Phala consumes the same desired
environment on creation, but Phala upgrade remains deliberately disabled until
its complete-environment replacement/rollback canary passes; this train does
not bypass that gate.

## Phase 2 — deployed canary (Paul's own account first)

- [ ] Reconciliation report for `paul@finite.vip` (currently mis-linked to
      Upgrade Canary's key) shows expected migrated/conflict rows BEFORE any
      mutation is approved.
- [ ] Post-reconciliation: every site Paul could access before is still
      accessible, same URLs, same visibility.
- [ ] Waffle Prime (older agent, previously repaired binding) can edit its
      sites through the new path.

## Phase 3 — Chat repair, local PR evidence

- [x] Two concurrent chats with a delayed response: visible chat never jumps
      (PR #356; retain as regression coverage).
- [x] Manually collapse a running tool rollup: it stays collapsed across
      snapshots, timers, and completion.
- [ ] Record how pinned Hermes Telegram and Discord adapters consume
      clarification plus adjacent working/typing/thinking/status callbacks.
- [ ] Hermes clarification question: renders fully, answer resumes the exact
      originating turn, and cannot be answered from another active chat.
- [ ] Clarification uses Hermes pending state and ordinary Chat messages; no
      new core Chat protocol type or emoji/prose classifier.
- [ ] Inspect whether pinned Hermes exposes semantic compaction start/finish
      to adapters. If not, record it in the parking lot and move on.
- [ ] Reload / adapter reconnect mid-turn: no lost messages or wrong-chat
      admission.
- [ ] Paul explicitly authorizes any merge, canary, or rollout separately.
