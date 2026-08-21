# Tier 1 Documentation Reduction Backlog

Status: FOLLOW-UP QUEUE - NOT DELETION AUTHORITY

Opened: 2026-08-20

This records the docs left intentionally untouched by the stale-doc cleanup.
Tier 1 means "probably reducible after an owner/context check", not
"safe to delete now".

The goal of a later pass should be reduction: choose one authoritative home for
current facts, preserve useful rollout/postmortem evidence, and remove
superseded queues, duplicate plans, or stale local mirrors only after that
current home exists.

## Reduction Rules

- Do not delete ADRs, postmortems, recovery notes, or production runbooks
  simply because they are old.
- Convert live work into GitHub issues or a current run doc before deleting
  dated planning queues.
- Move durable policy into ADRs, doctrine, runbooks, or component architecture
  docs before deleting the investigation that discovered it.
- Mark historical evidence as historical when it still explains a rollout,
  incident, or production boundary.

## Root Planning And Queue Docs

Candidates:

- `docs/next-work-plans-2026-07-23.md`
- `docs/open-questions.md`
- `docs/triage-and-priorities-2026-07-17.md`
- `docs/runs/parking-lot.md`
- `docs/runs/overnight-cleanup.md`
- `docs/runs/overnight-cleanup-report.md`

Why reduce:

These are useful context, but they mix proposed work, resolved decisions,
historical queue state, and live policy. They should not remain the primary
place to discover current priorities.

Follow-up action:

Extract still-live items into GitHub issues or current run docs. Move durable
policy into ADRs, `docs/monorepo-doctrine.md`, or focused runbooks. Delete or
mark historical sections that are fully superseded.

Link/staleness signals observed:

- `docs/next-work-plans-2026-07-23.md` still points at old or renamed docs such
  as `finitecomputer-v2/docs/existing-user-import-bridge.md` and the old
  "personal vaults" ADR path.
- `docs/open-questions.md` points at missing or old run docs such as
  `docs/runs/production-canary.md`.

## Root Run Docs With Stale Status Risk

Candidates:

- `docs/runs/camp-saas-stability-2026-07-26.md`
- `docs/runs/electron-device-parity-alpha.md`
- `docs/runs/electron-workos-remote-dashboard-spike.md`
- `docs/runs/finite-lat-capacity-and-redundancy.md`
- `docs/runs/hosted-brain-production-readiness.md`
- `docs/runs/hosted-web-chat-disaster-recovery.md`
- `docs/runs/ios-existing-agent-app-store-activation.md`
- `docs/runs/lat1-lat3-sops-nix-inventory-baseline.md`
- `docs/runs/lat1-lat3-sops-nix-phased-migration.md`
- `docs/runs/managed-agent-identity-conformance.md`
- `docs/runs/nixos-sops-operator-flow.md`
- `docs/runs/phala-confidential-runner-readiness.md`
- `docs/runs/platform-reliability-checklist-2026-07-21.md`
- `docs/runs/production-baseline-2026-07-15.md`
- `docs/runs/runtime-retirement-readiness.md`
- `docs/runs/stripe-checkout-readiness.md`
- `docs/runs/stripe-production-activation.md`

Why reduce:

Several run docs are marked ACTIVE, PROPOSED, PAUSED, ACCEPTED, or COMPLETE
with dates in July 2026. Some are still valuable production evidence, but stale
status labels can mislead future operators.

Follow-up action:

For each run, decide one of: close with final outcome, convert remaining work
to issues, move durable operator steps into `infra/runbooks/`, or mark the doc
as historical evidence. Delete only the ones whose facts are captured elsewhere.

Link/staleness signals observed:

- `docs/runs/camp-saas-stability-2026-07-26.md` has an expired date and points
  at a renamed postmortem path.

## Imported Or Broad Orientation Docs

Candidates:

- `docs/system-flow-and-trust-boundaries.md`
- `docs/architecture-overview.md`
- `docs/navigation-plan.md`
- `docs/ci-gate-mvp-plan.md`
- `docs/devfinity-architecture-plan.md`
- `docs/identity-rollout-reconciled-plan.md`
- `docs/identity-rollout-test-log.md`
- `docs/lat-logs-and-host-metrics-plan.md`
- `docs/local-dev-matrix.md`
- `docs/local-integration-harness.md`

