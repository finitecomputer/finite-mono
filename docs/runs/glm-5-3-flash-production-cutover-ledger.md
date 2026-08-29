# GLM-5.3-Flash production cutover ledger

## Run

- Run ID: `glm-5-3-flash-production-cutover-2026-08-27`
- Loop: Feature Dev, ending at a draft preparation PR by explicit operator request
- Target repo: `finitecomputer/finite-mono`
- Base branch: `main` (this repository has no `staging` branch)
- Feature branch: `codex/glm-5-3-flash-cutover-prep`
- Human owner: Finite operator
- Started: 2026-08-27
- Current status: first execution window (2026-08-28 02:31 America/Chicago)
  stopped no-go before any production mutation; all published artifacts
  verified reusable; retry pending runbook-exception review and operator
  scheduling
- Skill setup status: complete (`docs/agents/issue-tracker.md`,
  `docs/agents/triage-labels.md`, and `docs/agents/domain.md` are present)

## Goal

Prepare a production attempt for GLM-5.3-Flash on Finite Private's single
eight-H200 Tinfoil allocation, targeting the maintenance window beginning at
03:00 America/Chicago on 2026-08-28. The candidate must prove useful quality,
correct protocol behavior, and at least 120 simultaneous users at acceptable
interactive decode speed before it can remain live. The inference container
must adopt the model-independent `finite-private` identity while the model
continues to identify itself as `glm-5-3-flash`.

Preparation must not relaunch, drain, update, route, load-test, or otherwise
disturb the currently serving `kimi-k2-6` production container.

## Durable Artifacts

- Context updates: Tinfoil inventory, routing migration, candidate READMEs, and
  image catalog updated without changing production defaults
- ADRs: none; the stable Finite Private identity is already recorded in
  `infra/runbooks/finite-private-routing-migration.md`
- Prototype source branch: not required
- Research:
  `docs/research/2026-08-27-glm-5-3-flash-eight-h200-assessment.md`
- Spec/plan: this ledger and the GLM-5.3-Flash cutover runbook
- Tickets: preparation slices are tracked below; production execution remains
  an explicit operator handoff
- Ticket sessions: local implementation in the isolated worktree
- Agent briefs: none
- Review packets: two-axis review complete. Final Standards review found no hard
  violation; Spec review findings were fixed in follow-up commits.
- Local CodeRabbit report: not run; independent two-axis reviewers covered the
  full branch and the post-fix diff
- PR URL: <https://github.com/finitecomputer/finite-mono/pull/721> (draft by
  explicit operator request)

## Accepted Testing Seams

These are the preparation defaults pending any operator adjustment:

1. Limiter HTTP seam: `glm-5-3-flash`, `deepseek-v4-flash-0731`, and
   `glm-5-2` all reserve, route to SGLang as `glm-5-3-flash`, stream terminal
   usage, and settle; an unknown model fails closed before reservation.
2. Candidate manifest seam: repository validation asserts unique fixed
   checkpoint and base-image identities, eight H200s, private inference
   networking, parsers, context cap, model-routing variables, and the
   distinction between safe prep placeholders and release-ready pins. Tinfoil's
   decoded deployment validation remains the schema and resource authority.
3. Capacity CLI seam: the hard 120-client gate requires 120/120 successful
   terminal streams, p50 decode at least 20 output tokens/sec, p10 decode at
   least 10 output tokens/sec, aggregate output at least 2,400 tokens/sec, and
   p95 time to first token no more than 10 seconds.

## Commands

- Install: dependencies are provided by the root Nix flake; do not install on
  the host
- Typecheck: `just computer check`
- Targeted Rust test:
  `scripts/with-dev-env cargo test -p finite-private-limiter --locked`
- Targeted Python tests:
  `scripts/with-dev-env python -m unittest scripts.tests.test_finite_private_glm53_candidate`
- Candidate contract:
  `scripts/with-dev-env python scripts/check_finite_private_glm53_candidate.py`
- Full test: `just ci`
- Build: GitHub image workflows only; nothing is built on a production host
- Visual verification: not applicable

## Preparation Ledger

| Slice | Type | Status | Review | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| Pin SGLang/checkpoint and create Tinfoil candidate | AFK | Complete | Pass | None | Preparation contract green; release contract blocked as designed |
| Preserve mixed-version model labels through the limiter | AFK | Complete | Pass | None | 16 limiter tests green |
| Add protocol, 120-user capacity, latency, quality, and blind comparison gates | AFK | Complete | Pass | None | 20 Python gate tests green |
| Prepare `finite-private` rename with historical-route bridge | AFK | Complete | Pass | None | Candidate contract and ops tests green |
| Write 03:00 cutover and rollback procedure | AFK | Complete | Pass | None | Static contract and post-fix review green |
| Generate Tinfoil MPK and publish measured image pins | HITL/production access | Complete | Pass | None | MPK root `54b2859a…5ed79`; images `.3` pinned by digest (see attempt record below) |
| Execute production replacement and live load gate | HITL/production mutation | Parked | N/A | Explicit operator action | No |

## 2026-08-28 02:31 window attempt (no-go before mutation)

The authorized window opened at 02:31 America/Chicago (the runbook's 02:30
start; earlier ledger references to 03:00 are superseded). Timeline:

- Preflight clean; readiness-fix PR #722 merged. Both production images built
  from commit `b91eea86`; the service-image workflow's summary step failed on
  a missing `fi`, fixed by PR #723, and both images republished as
  `2026-08-28.3` from commit `35834e7e` with verified digests.
