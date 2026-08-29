# GLM-5.3-Flash production cutover ledger

## Run

- Run ID: `glm-5-3-flash-production-cutover-2026-08-27`
- Loop: Feature Dev, ending at a draft preparation PR by explicit operator request
- Target repo: `finitecomputer/finite-mono`
- Base branch: `main` (this repository has no `staging` branch)
- Feature branch: `codex/glm-5-3-flash-cutover-prep`
- Human owner: Finite operator
- Started: 2026-08-27
- Current status: GLM-5.3-Flash live on `finite-private` under temporary
  degraded admission (`v2026-08-28-glm-5-3-flash-4`, container
  `2aa4d230-0675-4c4a-a7b3-07776b24bfad`). Serving is the H200 DSA pair plus
  chunked prefill 16384. Wire name is hyphenated `glm-5-3-flash` (dotted
  `glm-5.3-flash` is a 400). 393,216 context is live-proven. Issued Runtime
  readers still point at the retired `kimi-k2-6` hostname. See
  `docs/runs/glm-5-3-flash-degraded-admission.md`.
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

Operator decision (same evening, authorized): single-container topology —
the failed bridge container was deleted rather than further iterated, so the
historical `kimi-k2-6` hostname is retired until readers migrate. The fleet
now runs exactly one live container: `finite-private` (GLM-5.3-Flash, ready).
Rollback is unaffected: it recreates DeepSeek from release
`v2026-08-13-deepseek-v4-flash-0731-128-2048-1` under the historical name via
`--replace`. The runbook's historical-route proof step is waived by operator
authorization for this window; migrating issued Runtime readers to the
stable `finite-private` endpoint is the follow-up that makes the retirement
permanent.

## 2026-08-28 degraded admission overlay (authorized)