Why reduce:

These are broad orientation or rollout-plan docs. Some may still be useful,
but they overlap with the monorepo doctrine, root `AGENTS.md`, ADRs, and infra
runbooks. `docs/system-flow-and-trust-boundaries.md` explicitly says it was
imported and not fully revalidated after the monorepo import.

Follow-up action:

Keep one current architecture orientation path. Revalidate any security or
trust-boundary claims against current code and ADRs, then either update the doc
or replace it with links to authoritative component docs.

Link/staleness signals observed:

- `docs/system-flow-and-trust-boundaries.md` still contains relative links that
  resolve incorrectly from its current location.

## Root Dated Audits

Candidates:

- `docs/audits/*.md`
- `docs/slop-audit.md`
- `docs/ui-changes-notes.md`

Why reduce:

The audit docs are likely useful evidence, but many are dated point-in-time
findings rather than current operating guidance. They should not compete with
ADRs, runbooks, or active issue queues.

Follow-up action:

For each audit, extract still-open action items into issues or current debt
ledgers. Keep audits that explain production incidents or architectural
decisions. Delete or archive audits whose findings are fully resolved and
captured elsewhere.

## FiniteBrain Feature-Dev Specs And Ticket Shards

Candidates:

- `finite-brain/docs/feature-dev/2026-06-27-smoke-alpha-hardening-issue-backup-cutover.md`
- `finite-brain/docs/feature-dev/2026-06-27-smoke-alpha-hardening-issue-browser-grants.md`
- `finite-brain/docs/feature-dev/2026-06-27-smoke-alpha-hardening-issue-daemon-watch.md`
- `finite-brain/docs/feature-dev/2026-06-27-smoke-alpha-hardening-issue-org-invites.md`
- `finite-brain/docs/feature-dev/2026-06-27-smoke-alpha-hardening-prd.md`
- `finite-brain/docs/feature-dev/2026-07-11-dashboard-theme-ticket-01-foundation.md`
- `finite-brain/docs/feature-dev/2026-07-11-dashboard-theme-ticket-02-knowledge-workspace.md`
- `finite-brain/docs/feature-dev/2026-07-11-dashboard-theme-ticket-03-access-workflows.md`
- `finite-brain/docs/feature-dev/2026-07-11-dashboard-theme-ticket-04-responsive-verification.md`
- `finite-brain/docs/feature-dev/2026-07-11-finitebrain-dashboard-theme-spec.md`
- `finite-brain/docs/feature-dev/2026-07-11-finitebrain-settings-brain-ui-spec.md`
- `finite-brain/docs/feature-dev/2026-07-12-product-client-audit-remediation-spec.md`
- `finite-brain/docs/feature-dev/2026-07-21-hybrid-wiki-search-beta-baseline.md`
- `finite-brain/docs/feature-dev/2026-07-21-hybrid-wiki-search-spec.md`
- `finite-brain/docs/feature-dev/2026-07-23-organization-brain-collaboration-spec.md`

Why reduce:

These were kept because they look like specs, PRDs, tickets, or measurement
baselines rather than disposable run artifacts. They still probably should not
live indefinitely in `feature-dev/` once implemented or superseded.

Follow-up action:

Move still-authoritative specs into `finite-brain/docs/specs/` or ADRs. Convert
remaining active tickets to GitHub issues. Delete ticket shards and PRD copies
only after implementation status and current product behavior are verified.

Link/staleness signals observed:

- Some Brain docs still point at missing concept docs such as
  `finite-brain/docs/concepts/title.md`.

## Finite Search Investigation And Runbook Sprawl

Candidates:

- `finite-search/docs/benchmark-comparison-2026-07-01.md`
- `finite-search/docs/production-readiness-investigation-2026-07-01.md`
- `finite-search/docs/tinfoil-evaluation-2026-07-01.md`
- `finite-search/docs/prd/0001-self-hosted-web-search-extract.md`
- `finite-search/docs/runbooks/hermes-integration.md`
- `finite-search/docs/runbooks/latitude-docker-spike.md`
- `finite-search/docs/runbooks/search-fallback-policy.md`
- `finite-search/docs/runbooks/tinfoil-follow-up.md`
- `finite-search/docs/runbooks/tinfoil-searxng-prototype.md`

Why reduce:

These docs preserve useful bootstrap and evaluation evidence, but several are
dated investigations around the same July 2026 search/extract rollout. They
should collapse into a smaller current contract for search fallback,
operations, and production readiness.

Follow-up action:

Keep the authoritative fallback and operations policy. Archive or delete dated
benchmark/evaluation notes once their still-relevant findings are moved into
ADRs, runbooks, or issues.

## Finite Chat Planning, Audit, And Debt Docs

Candidates:

- `finitechat/docs/agent-invite-plan.md`
- `finitechat/docs/chat-ui-port-handoff-2026-06-17.md`
- `finitechat/docs/darkmatter-port-log.md`
- `finitechat/docs/feature-audit-marmot-pika.md`
- `finitechat/docs/friends-alpha-hardening-plan.md`
- `finitechat/docs/friends-alpha-integration-runbook.md`
- `finitechat/docs/friends-alpha-self-build.md`
- `finitechat/docs/hermes-phone-canary-loop.md`
- `finitechat/docs/implementation-plan.md`
- `finitechat/docs/marmot-investigation.md`
- `finitechat/docs/oops-i-faked-it-audit.md`
- `finitechat/docs/perf-audit.md`
- `finitechat/docs/perf-log.md`
- `finitechat/docs/perf-plan.md`
- `finitechat/docs/real-state-offline-plan.md`
- `finitechat/docs/rmp-app-runtime-hard-cut-plan.md`
- `finitechat/docs/room-topics-electron-daemon-plan.md`
- `finitechat/docs/technical-debt-ledger.md`

Why reduce:

Finite Chat has many useful but overlapping pre-release plans, audits,
handoffs, and debt ledgers. The risky part is not age by itself; the risky part
is letting old planning docs compete with current ADRs, protocol docs,
runbooks, and product behavior.

Follow-up action:

Preserve facts that still explain compatibility, protocol, or rollout
decisions. Move live debt rows to GitHub issues or a smaller current debt
ledger. Delete old port logs, implementation plans, or investigation docs only
after the current architecture/protocol docs cover the surviving facts.

Link/staleness signals observed:

- `finitechat/crates/finitechat-rmp/README.md` points at missing RMP docs,
  which suggests the RMP planning/docs set needs a focused pass.

## Infra Host Captures And Deployment Runbooks

Candidates:

- `infra/hosts/**/*.md`
- `infra/runbooks/decommission-lat2.md`
- `infra/runbooks/lat1-catastrophic-recovery-copy.md`
- `infra/runbooks/lat1-nixos-reinstall.md`
- `infra/runbooks/phala-confidential-runner.md`
- `infra/runbooks/finite-private-deepseek-production-update.md`
- `infra/runbooks/finite-private-limiter-mono-switch.md`
- `infra/runbooks/finite-private-routing-migration.md`
- `infra/tinfoil/**/*.md`

Why reduce:

These may be production-critical, so they are not deletion candidates without
operator review. They are Tier 1 because host names, live topology, and rollout
state change quickly, and stale host captures are dangerous if treated as
current procedure.

Follow-up action:

Run `scripts/finite-status` before deciding anything. Keep current recovery,
backup, deployment, and break-glass runbooks. Mark decommissioned-host docs as
historical or delete them after the authoritative current topology and recovery
procedure are verified.

## Finite Skills Source And Reference Docs

Candidates:

- `finite-skills/skills/**/SKILL.md`
- `finite-skills/skills/**/references/*.md`
- `finite-skills/skills/**/templates/**/*.md`
- `finite-skills/tests/fixtures/**/*.md`

Why reduce:

This is a large amount of Markdown, but much of it is product source for the
managed skills system or test fixture data. It should not be cleaned up with a
generic docs pass. The likely reduction work is deduplication, stale reference
repair, and fixture pruning after skill tests prove what is still used.

Follow-up action:

Run the skills static checks and inspect skill packaging before deleting
anything. Reduce duplicated reference sets, fix stale cross-skill links, and
delete unused fixtures only with test coverage.

Link/staleness signals observed:

- `finite-skills/skills/finance/trading-agent-finite/SKILL.md` has stale
  relative links to research skills.
- Several `llm-wiki` reference docs contain illustrative Markdown paths that a
  naive link checker reports as missing; review these with skill-specific
  intent instead of doing blind link rewrites.
