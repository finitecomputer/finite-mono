# Triage & Priorities — 2026-07-17

> **Status: historical assessment; dispositioned 2026-07-20.** This file
> records what was believed on July 17 and is not deployed-state or execution
> authority. Current host truth lives in [`infra/README.md`](../infra/README.md);
> the accepted next candidate lives in
> [`docs/runs/finite-lat-capacity-and-redundancy.md`](runs/finite-lat-capacity-and-redundancy.md)
> and [ADR 0005](adr/0005-finite-lat-host-roles-and-placement.md).
>
> The recommendation to keep `finite-lat-2` dedicated to CI/building remains
> accepted. The “no new boxes” posture is superseded by the owner's
> `finite-lat-3` decision. The claim that live Postgres and its dumps share one
> disk is stale: the 2026-07-18 inventory observed Postgres on lat1 root and
> dumps on its separate `/data` disk. Both remain single disks in one chassis,
> and complete Agent recovery is still unproved.

**For:** Paul. **From:** a full read of `docs/` (doctrine, ADRs, audits, runs,
open questions, parking lot, slop audit), `infra/` (hosts, runbooks, nixos,
workflows), every component's CONTEXT/README/debt ledgers, and the last two
weeks of git history, against your product writeup.

This is a triage document, not a run. It complements
`docs/runs/parking-lot.md` and `docs/open-questions.md`; where it disagrees
with them on sequencing, update those.

---

## TL;DR

There are three critical paths, in this order:

