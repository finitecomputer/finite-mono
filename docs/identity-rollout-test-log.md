# Identity Rollout: Paul's Test Log

Companion to `docs/identity-rollout-reconciled-plan.md`. Things Paul needs to
test, provide, or eyeball — locally or after deploy — in the order they'll
come up. Check items off as they happen.

## Needed from Paul before local validation

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
- [ ] (c) Agent creates a private site in a hosted chat turn; Paul sees it in
      the dashboard WITHOUT any manual share step.
- [ ] (c) Private-site email-auth page: unshared email gets truthful "not
      shared" + Request Access (no share-status leak before verification).
- [ ] (c) Request Access approval email arrives via the unified mailer;
      approval grants persistent access — no single-use link UX.
- [ ] (c) First-publication notification arrives exactly once per site.

## Phase 2 — deployed canary (Paul's own account first)

- [ ] Reconciliation report for `paul@finite.vip` (currently mis-linked to
      Upgrade Canary's key) shows expected migrated/conflict rows BEFORE any
      mutation is approved.
- [ ] Post-reconciliation: every site Paul could access before is still
      accessible, same URLs, same visibility.
- [ ] Waffle Prime (older agent, previously repaired binding) can edit its
      sites through the new path.

## Phase 3 — Chat repair, local then canary

- [ ] Two concurrent chats with a delayed response: visible chat never jumps.
- [ ] Manually collapse a running tool rollup: it stays collapsed across
      snapshots, timers, and completion.
- [ ] Hermes clarification question: renders fully, answer resumes the exact
      originating turn, survives an adapter restart.
- [ ] Forced compaction: scoped "Summarizing earlier context…" appears in the
      affected chat only, clears on resume.
- [ ] Reload / restart mid-turn: no lost messages or pending turns.
