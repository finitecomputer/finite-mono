# Parking lot

Status: PROPOSED

One line per idea outside the active run. This is not a queue; an item becomes work only in a later proposed and blessed run doc.

- 2026-07-10 — After the internal production canary passes, propose the paid invited-customer-cohort launch run; retain `infra/README.md`'s off-host, service-consistent backup and empty-target restore gate, and require real Stripe Checkout plus webhook-to-Core entitlement without prematurely choosing final key custody.
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