1. **Close the revenue loop.** Stripe production activation is ACTIVE and the
   only remaining acceptance step is yours: an inspection-only live audit plus
   the first real $200 charge (`docs/runs/stripe-production-activation.md:18,256`,
   expires 2026-08-03). This is hours of your time and it unblocks billing
   (Skyler's biggest headache), paid cohorts, trial modes, and the Phala paid
   tier. Nothing else on this list has that leverage-per-hour ratio.
2. **Data availability on lat1.** The doctrine calls user data availability
   "the first security invariant," and it is currently the least true thing
   about the platform: one box, one NVMe, Postgres dumps written to the *same
   disk* as the live DB, off-host borg covering only the hosted-chat set and
   never restore-drilled, Brain/Sites/agent-`/data` with no off-host recovery
   at all. Every component's own docs independently name recoverability as
   the launch blocker. You are one decision (name a drill target) away from
   unblocking the fix.
3. **Reliability of the shipped chat surface.** The 07-13→07-15 incident
   cluster (stale panes, latched "Working", tool-ordering corruption, frozen
   timestamps, phantom topics) is fixed; the durable protocol was never the
   problem — the Hermes bridge/dashboard boundary was. Close the remaining
   open loops and add the observability that would have caught them earlier.

And one structural finding that sits above all three: **you are the
bottleneck.** 300 of 447 commits since 2026-07-10 are yours, and every
readiness run gates on a scripted Acceptance Request from you. The
highest-leverage non-engineering move available is delegating acceptance
classes and batching your decisions (§8).

---

## 1. Where you actually are

Genuinely true this week, per the repo's own evidence:

- **A production baseline exists.** `docs/runs/production-baseline-2026-07-15.md`
  records a known-good mono rev, digest-pinned runtime image, dashboard digest,
  and NixOS closure, proven with fresh-agent launch, real hosted chat, site
  publish, Telegram pairing, and a serial zero-mutation rollout of four named
  agents. The first training cohort ran against this.
- **Billing is one step from live.** Customer mode is on
  (`FC_DASHBOARD_RUNTIME_MODE="customer"`), the live price is pinned and
  CI-enforced (`scripts/check_stripe_price_contract.py`), sandbox acceptance
  is done. Outstanding: live audit + first real charge, by Paul.
- **The monorepo consolidation worked.** lat1 is the single app server
  (core, dashboard, Postgres, chat, sites, brain, search, runner, one Caddy
  edge); lat2 is the CI/build box; CI gates every PR with real-Postgres tests
  and devfinity smoke. This is real progress — the "scattered hosting"
  problem is now mostly a *legacy fleet* problem, not a SaaS problem.
- **Recovery is built but never proven, and coverage is partial.**
  Snapshots + encrypted Borg archives exist for the hosted-chat set; the
  empty-target restore has never been run (`docs/runs/hosted-web-chat-disaster-recovery.md`).
  The 15-minute snapshot timer was removed 2026-07-14 (fencing broke live
  chat); snapshots now happen at deploy time or manually, and the health
  check tolerates 7 days of staleness (`infra/nixos/modules/backups.nix:99-126`).
- **lat1 is one disk.** mdadm failed at cutover; two spare NVMes sit unused;
  Postgres dumps land on the same disk as the live DB and their offsite borg
  repo is "credentials, deployment, first archive, and restore proof are not
  complete" — the runbook itself calls enabling it "the highest-priority
  infra follow-up" (`infra/runbooks/postgres-backup-restore.md:20-27`).
- **Legacy is a blind spot by design.** box1/TRF/clawland (~50 agent
  namespaces, ~240 published endpoints, old insecure sites, manual OpenRouter
  billing) are run by the legacy repo, deliberately outside mono
  (`docs/monorepo-doctrine.md:42`). Migration is the only exit.

---

## 2. Product-line triage

Your writeup, mapped to verdicts:

| Product | State per repo | Verdict | Next gate |
|---|---|---|---|
| **Private** (Tinfoil inference) | Live; limiter validates against lat1 Core; relaunch script still in legacy repo, ~35 min downtime | Maintain | Move relaunch script into mono; don't touch the model until revenue + DR are done — accept the upgrade-downtime CON for now |
| **Agent** (Kata runners) | Live, only runner class; secrets via `runtime-secrets.env`; launch path once took 238 s unexplained | Invest (reliability) | Launch-phase observability; Kata smoke in CI |
| **Specialization** (Austin) | Active verified checkpoint (AEON Gemma 4 12B); knowledge split across 3 repos; canaries pinned after 07-15 mitigation | Maintain | One doc giving the whole picture; audio still blocked on Hermes capability |
| **Private Agent** (Phala TEE) | Dark worker; readiness doc is blunt about the old CLI experiment; plan is a typed adapter + two closed tiers | **Defer** | Hard-gated on Stripe live + recovery proof + independent restore gates (`docs/runs/phala-confidential-runner-readiness.md`) |
| **Sites** | In production; best debt ledger in the repo; caching fix shipped 07-15 | Maintain | Recovery set for `/var/lib/finite-sites`; owner-key recovery path (ledger item 6) |
| **Brain** (Austin) | Integration merged (PR #70) but disabled; readiness run open, expires 07-22 | Fix the gate, then ship | fbrain release + Identity Authority on lat1 + joint deploy acceptance (`docs/runs/hosted-brain-production-readiness.md`) |
| **Identity** | v1 contract live, consumed everywhere; Authority not on lat1 | Invest (small) | Deploy Authority to lat1 (unblocks Brain); key-loss recovery is a declared launch gap (SPEC.md:6) |
| **Search** (Austin) | Live on lat1 (SearXNG + Firecrawl; Firecrawl API documented DOWN); own investigation says "not yet proven as a production dependency" | Maintain, low spend | Fix or fence Firecrawl; don't sell it as a capability yet |
| **Chat protocol** | Server healthy; incident cluster was bridge/presentation, not protocol | Invest (close loops) | One-turn-late final message repro; SecretReuseError closure |
| **Dashboard** | The surface that matters; 201 unit tests; pinned 2026-07-16.1 | Invest | npm audit baseline (12 transitive findings); Brain nav stays hidden until readiness |
| **Chat iOS** | Built, TestFlight runbook exists; phone harness not a passing proof; server-deployment gate unmet | **Defer** (small pushes only) | Physical-phone matrix pass; deployed-server gate |
| **Chat Electron** | PARKED 2026-07-11 after it broke prod web UI | **Defer** | Resume only via a blessed run reusing the protected web surface |
| **Mono** | Migration done; Phase 13 stale-doc purge never happened; devfinity undocumented | Invest (cheap) | Doc purge + devfinity README + compat-matrix CI enforcement |
| **Legacy** (box1/TRF/OpenClaw) | Outside mono, security-vulnerable old sites, manual billing | Migrate — but sequenced (§5) | SaaS stable + recovery proof + retirement path first |
| **Agent Camp** (Skyler) | Revenue + product intelligence | Keep | Feed camp findings into the company brain (§7) |

---

## 3. P0 — the only-Paul list (this week)

Everything here is blocked on you personally, per the repo's own docs.
Ordered by leverage:

1. **Run the Stripe live acceptance.** Inspection-only live audit, then the
   first real $200 charge from a fresh public signup, webhook → Core → one
   agent (`docs/runs/stripe-production-activation.md`). *Unblocks:* paid
   cohorts, self-serve revenue, trial-mode design, the Phala paid tier, and
   honest runway math. Effort: an afternoon. Deadline: 2026-08-03.
2. **Name the DR drill target and key custody.** The empty-target restore is
   fully scripted and locally proven (306 ms synthetic restore, six negative
   attempts refused); it's blocked on you naming an isolated target, a
   network fence, and confirming independent Borg passphrase custody
   (`docs/runs/hosted-web-chat-disaster-recovery.md`,
   `docs/runs/overnight-cleanup-report.md` item 2). A cheap OVH VPS or a
   fenced lat2 namespace works. *Unblocks:* the recovery proof that ADR 0001
   makes a prerequisite for TEE claims, Phala, Brain durable use, and any
   marketing language. Effort: one decision + one acceptance session.
3. **Authorize completing the Postgres offsite backup.** The Nix definition
   and rsync.net repo already exist; credentials, first archive, and restore
   proof are incomplete, and today the dumps share a disk with the live DB
   (`infra/runbooks/postgres-backup-restore.md:20-27`). That DB holds
   entitlements, billing state, and the 87 Finite Private keys. *Unblocks:*
   the single sharpest data-loss risk in the company. Effort: say "do it"
   + acceptance.
4. **Confirm the 2026-07-09 key rotations happened** (`FINITE_PRIVATE_API_KEY`,
   `OPENAI_API_KEY`, `FAL_KEY`) — parked off-repo since 07-13
   (`docs/runs/parking-lot.md`). Effort: minutes.
5. **Decide the two image-size tradeoffs.** Telegram messaging extra +12.8 MB
   (5.85%), `gws` preinstall +6.8 MB (3.23%) — both shipped, both parked on
   your acceptability call. The launch-time audit found no evidence `gws`
   slows cached starts. Recommendation: accept both. Effort: one reply.
6. **Give the Runtime Retirement policy inputs** — entitlement-release and
   retained-data semantics — or explicitly accept zombie accumulation for
   now. Retirement is advertised false everywhere, so Sol 2, Waffle, and the
   AEON/Sites canaries *cannot be removed*, and the production baseline
   forbids bypassing the gate. This blocks both hygiene and the legacy
   migration's endgame. Effort: a paragraph of decisions.
7. **Confirm the Google OAuth scope change stays batched with the legacy
   migration.** Parking-lot (07-14) deliberately supersedes the overnight
   report's "do it now" — the console change force-disconnects every existing
   Workspace connection, so it should ride the migration re-onboarding. Just
   close the contradiction so nobody acts on the stale instruction.

Total: roughly one focused day of your week, and it unblocks revenue,
recovery, security hygiene, and three parked engineering threads.

---

## 4. P1 — delegate these (suggested owners)

You should do *none* of this yourself. Each item is scoped to be handed off
with an acceptance gate:

**Recovery & data availability** (Alex, run-governed)
- Extend the off-host recovery set beyond hosted chat: Brain SQLite, Sites
  `/var/lib/finite-sites`, agent `/data` (all three explicitly uncovered today:
  `infra/runbooks/deploy-brain.md:176`, `infra/runbooks/deploy-sites.md:36`,
  `infra/README.md:142`).
- Run the Postgres restore drill (scripted, ~10 min, read-only vs prod) and
  the empty-target chat drill once you name the target.
- Restrict the rsync.net credential to append-only (currently accepts
  arbitrary remote commands — accepted debt, cheap to fix).
- Use the two spare lat1 NVMes (mirror or at least on-box second-copy) —
  single-disk root is a named gap since cutover.
- Delete the stale root-only `runner.env` backup sitting outside `/etc`
  (`infra/hosts/lat1/README.md` appendix item 5).

**Reliability loops** (Austin or Alex)
- Reproduce/close the "final message one turn late" symptom — the 4-point
  trace is already written (`docs/audits/finite-chat-tool-ordering-2026-07-15.md`).
- Close the loop on the MLS `SecretReuseError` pairing bug from the 07-06
  postmortem — recent ordering fixes are suggestive but no doc confirms it.
- Launch-path phase markers at the four orchestration boundaries — the one
  238 s launch is unattributable without them
  (`docs/audits/launch-time-audit-2026-07-13.md`). Observability-only, cheap.
- **Paging:** the healthcheck yells into journald; the dead-man's switch is a
  TODO (`infra/nixos/modules/monitoring.nix:6-8`). Smallest item on this
  list, biggest sleep-quality return.

**CI & release hardening** (Alex)
- Kata-path SaaS smoke in CI (currently only Apple Container locally; the
  upgrade-acceptance gap from 07-15 lives here).
- Re-enable the runtime-image smoke on push to main once it has green
  history; today it's dispatch-only on the single lat2 runner.
- Enforce `compat/matrix.toml` updates in CI (it already lags the deployed
  dashboard pin).
- Fix the possibly-vacuous Hermes CI wrapper that synthesizes a passing
  report when the hook is absent (`finite-agentd/README.md:106`,
  `docs/open-questions.md`) — this is the exact "overclaimed demo" failure
  mode the slop audit warns about.

**Doc & code hygiene** (Austin — this is your "coding agents get
overwhelmed" CON, made concrete)
- Run doctrine Phase 13 (stale-doc purge): `architecture-overview.md`,
  `system-flow-and-trust-boundaries.md`, and `docs/README.md` still describe
  the *superseded* hot-swap skills model re-decided in ADR 0002; several
  imported docs carry "not revalidated" banners. Contradictory docs are how
  agents (and new hires) break things confidently.
- Write `devfinity/README.md` — the CI gate has zero self-documentation.
- Dashboard npm audit baseline (12 transitive findings, 3 high).
- Promote the four debt ledgers into `docs/` nav per the slop audit; keep
  the "delete conditions" discipline — it's working (see: `finite-auth`,
  imported then deleted when retired).

**Brain launch** (Austin owns; your role is acceptance only)
- The readiness run expires 2026-07-22 and names three blockers: no matching
  `fbrain` release, prod dashboard Brain UI ≠ validated UI, and no Identity
  Authority on lat1 (`docs/runs/hosted-brain-production-readiness.md`).
  Sequence: Authority deploy → fbrain release → joint acceptance → re-enable
  nav. Note Brain's own README makes recovery principals a durable-use
  blocker — pair this with the recovery work above.

---

## 5. Sequencing — what gates what

```
Stripe live charge (P0-1)
   └─► paid cohorts, trial modes, white-glove one-click billing
   └─► Phala paid tier (hard prerequisite per its readiness doc)

Recovery proof + coverage (P0-2/3, P1)
   └─► honest privacy/marketing claims (ADR 0001 ordering)
   └─► Phala/TEE anything
   └─► Brain durable SaaS use
   └─► legacy migration (don't move users onto a platform you can't restore)

Runtime retirement policy (P0-6)
   └─► zombie cleanup, migration endgame, Phala delete semantics

SaaS stable + recovery proven + retirement defined
   └─► LEGACY MIGRATION (box1/TRF/OpenClaw, ~240 endpoints)
        ├─ batch the Google OAuth re-consent here (P0-7)
        ├─ "Migrate me to Finite SaaS" skill/runbook as the vehicle
        └─► then: archive legacy repo, consolidate lat2 runners,
            retire box1/clawland blind spot and old-sites vulns
```

The legacy migration is your biggest operational risk-reducer and billing
simplifier, but doing it before recovery and retirement are proven moves
users from a known-bad platform to an unproven one. Hold the sequencing.

---

## 6. Defer, explicitly

- **Phala Private Agent** — expensive, scary recovery story, and gated
  behind Stripe + recovery proof by its own readiness doc. Keep the worker
  dark. Revisit after first paid cohort.
- **Electron** — parked for cause (broke prod web). It reuses the web
  surface; resume only after that surface is stable and the resume run
  forbids changing it.
- **iOS app** — gates unmet (phone harness, server deployment). Small pushes
  only; App Review is not this quarter's battle.
- **Nous Tool Gateway replacement / Finite Drive / org connections
  (Slack, Salesforce…)** — your edges-first instinct is right: let Alex,
  Brandon, and Austin build these as forward-deployed custom solutions for
  HRF/NED, and pull proven primitives into mono. Don't design the general
  system yet. Exception: Search is already 80% of web_search/web_extract —
  fixing Firecrawl is maintenance, not the gateway project.
- **SOC 2** — but note: your runs/readiness/acceptance-request culture is
  genuinely the raw material. Keep the discipline; formalize later.
- **Splitting services off lat1** — consolidation was deliberate and recent.
  The risk was never "everything on lat1" per se; it's single-disk +
  no-drill + no-paging. Fix those three before re-litigating topology. And
  don't wire lat2 as an extra runner — it's your only builder and CI
  capacity; keep its role clean.

---

## 7. Your open questions, answered from the repo

- **Slop whack-a-mole.** The repo already has the strategy; it's just not
  finished: the devfinity saas-smoke (real Hermes, restart healing), the
  finitechat Product-State Harness, contract checks in CI, and debt ledgers
  with delete conditions. The gaps: Phase 13 doc purge (contradictory docs
  → confident wrong changes), the possibly-vacuous CI wrapper, and the
  oversized-module list in `docs/slop-audit.md` (`finitechat-core/lib.rs`
  >13k lines, etc.). Split those only when they block comprehension — the
  audit itself says so. "Tame the beast" = finish the harness culture +
  delete with conditions, not more tests for their own sake.
- **Notifications to users.** You already pay for and integrate Resend/
  Postmark (Identity Authority mailers). Transactional email (feature
  advisories, billing) is a small feature on existing plumbing, plus an
  in-dashboard banner. Put it on the post-Stripe product list, not before.
- **API keys everywhere.** Step 1 is a doc, not a system: one
  `infra/` inventory of every third-party account (name, console URL, which
  host/env file consumes it, top-up cadence, low-balance alert owner).
  Names and locations only — fits your public-repo rule. Step 2 (calendar
  reminders or balance probes) after the inventory exists. This is a
  one-hour task that removes a whole class of surprise outages.
- **Hosting scatter.** Mostly solved for the SaaS (§1). What remains: the
  legacy fleet (migration fixes), the smoke box (no backups, 82% disk —
  either back it up or retire it once Brain's rollback need ends), and
  Firecrawl (down). No new boxes.
- **Billing for Skyler.** Stripe live (P0-1) is 80% of it. Then: white-glove
  one-click invoice/seat-add as a small Core+dashboard feature, and the
  2-week/3-month trial modes as entitlement states — design them *after* the
  first real charges, when you have data.
- **Incorporation / salaries / runway.** Not a repo problem, but: Stripe
  live + a trivial revenue export (the `reporting` repo already exists for
  this class of thing) gives the team the honest dashboard you want. Ship
  revenue first, dashboard second, incorporation paperwork in parallel with
  someone who isn't an engineer.
- **Company brain.** Don't build new tooling. Dogfood finite-brain the
  moment its readiness run closes — that *is* the product finding its own
  use case — and seed it with this doc, `open-questions.md`,
  `parking-lot.md`, and camp notes. The agent-operated bug-fix/billing/
  comms flows come later, built on finite-agentd's command surface, which
  is exactly what ADR 0003 designed it for.

---

## 8. Operating rhythm — how to stop being the bottleneck

1. **Delegate acceptance by class.** Today every run's Acceptance Request
   names you. Keep that for: anything touching money, anything touching
   recovery/deletion, first-of-class external mutations. Delegate to named
   owners: deploy-only pins → Alex; brain/search/identity runs → Austin.
   Write the matrix into `docs/runs/README.md` so it's durable.
2. **Batch your decisions.** The parking lot is full of 5-minute Paul
   decisions (§3). A standing weekly 30-minute "acceptance window" clears
   them instead of each becoming a multi-day block.
3. **Make the doc purge a priority, not hygiene.** In a repo where coding
   agents do much of the work, stale contradictory docs are an active
   reliability hazard — this is the mono CON you named, with a cheap fix.
4. **Keep the honesty culture.** The most valuable thing in these docs is
   the repeated refusal to overclaim (echo-agent audit, "green timers are
   not recovery proof", `runtime_upgrade=false` caveats). That's what makes
   everything else in this plan trustworthy. Protect it as the team grows.

---

## Appendix — top risks, ranked

| # | Risk | Evidence | Mitigation |
|---|---|---|---|
| 1 | lat1 disk loss takes the company: Postgres (87 keys, billing), chat, brain, sites, agent data | `infra/runbooks/postgres-backup-restore.md:20-27` | P0-2/3 + P1 recovery set + spare NVMes |
| 2 | Recovery never proven end-to-end; backups could be silently unrestorable | `docs/runs/hosted-web-chat-disaster-recovery.md` | P0-2 drill |
| 3 | No paging — failures found by users | `infra/nixos/modules/monitoring.nix:6-8` | P1 dead-man's switch |
| 4 | Revenue blocked on one un-run acceptance | `docs/runs/stripe-production-activation.md` | P0-1 |
| 5 | Paul SPOF: acceptance, deploys (Mac + agent forwarding), 2/3 of commits | git log; `docs/runs/README.md` | §8 |
| 6 | Legacy fleet: old sites vulns, manual billing, no mono runbooks | `docs/monorepo-doctrine.md:42` | §5 migration sequencing |
| 7 | Zombie agents accumulate; no retirement path | `docs/runs/production-baseline-2026-07-15.md` | P0-6 |
| 8 | Possibly-vacuous CI evidence on the agent boundary | `finite-agentd/README.md:106` | P1 CI hardening |
| 9 | Secrets sprawl: runtime env stuffing, broad rsync.net cred, stale env backups | `infra/runbooks/hosted-web-chat-recovery.md:26-32` | P1 + §7 inventory |
| 10 | lat2 is sole builder/CI/deploy path | `infra/hosts/lat2/README.md` | Accept for now; document rebuild procedure |