- `confidential-finite-private` satellite tagged and measured from commit
  `72ef45ed`. Release `v2026-08-28-glm-5-3-flash-1` (deployment hash
  `4c90778f…a7a06`); compatibility bridge release
  `v2026-08-28-glm53-compatibility-bridge-1` (deployment hash
  `da6fad29…eab9`). Candidate config SHA-256 `91fe432e…cc9d35`, committed to
  this repository on this run's retry-preparation PR.
- Canonical `scripts/finite-status` from `finite-lat-1` returned
  `fleet_convergence` red/unknown twice: all 51 active Runtimes readiness
  `unknown` (never reported), and the distribution aggregate disagreed with
  the detail snapshot. The runbook then carried no named exceptions, so the
  window stopped before the first Tinfoil mutation. DeepSeek was never
  replaced; no rollback was necessary.

Post-attempt root cause (both findings pre-existing, not regressions):

- Readiness `unknown` everywhere: the standing-readiness ferry registers a
  report target only on fresh launch/relocation completion; the fleet was
  upgraded in place on 2026-08-27, so no current Runtime can ever report.
  Tracked for repair separately from this cutover.
- Aggregate/snapshot mismatch: inner join vs left join on
  `runtime_artifacts`; explained by the documented artifact-less `smoke` row.

DeepSeek remained production-healthy throughout and passed the full direct
pre-cutover suite (protocol canaries, 32-way baseline 32/32, p50 TTFT 0.144s,
quality 10/10, six-case blind reference capture). All published artifacts
were re-verified on 2026-08-28: both release deployment JSONs match the
retained evidence, and both pinned GHCR digests resolve anonymously. The
retry needs the runbook's named-exceptions amendment reviewed, the retained
before-report baselines re-captured fresh, and an authorized window.

Retained evidence: `.local-state/glm53-cutover-2026-08-28/` (operator
worktree `finite-mono-glm-5-3-flash-cutover`; not in git).

## 2026-08-28 evening execution (authorized, product outage in progress)

The operator authorized immediate execution during an unrelated product
outage: live traffic was zero, so the swap's maintenance cost was nil. State
at time of writing:

- GPU replacement executed exactly per runbook: `finite-private` created
  (container `52cd8373-fb40-485b-be68-d35bfcbfdb5a`) consuming rollback ID
  `a1220ca5…`, bridge created at the historical name. DeepSeek rollback
  identity unchanged and restorable (~35-45 min readiness).
- **GLM-5.3-Flash is `ready` on 8×H200**: `/live`, `/health` 200, release
  `v2026-08-28-glm-5-3-flash-1`, config sha `91fe432e…cc9d35`.
- Qualification gates are blocked by the unrelated outage: the limiter
  returns 503 `usage_api_unavailable` for every request (usage admission
  down), so protocol/quality/capacity/settlement gates cannot pass for any
  model. A watcher polls admission and will run the canary battery the
  moment it recovers.
- Bridge (historical `kimi-k2-6` name) is the one open item: its first-ever
  live deploy surfaced three defects the review-and-measure process could
  not catch, because measurement hashes bytes and never boots: (1) network
  name `finite-private-upstream` exceeded the 15-char interface limit
  (renamed `fp-upstream`); (2) short start periods fail the container before
  GLM finishes loading; (3) even with upstream live, proxy/admin/wget
  healthchecks in `caddy:alpine` all fail on the platform's health model at
  ~45s while identical shapes pass in the GPU images (curl, own process
  port). Releases `.2`-`.5` cut on
  `codex/glm53-compatibility-bridge`; `.5` (Caddyfile, dedicated :8888
  liveness endpoint) is the current best config. Parked for a daylight
  debugging session; the historical name was already dark due to the outage,
  so no additional availability was lost.

## Parked HITL Slices

| Slice | Why parked | Blocks | Required human action | Draft PR decision |
| --- | --- | --- | --- | --- |
| Generate modelwrap MPK | Requires Tinfoil Models access and creates an external artifact | Release-ready candidate | Generate from the pinned checkpoint and record MPK/root hash | Complete 2026-08-28; root pinned in candidate |
| Publish limiter and SGLang wrapper images | Mutates GHCR and production tags | Release-ready candidate | Dispatch reviewed workflows and record immutable amd64 digests | Complete 2026-08-28 as `2026-08-28.3` |
| Publish satellite releases | Creates external release state | Tinfoil relaunch | Review exact satellite commits and publish artifacts | Complete 2026-08-28; both releases verified |
| Replace the live eight-H200 container | Causes planned inference downtime | Production cutover | Explicitly execute the 02:30 runbook | Out of draft PR execution scope |

## Issue Session Ledger

| Slice | Fixed point | Worker | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| Preparation PR | `9f9c81db6a991665bcc57f4bb7c26cc0b71dfe78` | Primary Codex worktree | `3a3f5d28` plus final review fixes | Standards: no hard violations; Spec: all material findings fixed | `cargo test` and clippy for limiter; 29 Python tests; candidate prep/release-stop contracts; YAML/shell syntax; `git diff --check` |

## Remaining release blockers

- The 120-user definition of "decent" is fixed for this candidate at 20 tok/s
  p50, 10 tok/s p10, 2,400 aggregate output tok/s, and 10-second p95 TTFT. It
  may be raised before publication; lowering it requires a reviewed PR and a
  new operator decision.
- The runbook's named pre-existing fleet exceptions (readiness reporting gap
  and aggregate/snapshot mismatch) must be reviewed and accepted by the
  operator before the retry window; without them the entry gate fails exactly
  as it did on 2026-08-28.

## Escalations

- The old generated `kimi-k2-6.finite.containers.tinfoil.dev` hostname cannot
  move with the GPU container. The safe one-window rename therefore requires a
  separately measured CPU-only compatibility bridge at the historical name,
  or an already-verified stable custom domain. Directly replacing the hostname
  would break issued Runtime readers.
