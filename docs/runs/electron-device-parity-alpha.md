# Electron Device Parity Alpha

Status: ACTIVE

Owner: Paul

Opened: 2026-07-10

Expires: 2026-07-24

Acceptance: Paul signs in to the Hosted Web Device and approves one Electron
Device for the same Finite Chat account. The Hosted Web Device and Electron
Device then both participate in one existing Room, Topic, and Chat with the
same agent: each device can send, both devices observe the ordered transcript
and agent replies, and restarting either device returns to that same Chat
without duplicated messages or a replacement identity.

## Authority and boundaries

This is an internal alpha of the intended customer architecture. Electron is a
distinct revocable Device with local custody and a real local daemon; it does
not reuse the Hosted Web Device or create a desktop-only account, Room, Chat,
or agent path. WorkOS authorizes the signed-in human's device-link request but
does not carry or become a Finite Chat account signer.

The paid invited-customer cohort and Stripe admission remain the next run.
This run does not admit customers, change billing, deploy production services,
or change agent/runtime authority.

## Queue

Work top-down. Every retained item is required.

### P0 — Local daemon is a product boundary

- Authenticate every Electron-to-daemon request with a per-launch secret that
  never enters renderer state, URLs, argv, logs, or the daemon store.
- Remove permissive browser CORS and the renderer's raw daemon URL. Keep the
  preload surface narrow and typed.
- Bind an ephemeral loopback port, require a successful readiness handshake,
  and make window close/reopen plus daemon restart deterministic.
- Persist a random Device id under Electron app support rather than deriving it
  from the host name.
- Package the daemon as an Electron resource. Product startup must never fall
  back to host `cargo run`; explicit developer/test binary overrides may remain.
- Add negative and positive boundary tests plus restart/store regression tests.

### P0 — Enroll Electron as the signed-in account's distinct Device

- Add a short-lived, single-use WorkOS-authorized link ceremony using the
  existing opaque link-session rendezvous. The rendezvous sees only public or
  encrypted material and cannot recover the account secret.
- Keep account-key material out of the browser renderer. Deliver it only to the
  Electron main process/local daemon and store it through the platform secret
  store.
- Publish the Electron Device's KeyPackages and drive the existing durable
  account-room fanout path so it joins every current account Room, including
  the canonical agent Room. Do not grant the Device room authority.
- Make retry, expiry, claim, acknowledgement, and replay behavior explicit and
  regression tested.

### P1 — Present the same Finite Chat product

- Render the canonical Room, Topic, Chat, transcript, composer, attachments,
  activity, and working/final-delivery semantics from `finitechat-core`
  projections and typed actions.
- Reuse shared presentation and interaction code where it is genuinely the
  same product behavior. Do not maintain a second Electron-only transcript or
  conversation state machine.
- Make linking, linked-device state, revocation/error state, and recovery copy
  honest for an internal alpha.

### P1 — Prove multi-device parity

- Add an automated product acceptance test with one Hosted Web Device and one
  Electron daemon Device on the same account, using distinct device ids in the
  same agent Room, Topic, and Chat.
- The test sends from both devices, observes the same ordered transcript and
  agent replies on both, restarts Electron and the Hosted Web Device
  independently, and proves continuation without duplicate delivery.
- Run the relevant Rust, dashboard, Electron, and root integration gates.
- Record any step that requires external signing/notarization or a production
  deployment as a true handoff; automated/local evidence is not Paul's final
  acceptance.

### P1 — Paul acceptance

- Paul completes the acceptance statement at the top of this run against the
  alpha build. Do not claim acceptance from automated tests alone.

## Out of scope

- Paid/customer admission, Stripe, entitlement or Finite Private limit changes.
- Brain, Sites viewer authorization, provider/runtime management, Runner
  features, remote `.env`, or a filesystem control plane.
- New account, Room, Topic, Chat, transport, or runtime-authority models made
  only for Electron.
- Public release signing/notarization beyond what blocks installing the
  internal alpha.

## Governing documents

- [`docs/monorepo-doctrine.md`](../monorepo-doctrine.md)
- [`docs/adr/0001-recoverability-precedes-operator-blindness.md`](../adr/0001-recoverability-precedes-operator-blindness.md)
- [`finitechat/CONTEXT.md`](../../finitechat/CONTEXT.md)
- [`finitechat/docs/adr/0011-hosted-web-chat-uses-a-revocable-device.md`](../../finitechat/docs/adr/0011-hosted-web-chat-uses-a-revocable-device.md)
- [`finitechat/docs/room-topics-electron-daemon-plan.md`](../../finitechat/docs/room-topics-electron-daemon-plan.md)
- [`finitecomputer-v2/docs/identity-boundary-v1.md`](../../finitecomputer-v2/docs/identity-boundary-v1.md)

