# GLM-5.3-Flash production cutover ledger

## Run

- Run ID: `glm-5-3-flash-production-cutover-2026-08-27`
- Loop: Feature Dev, ending at a draft preparation PR by explicit operator request
- Target repo: `finitecomputer/finite-mono`
- Base branch: `main` (this repository has no `staging` branch)
- Feature branch: `codex/glm-5-3-flash-cutover-prep`
- Human owner: Finite operator
- Started: 2026-08-27
- Current status: preparation; no production mutation authorized or performed
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
- Review packets: pending final two-axis review
- Local CodeRabbit report: pending
- PR URL: <https://github.com/finitecomputer/finite-mono/pull/721> (draft by
  explicit operator request)

## Accepted Testing Seams

These are the preparation defaults pending any operator adjustment:

1. Limiter HTTP seam: `glm-5-3-flash`, `deepseek-v4-flash-0731`, and
   `glm-5-2` all reserve, route to SGLang as `glm-5-3-flash`, stream terminal
   usage, and settle; an unknown model fails closed before reservation.
2. Candidate manifest seam: repository validation proves fixed checkpoint and
   base-image identities, eight H200s, private inference networking, parsers,
   context cap, model-routing variables, and the distinction between safe prep
   placeholders and release-ready pins.
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
| Pin SGLang/checkpoint and create Tinfoil candidate | AFK | Complete | Pending | Pending | Preparation contract green; release contract blocked as designed |
| Preserve mixed-version model labels through the limiter | AFK | Complete | Pending | Pending | 16 limiter tests green |
| Add 120-user capacity, latency, and quality gates | AFK | Complete | Pending | Pending | 8 Python gate tests green |
| Prepare `finite-private` rename with historical-route bridge | AFK | Complete | Pending | Pending | Candidate contract and ops tests green |
| Write 03:00 cutover and rollback procedure | AFK | Complete | Pending | Pending | Static contract green |
| Generate Tinfoil MPK and publish measured image pins | HITL/production access | Parked | N/A | Operator/Tinfoil action | No |
| Execute production replacement and live load gate | HITL/production mutation | Parked | N/A | Explicit operator action | No |

## Parked HITL Slices

| Slice | Why parked | Blocks | Required human action | Draft PR decision |
| --- | --- | --- | --- | --- |
| Generate modelwrap MPK | Requires Tinfoil Models access and creates an external artifact | Release-ready candidate | Generate from the pinned checkpoint and record MPK/root hash | Remains an explicit placeholder |
| Publish limiter and SGLang wrapper images | Mutates GHCR and production tags | Release-ready candidate | Dispatch reviewed workflows and record immutable amd64 digests | Preparation workflow is included |
| Publish satellite releases | Creates external release state | Tinfoil relaunch | Review exact satellite commits and publish artifacts | Commands only |
| Replace the live eight-H200 container | Causes planned inference downtime | Production cutover | Explicitly execute the 03:00 runbook | Out of draft PR execution scope |

## Issue Session Ledger

| Slice | Fixed point | Worker | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| Preparation PR | `9f9c81db6a991665bcc57f4bb7c26cc0b71dfe78` | Primary Codex worktree | Pending final commit | Pending | `cargo test -p finite-private-limiter --locked`; 23 Python tests; candidate prep contract; YAML/shell syntax; `git diff --check` |

## Open Questions

- Whether the proposed 20 tok/s p50 and 10 tok/s p10 120-user thresholds match
  the operator's intended meaning of "decent output speeds."
- Whether Tinfoil organization access can publish the model MPK and both
  satellite releases before the maintenance window.

## Escalations

- The old generated `kimi-k2-6.finite.containers.tinfoil.dev` hostname cannot
  move with the GPU container. The safe one-window rename therefore requires a
  separately measured CPU-only compatibility bridge at the historical name,
  or an already-verified stable custom domain. Directly replacing the hostname
  would break issued Runtime readers.