The product edge still 307'd `POST /internal/finite-private/v1/reservations`
to the homepage, so every request 503'd `usage_api_unavailable` before a
GPU was touched. Operator chose option 2: an env-gated allowlist mode in
the limiter (PR #746), measured as `v2026-08-28-glm-5-3-flash-2`, rather
than waiting on the outage.

Trade-off (full write-up:
`docs/runs/glm-5-3-flash-degraded-admission.md`):

- Only keys in the Tinfoil secret `FINITE_ADMISSION_ALLOWLIST` are
  admitted. No reservation, no settlement; tokens in this mode are
  unaccounted. Every response carries `x-finite-admission: degraded-allowlist`.
- GLM checkpoint, SGLang pin, and MPK are identical to `flash-1`.
- Revert is one `--replace` back to `v2026-08-28-glm-5-3-flash-1` (or a
  later measured usage-api tag). The limiter defaults to usage-api when
  `FINITE_ADMISSION_MODE` is unset. Overlay config lives beside the
  candidate and is not the production default:
  `infra/tinfoil/confidential-finite-private/tinfoil-config.glm-5.3-flash.degraded-allowlist.yml`.

Container `197d6a7b-f7a3-458c-bfa6-613f49e0e7cd` created 2026-08-29
01:52 UTC consuming `52cd8373…` (`flash-1`). Ready 2026-08-29 02:20 UTC.
Listed-key canary 200 with `x-finite-admission: degraded-allowlist`;
unlisted key 401. Rollback to DeepSeek
`v2026-08-13-deepseek-v4-flash-0731-128-2048-1` is unchanged.

First speed numbers (see degraded-admission doc for the tables): 1-way
~88–90 tok/s at 0.3–0.5s TTFT; 32-way thinking-on 57 tok/s per request
but 33s TTFT and 218 aggregate tok/s. The 120-user gate's 10s p95 TTFT
and 2,400 aggregate bars are not in reach on this topology without a
separate candidate.

## 2026-08-28 flash-3 DSA + thinking-high (authorized)

Operator authorized a one-mutation retune: keep degraded admission, swap
TileLang DSA for the LMSYS-measured H200 pair (`flashmla_sparse`/`fa3`),
and fill omitted `reasoning_effort` with `high`. Did not add
`--disable-shared-experts-fusion` (later cookbook dropped it; live
`flash-2` already answered without it). Did not add MTP or retune
`--mamba-full-memory-ratio`.

- Limiter image `2026-08-28.6` from branch SHA `06a538b2` (linux/amd64
  manifest `sha256:47463982…23461`).
- Satellite `v2026-08-28-glm-5-3-flash-3` from
  `confidential-finite-private@c8533b5`. Deployment hash
  `164d4b8fef024823fc9a451c9634be4dea669f1f73746f5eff38e97a51ce3043`.
- `--replace` consumed `197d6a7b…` (`flash-2`). New container
  `fa79c9b9-551c-4307-9ee0-cba2e5662e2d` created 2026-08-29 02:57 UTC,
  ready 03:25 UTC. `/live` reports `defaultReasoningEffort=high`.
- Diagnostic 1/32-way vs `flash-2`: decode +4–8%; 32-way thinking-on TTFT
  still 33.8s (was 33.1s). Short-prompt load does not show LMSYS's 24k-prefix
  TTFT win. 120-user bars still out of reach.

## 2026-08-29 flash-4 chunked prefill + 392k proof (authorized)

Operator authorized one more `--replace` onto
`v2026-08-28-glm-5-3-flash-4`: same overlay, same limiter `.6`, add
`--chunked-prefill-size 16384`. Container
`2aa4d230-0675-4c4a-a7b3-07776b24bfad` is `ready` on `control.inf9.tinfoil.sh`.
The Kimi compatibility bridge stays deleted; the fleet has exactly this one
GPU container.

Load canary vs flash-3 (thinking-off, 64 completion tokens):

| concurrency | flash-3 TTFT p50 | flash-4 TTFT p50 | flash-3 aggregate | flash-4 aggregate | per-request p50 |
| --- | --- | --- | --- | --- | --- |
| 1 | 0.684s | **0.287s** | — | — | 96.8 tok/s |
| 32 | 15.70s | **15.13s** | 124.1 | **128.5** | 81.9 tok/s |

64-way was measured on this box earlier in the cutover (not re-run on
flash-4): 69.6 tok/s p50 per stream, 237.4 aggregate, still under the 90s
TTFT ceiling.

Full advertised context is live, not just configured. A needle probe of
**387,498 prompt tokens** (98.5% of the 393,216 cap) through the limiter
returned the needle on cold prefill in 21.3s and on the warm cached path in
2.5s. Protocol 11/12 through 128k; the remaining failure is the limiter's
pre-existing 502 on malformed JSON.

Wire-name gotcha, proven the same night: the limiter accepts only the
hyphenated id `glm-5-3-flash`. The dotted `glm-5.3-flash` (Z.ai product
spelling) is `400 unsupported_model`. The candidate now lists the dotted
form as an alias so copied health/docs names do not 400; the canonical
served name stays hyphenated.

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
  p50, 10 tok/s p10, 2,400 aggregate output tok/s, and 10-second p95 TTFT.
  flash-4 does not meet those bars. Lowering them requires a reviewed PR and
  a new operator decision.
- Usage admission on `finite.computer` is still missing, so the live
  container stays on the degraded allowlist overlay. Settlement and
  accounting gates cannot pass until that route returns a real 2xx/4xx.
- Issued Runtime / Hermes / Runner readers still default to the retired
  `kimi-k2-6` hostname and the DeepSeek model label. That is the stacked
  follow-up, not this serving PR.

## Escalations

- The old generated `kimi-k2-6.finite.containers.tinfoil.dev` hostname cannot
  move with the GPU container. The compatibility bridge was deleted by
  operator decision the same evening; the historical name is dark. New
  launches and existing Runtime env still need the reader cutover onto
  `https://finite-private.finite.containers.tinfoil.dev/v1`.
