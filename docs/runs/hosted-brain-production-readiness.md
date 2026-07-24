# Hosted Brain Production Readiness

Status: ACTIVE — READY FOR PAUL (2026-07-24)
Owner: Paul
Opened: 2026-07-15
Rescoped: 2026-07-24 after Paul authorized the Brain route, hard-cut identity
model, production rollout, and one Kata canary runtime upgrade.
Acceptance: a signed-in user can open the production Brain UI from an Agent,
confirm that Agent by its canonical managed email, write anywhere in the empty
Personal Brain as its one Personal Agent, and read the result as the Personal
Brain owner after a service restart.
Expiry: 2026-07-31; stop and rescope if the identity or durable-data boundary
changes before acceptance.

Brain is live in dashboard navigation. The WorkOS-bound signer connects and
the server-owned visible-Brain list correctly returns no Brains for the
acceptance account. No Personal Brain was created implicitly during rollout;
the remaining state mutation is the explicit user-confirmed creation and
end-to-end acceptance below.

## Problem statement

The implementation and rollout gates are closed. Production now has the
current Product Client UI, Finite Identity, bounded Brain/Core authority,
service-consistent Brain/Identity backups, a current `fbrain` runtime, and a
digest-pinned dashboard with Brain navigation enabled.

The remaining gate is deliberately small and user-visible: create the
acceptance account's first Personal Brain through **Manage Brains**, confirm
the selected Agent's canonical managed email, prove owner and Personal Agent
read/write behavior, restart the named services, and prove the same state
again. Until Paul performs that flow, healthy endpoints and an empty signed
Brain list do not claim product acceptance.

## Constraints

- Reuse the merged Finite Identity and Brain contracts; do not introduce a
  second identity store, pairing protocol, or product-specific email resolver.
- The canonical Identity Authority signing/API origin is
  `https://identity.finite.vip`; trusted same-host products use its loopback
  listener. Production examples must not reintroduce an older placeholder.
- Keep Identity Authority operator credentials on trusted services only. They
  never enter an Agent Runtime.
- Treat the Identity Authority SQLite directory and Brain SQLite database as
  durable production state. Name backups and rollback boundaries before any
  production mutation.
- Personal Brain creation must remain an explicit action. Opening Brain,
  connecting the signer, selecting an Agent, or receiving an empty Brain list
  must never create durable state.

## Production checkpoint

- [x] Product Client UI and isolated dashboard frame deployed.
- [x] `fbrain` v0.1.3 remains the released CLI contract; runtime image
  `2026-07-24.1` contains the current reviewed command surface.
- [x] Finite Identity, Brain, Core, Hosted Device, dashboard, and
  service-consistent backup boundary deployed together.
- [x] Brain navigation enabled in dashboard image
  `2026-07-24.3@sha256:85458035b447a5eef6960e90842ecf939f0ab685678bf3ff33cad9fcc308f8c1`.
- [x] Upgrade Canary 0715 explicitly moved to runtime artifact
  `finite-agent-runtime-2026-07-24.1` while preserving its Agent Principal and
  writable durable `/data` mount.
- [x] Electron signer session and empty server-owned Brain list verified
  without creating a synthetic or implicit Personal Brain.
- [x] Compatibility and run docs reconciled with the production checkpoint;
  superseded Electron run docs deleted and Phala authority paused.
- [ ] Paul completes the explicit create, owner/Agent read-write, and
  post-restart acceptance flow.

## Evaluation and rollback

- Local gates: Identity Authority tests, Brain CLI/server tests, dashboard
  tests/browser/build, and `just dev smoke`.
- Production evidence: exact Git revisions and image digests, healthy public
  authority and Brain endpoints, one immutable Agent Email binding, Personal
  Brain-wide Agent write/read access, owner readback, and restart persistence.
- Before the acceptance service restart, take consistent backups of Identity
  Authority and Brain state and record their hashes outside database contents.
  A NixOS rollback is not a data rollback; preserve both sides if either
  service accepts writes.
- Fail closed on an unavailable authority, mismatched email/principal binding,
  ambiguous Brain ownership, authority beyond the user's Personal Brain, or
  UI/release revision mismatch. Do not repair identity or Brain state in place;
  preserve evidence and return to the last known-good system generation if the
  deployed software regresses.

## Acceptance Request

- **Revision:** mono
  `3a2a9b46edb52441f884f60351b0bae8ad6abc32`; Electron
  `finitechat/v0.1.8`; `fbrain` v0.1.3; runtime
  `finite-agent-runtime-2026-07-24.1@sha256:87ca23a5ee004a6691bd8df950109e220e71e4e24fadb3b75e716b25ba68c0b5`;
  dashboard
  `2026-07-24.3@sha256:85458035b447a5eef6960e90842ecf939f0ab685678bf3ff33cad9fcc308f8c1`;
  NixOS
  `/nix/store/rbfr6db93x2a0cidkwjzsg0f2s3k1bb4-nixos-system-finite-lat-1-25.11.20260630.b6018f8`.
- **Where:** the signed Electron v0.1.8 app loading
  `https://finite.computer`, signed in as `paul@finite.vip`, with Upgrade
  Canary 0715 selected.
- **Time:** 10 minutes.
- **Steps and observations:**
  1. Open Brain from the Agent sidebar. The Product Client renders at normal
     scale with a connected signer and either the existing signed Brain list or
     `No Brains available`; opening alone creates nothing.
  2. Open **Manage Brains**. With no Personal Brain, the creation card is
     visible and its Personal Agent field names Upgrade Canary 0715's canonical
     managed `@finite.vip` email.
  3. Click **Create Personal Brain** and accept the parent dashboard's
     confirmation only if it repeats that exact email. One empty Personal Brain
     appears and opens; no second Brain or owner identity is invented.
  4. Create a Folder and Page as the owner, then ask Upgrade Canary 0715 in
     ordinary chat to use the Personal Brain and add a second Page. Refresh the
     Product Client and read both Pages as the owner.
  5. Ask Codex to take the named Brain/Identity backup and restart
     `finite-brain-app` and `finite-identity`. After health returns, unlock the
     same Brain and repeat the owner readback; ask the Agent to read both Pages
     without a new pairing ceremony.
- **Pass:** the same Agent Principal retains full operational access throughout
  the Personal Brain, and the owner retains ownership and post-restart data.
- **Fail/stop:** capture read-only service health, deployed revisions, binding
  inspection, and Brain authorization output; do not retry creation with a
  different identity and do not rewrite identity or Brain state.
