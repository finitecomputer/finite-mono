# Parking lot

Status: PROPOSED

One line per idea outside the active run. This is not a queue; an item becomes work only in a later proposed and blessed run doc.

- 2026-07-10 — After the explicitly prioritized Electron Device Parity Alpha, propose the paid invited-customer-cohort launch run; retained in the 2026-07-13 [`Stripe Production Activation`](stripe-production-activation.md) draft without changing the ACTIVE run. It keeps `infra/README.md`'s off-host, service-consistent backup and empty-target restore gate and requires real Stripe Checkout plus webhook-to-Core entitlement.
- 2026-07-10 — In the customer-facing run, replace the canary landing posture with the explicitly blessed paid/self-serve entry path only after Stripe and customer-admission gates pass.
- 2026-07-10 — In the customer-facing run, define and implement honest end-to-end cancellation for a queued or active Hermes turn without using compute restart as a substitute; see `docs/open-questions.md`.
- 2026-07-10 — In the customer-facing run, add Finite Sites list/share only after deciding whose Projects or Outputs appear and which Principal may mutate sharing; see `docs/open-questions.md`.
- 2026-07-10 — In the customer-facing run, replace the hidden normal-user Skills entry with a read-only canonical catalog and honest `finite skills sync` guidance; require a separate agent-owned contract before showing installed/sync state.
- 2026-07-10 — Before customer admission, add a bounded stuck-launch escape only after defining timeout, cancellation, entitlement release, and provider-cleanup semantics; see `docs/open-questions.md`.
- 2026-07-10 — Re-enable dashboard Brain only after resolving its Nostr Principal and Folder Key path; preserve the existing iframe/proxy work and finish an Electron signer bridge if Electron is chosen as the first usable human Brain client.
- 2026-07-10 — Reduce the manual burden of provider/runtime credential rotation with a deliberate issue/revoke/replace workflow; do not make retrospective rotation of the currently named low-sensitivity keys an internal-canary blocker.
- 2026-07-10 — Finish the paused, two-generation production rollout and evidence for the runtime-upgrade compatibility path after CI is green.
- 2026-07-10 — Quarantine or remove the legacy `finite-core/src/control_plane.rs` anti-pattern crate.
- 2026-07-10 — Make the deployed skills catalog source the monorepo `finite-skills` tree rather than the archived GitHub fallback.
- 2026-07-10 — Add a Kata-path equivalent of SaaS smoke coverage to automated CI.
- 2026-07-10 — Revisit rich remote-Markdown image cards in chat after the launch path is stable.
- 2026-07-10 — Audit the WorkOS AuthKit lifecycle around expired/reused login state without changing identity architecture in a launch-blocker run.
- 2026-07-10 — Before the customer run needs split operator duties, replace the single internal-operator check with an idiomatic WorkOS permission taxonomy for runtime read/operate/upgrade, Finite Private key/limit management, and Launch Code management.
- 2026-07-13 — Paul confirms off-repo whether the 2026-07-09 `FINITE_PRIVATE_API_KEY`, `OPENAI_API_KEY`, and `FAL_KEY` exposures were rotated; record no values here.
- 2026-07-13 — Paul explicitly accepts or replaces the first-admitted-Principal owner-claim posture before the training cohort; do not let an implementation default make this product decision.
- 2026-07-13 — Restrict the finite-lat-1 rsync.net archival credential to destination-enforced append-only access while preserving a separate administrative retention path; Paul accepts the current overprovisioned credential as non-blocking hardening debt.
- 2026-07-13 — Narrow the Nix source inputs so a dashboard-only digest pin does not invalidate and rebuild unrelated Rust service derivations on finite-lat-2.
- 2026-07-14 — Reconcile the dashboard's npm audit baseline (currently 12 reported transitive findings: 2 low, 7 moderate, 3 high) in a dependency-focused run rather than changing packages during cleanup.
