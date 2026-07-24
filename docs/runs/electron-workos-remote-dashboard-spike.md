# WorkOS Electron Remote Dashboard Spike

Status: IMPLEMENTED LOCALLY — NOT DEPLOYED OR ACCEPTED (2026-07-21)

This spike makes the deployed Finite dashboard the only Electron product UI.
Electron and an ordinary browser load the same JavaScript from the same Finite
origin and use the same WorkOS-backed dashboard session. Electron differs only
where a versioned native capability is present: chat state, chat mutations,
attachments, and chat updates use the local `finitechatd` Device.

This is a new, explicitly requested spike. It does not reactivate or rewrite
the historical parked run in `electron-device-parity-alpha.md`, deploy a build,
or claim product acceptance.

## Acceptance for the spike

- Electron opens the configured `/dashboard` surface and completes WorkOS
  sign-in inside a persistent Electron session.
- WorkOS and identity-provider documents never receive the privileged preload.
- The privileged preload is available only to the exact configured origin's
  `/dashboard` subtree.
- On first authenticated use, Electron creates a distinct Nostr Device and the
  signed-in Hosted Web Device authorizes it without a QR, PIN, approval button,
  or account secret entering the renderer.
- When the same user creates an Agent later, selecting it silently reconciles
  the already-linked Electron Device into its canonical Room without a new
  pairing ceremony.
- The local Device secret is stored with Electron `safeStorage`; the daemon URL,
  bearer, and secret never enter renderer state.
- The canonical dashboard chat uses local daemon state, actions, updates, and
  attachments. The browser dashboard continues using the existing hosted path.
- WorkOS/web control-plane behavior remains unchanged, including machine
  binding recovery, owner claim, Connections, account management, and sign-out.
- Connections passively lists account-wide chat Devices. It offers no pairing,
  refresh, or revocation controls in this spike.
- A WorkOS/local-Nostr account mismatch fails closed and never clears, replaces,
  or silently relinks the local Device.

## Capability boundary

The dashboard detects one exact bridge contract (`version: 1`) with the ordered
capabilities `local-chat-v1` and `automatic-device-link-v1`. Absence of that
exact contract is the feature flag: the existing browser path is unchanged.

The privileged dashboard may request only:

- automatic local-Device readiness;
- daemon state;
- the canonical dashboard chat action allowlist;
- bounded binary attachment upload and local attachment URLs;
- daemon generation/state/error updates; and
- public Device-link progress.

The remote renderer does **not** receive:

- the account secret, daemon URL, daemon bearer, raw IPC, or a generic signer;
- secret deletion/import/export or automatic account replacement;
- manual pairing URLs, approval controls, or Device-link rendezvous values;
- arbitrary daemon actions, including Device revocation or Room joins;
- shell, clipboard, filesystem, or onboarding IPC; or
- the old Electron-only `finite://join` / `ScanTarget` presentation path.

Sign-out ends the WorkOS browser session only. It does not revoke or delete the
local Nostr Device. Signing back in as its account reuses it; signing in as a
different account produces a fixed mismatch error.

## Trust decision

The configured deployed dashboard JavaScript is a trusted controller for the
allowlisted local chat operations. This is the explicit same-origin tradeoff of
the spike. External auth pages are unprivileged, all Electron permissions are
denied, privileged navigation is confined to `/dashboard`, and IPC validates
the current top-level frame and exact origin.

## Automatic enrollment

1. Electron authenticates WorkOS in `persist:finite-dashboard-v1`.
2. Main reads the authenticated Hosted Web Device's public Nostr account id.
3. If no local secret exists, main starts the existing private-FD Device-link
   supervisor and receives only its public rendezvous tuple.
4. Main, not dashboard JavaScript, submits that tuple to same-origin,
   WorkOS-protected approve/status routes using the persistent session cookies.
5. The account secret returns only through the supervisor's private child FD,
   is stored provisionally, acknowledged, and then promoted with `safeStorage`.
6. Main waits for existing Room fanout to report ready, starts `finitechatd`,
   and compares its account id to the WorkOS-bound account id.
7. The provider also requires the selected machine's authoritative canonical
   Room to exist locally before exposing local chat state.

Only public pending rendezvous identifiers are persisted, so a completed link
can resume readiness polling after a crash. Provisional secrets are discarded
on restart.

## Later-created Rooms

Initial enrollment fans out every Room that exists at that time. If the user
later creates an Agent, its canonical Room will initially be absent from an
already-linked Electron Device. The provider handles that case invisibly:

1. It first verifies that the local Device, Hosted Web Device, and sealed Agent
   binding all name the same Nostr account.
2. It sends only the local public Device id to a WorkOS-protected,
   machine-scoped dashboard route.
3. The dashboard derives the Project id from its authoritative machine context;
   the browser cannot choose it.
4. Hosted Device validates that Project's sealed Agent binding and uses a
   deterministic, retry-stable fanout id with
   `FiniteChatRuntime::link_device_and_wait`.
5. The provider retries bounded public progress and exposes chat only after the
   local daemon actually reports the canonical Room.

This is reconciliation of an already-linked account Device, not a new pairing
session. There is no QR, code, approval, retry, or Room-join control in the UI.
Account mismatch, missing binding, malformed progress, cancellation, and retry
exhaustion all fail closed.

## Remaining spike limits

- Device admission gives Electron forward continuity, plus a bounded bootstrap
  of conversation foundations and at most 64 application events / 176 KiB per
  Room. It is not a promise that Electron receives the complete transcript from
  before its admission. After admission, the same new Chats and Messages are
  delivered to Hosted Web and Electron.
- One provider load performs eight bounded reconciliation attempts. At the
  current 16-Room discovery page size, an account with more than 128 Rooms may
  need a reload to resume the same deterministic fanout.
- The protocol, service, renderer boundary, and provider pieces are covered
  separately, but there is not yet one deployed live test spanning WorkOS,
  Electron, Hosted Device, the chat server, and React.

## Evaluation

Automated gates:

- Electron Node boundary tests and a signed package build. Electron has no
  separate TypeScript/Vite renderer build.
- Dashboard unit suite and lint, including browser fallback, account mismatch,
  canonical-Room enforcement, later-Room reconciliation, strict machine route,
  and structured-clone attachment transport.
- Daemon HTTP tests for authenticated, bounded, idempotent New Chat intent.
- Hosted Device tests for authenticated, sealed-binding-bound, idempotent Room
  reconciliation, plus Rust formatting, targeted clippy, and daemon tests.
- `git diff --check` and the relevant root `just` checks.

Manual acceptance still requires a deployed alpha and two real Devices: sign in
through Electron, observe zero-click enrollment, send in one existing Chat from
Electron and web, observe the same ordered transcript on both, restart each
Device independently, create another Agent after Electron enrollment, open it
without pairing, and verify continuation without duplicate messages or a
replacement identity.
