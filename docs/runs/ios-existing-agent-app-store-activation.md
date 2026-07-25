# iOS Existing-Agent Companion App Store Activation

Status: ACTIVE (2026-07-24 — Paul explicitly selected the existing-agent
companion path and prioritized CI-driven internal TestFlight distribution)

Sequence note: On 2026-07-24, Paul moved the active sequence here from
[`Phala Confidential Runner Readiness`](phala-confidential-runner-readiness.md).
That run is PAUSED with its queue preserved and no current provider-spend or
production-mutation authority.

Owner: Paul

Opened: 2026-07-24

Expires: 2026-08-21

Acceptance: A clean physical iPhone installs a CI-built Finite Chat release
from the team's internal TestFlight group, signs in through the production
WorkOS application using Sign in with Apple, automatically links a fresh
cryptographic Device to one already-provisioned Finite agent, and completes the
retained chat, attachment, voice, notification, background, relaunch, sign-out,
and relink matrix without local overrides or operator repair. App Store Connect
contains accurate privacy, encryption, age-rating, review-account, and review
notes. A stable review account exposes seeded chats and a live agent throughout
review. The run stops at submission-ready plus Paul's physical-device
acceptance; it does not authorize public App Store release.

## Problem statement

The focused native iOS app is locally real: the Rust/UniFFI runtime builds,
Simulator tests pass, and an iOS Device can talk to a local Hermes agent. It is
not yet a shipped product because the team cannot install a repeatable
production-shaped build from TestFlight, and the remaining Apple/WorkOS,
privacy, review-account, deletion, push, and physical-device facts are not one
accepted release contract.

The repository still contains the original Xcode Cloud bootstrap script,
automatic signing configuration, production WorkOS client id, App Store
entitlements, icon assets, and TestFlight runbook. Xcode Cloud workflows and
build history live in App Store Connect and must be inspected there; their
existence or correctness cannot be inferred from checked-in files.

The product boundary is one existing human talking to one existing Finite
agent. This run does not add in-app signup, StoreKit, agent purchasing,
self-hosted Hermes pairing, People, invites, or legacy migration.

## Authority and constraints

- ACTIVE status currently authorizes local repository work, validation, and
  milestone commits in the isolated iOS worktree. Branch publication, pull
  requests, deployments, Xcode Cloud builds/uploads, TestFlight distribution,
  and merges require Paul's explicit go-ahead.
- Public App Store release, destructive account or Agent deletion, production
  user-state repair, new paid infrastructure, StoreKit products, and customer
  admission require separate explicit authority.
- This public repository contains no WorkOS secret, App Store Connect API key,
  APNs private key, review-account password, recovery key, or account secret.
  Record only secret names and their external custody location.
- The iOS binary contains only the public WorkOS application client id. OAuth
  token exchange uses PKCE; server credentials remain server-side.
- The app is a free native companion to an existing Finite agent. It contains
  no purchase UI, external purchase link, price, signup promise, or dead-end
  empty account experience.
- A successful archive, upload, login, or notification is evidence only for
  that gate. It does not prove the complete physical-device matrix.
- Review uses a dedicated recoverable account and Agent, never Paul's personal
  account or an arbitrary existing production row.
- Account deletion must preserve the repository's recoverability invariant.
  Defining or initiating deletion is not authority to destroy an Agent Runtime
  or its Recovery Set.

## Current inventory

Already present in the repository:

- bundle id `computer.finite.finitechat`;
- Apple team `JBLHZ83X6T` with automatic signing;
- generated Xcode project source at `finitechat/ios/project.yml`;
- executable Xcode Cloud bootstrap at
  `finitechat/ios/ci_scripts/ci_post_clone.sh`;
- pinned Rust `1.91.1`, iOS device/simulator Rust targets, UniFFI binding and
  XCFramework generation, and XcodeGen regeneration;
- Release WorkOS application client id
  `client_01KYA32JRWEE23J7QW1F882DVA`;
- Sign in with Apple, Associated Domains, APNs, and remote-notification
  declarations;
- App Store icon assets and native privacy permission strings;
- `finitechat/docs/testflight-runbook.md`;
- `finitechat/docs/push-notifications-apple-runbook.md`; and
- a draft product privacy policy at `finitechat/privacy.txt`.

Externally reported by Paul:

- an App Store Connect app exists for `computer.finite.finitechat`;
- an Xcode Cloud setup may exist from the earlier broad iOS app;
- production and staging WorkOS iOS applications exist;
- `https://finite.computer/auth/ios/callback` is registered in both WorkOS
  environments; and
- Apple Sign in credentials have been installed in WorkOS production, while
  staging uses WorkOS demo credentials.

These external facts remain unaccepted until read-only inspection or a real
build proves them.

Read-only App Store Connect inspection on 2026-07-24 established:

- the `computer.finite.finitechat` app record exists as version `1.0` in
  Prepare for Submission;
- the existing `Default` Xcode Cloud workflow is still attached to the archived
  `finitecomputer/finitechat` repository, project `ios/FiniteChat.xcodeproj`,
  and `main`;
- Build 42 compiled, tested, archived, and exported successfully from that old
  repository, then failed only while preparing version `0.1.0` for App Store
  Connect;
- the workflow prepares for App Store Connect rather than internal-only
  TestFlight and has no TestFlight post-action; and
- TestFlight contains no builds, internal groups, or testers yet.

Local release-path evidence on 2026-07-24 established:

- the RMP device path regenerated a device-capable Rust XCFramework, compiled
  the Swift app with the Release configuration, installed it on Paulphone Air
  running iOS 26.5.2 without erasing its app data, and launched it;
- the full `finitechat-rmp` unit suite passed (69 tests);
- the cold Xcode Cloud preflight regenerated the bridge and Xcode project,
  verified bundle id `computer.finite.finitechat`, marketing version `1.0`,
  production WorkOS client configuration, and completed an unsigned Release
  Simulator build; and
- Paul's production login, existing-agent selection, and chat observations on
  that device remain pending.

## Queue

Work top-down. Discovered enhancements that are not required for acceptance go
to `parking-lot.md`.

### P0 — Restore CI-to-internal-TestFlight delivery

- Inspect the existing App Store Connect app, Xcode Cloud workflow, source
  repository connection, product path, scheme, last build, signing status, and
  internal tester group. Record no credentials or personal tester details.
- Make the checked-in Xcode Cloud bootstrap deterministic from the monorepo
  root and add a local preflight that proves its path/toolchain assumptions
  without uploading or signing.
- Ensure Release archives use a monotonically unique build number and an
  explicit marketing version without hand-editing generated project files.
- Run the normal Rust core regression, iOS Simulator test suite, Release
  compile/archive preflight, and real Simulator launch on the rebased source.
- Publish the active branch to GitHub only after the local gates pass.
- Create or repair one Xcode Cloud workflow for
  `finitechat/ios/FiniteChat.xcodeproj`, scheme `FiniteChat`, with an Archive
  action prepared for **TestFlight (Internal Testing Only)** and an internal
  TestFlight post-action/group.
- Start one build, retain the Xcode Cloud build number and source revision, and
  prove that a team member can install it from TestFlight.

### P0 — Prove production WorkOS and Sign in with Apple

- Inspect the production WorkOS iOS application without changing its client id,
  redirect URI, or Apple credentials unless a concrete mismatch is found.
- Verify the Release binary resolves the production client id and the
  production callback origin; no staging or localhost value may enter the
  archive.
- From a clean physical install, complete native PKCE authentication with Sign
  in with Apple, return through the universal-link callback, and retain the
  authenticated session across background, force-quit, and relaunch.
- Sign out, confirm local credentials and cryptographic Device state are
  removed, sign in again, and prove the replacement uses a fresh Device id and
  links without false-ready state.

### P0 — Establish the App Review account and Agent

- Select or create one dedicated review WorkOS account whose credentials and
  any required verification procedure can be supplied safely in App Store
  Connect. Do not use a personal account.
- Provision exactly one stable review Agent through the normal product path;
  do not repair or synthesize production membership by database edit.
- Seed three harmless chats that demonstrate topics, recent chats,
  attachments, voice, and a normal agent response without exposing company or
  customer data.
- Add an uptime owner and a review-window check so the account, Agent, chat
  server, dashboard callback, and required model provider remain available.
- Re-run the full login/link/chat path using only the review credentials.

### P1 — Close privacy and deletion requirements

- Audit `finitechat/privacy.txt` against actual WorkOS, Apple, APNs, speech,
  attachment, encrypted metadata, diagnostics, retention, and agent/model
  provider behavior; publish the accepted policy at a stable HTTPS URL.
- Produce the exact App Privacy answers from code and deployed behavior,
  distinguishing data linked to identity, unlinked diagnostics, and data not
  collected. Paul enters and confirms them in App Store Connect.
- Decide and document the existing-agent account-deletion story: where a user
  initiates it, what local state is erased immediately, what server/account
  data is scheduled for deletion, what recovery material is retained and why,
  and how an active Agent is retired without coupling compute teardown to data
  loss.
- Add only the smallest in-app affordance App Review requires. Do not implement
  destructive deletion until its server contract, recovery boundary, and
  rollback/support process are accepted.

### P1 — Execute the physical TestFlight matrix

- On a clean physical iPhone install: login, automatic Device Link, paired-agent
  selection, home load, existing-chat navigation, and new chat.
- Prove text, photo/file attachment, voice recording/transcription, agent
  response, delivery/read state, and encrypted history after relaunch.
- Prove foreground/background transitions, force-quit/relaunch, temporary
  network loss/recovery, and no UI freeze while Rust work is active.
- Prove APNs token registration and one wake-only production push while the app
  is backgrounded; record no plaintext or token.
- Prove sign-out and relink with a fresh Device id, then repeat one chat turn.
- Record the TestFlight build, source revision, iOS/device version, server
  revision, and pass/fail evidence in this run.

### P1 — Make the App Store submission reviewable

- Complete export-compliance, age-rating, content-rights, support URL, privacy
  URL, category, copyright, and required contact metadata.
- Capture App Store screenshots from the accepted focused experience, never a
  login-only screen or personal account.
- Write review notes explaining the existing-agent companion model, demo
  credentials, native WorkOS/Sign in with Apple flow, automatic Device Link,
  end-to-end encryption, copied user key custody, microphone/speech/photo
  permissions, wake-only APNs, and exact steps to exercise the app.
- Validate the candidate archive and ensure its TestFlight build is eligible
  for submission, but stop before public App Store submission/release.

### P1 — Acceptance Request

- **Revision:** exact Git revision, Xcode Cloud workflow/build number, TestFlight
  version/build, and deployed server revision.
- **Where:** the internal TestFlight group and dedicated App Review account;
  name credentials only by external custody location.
- **Time:** 30–45 minutes on one clean physical iPhone.
- **Steps and observations:** the retained physical matrix above, one expected
  observation per action.
- **Pass:** all acceptance language at the top of this run is directly
  observed and the App Store metadata/review notes are complete.
- **Fail/stop:** any wrong auth environment, unavailable review Agent,
  plaintext push/logging, false-ready Device Link, state loss after relaunch,
  archive/signing ambiguity, or destructive deletion requirement stops the
  run with read-only evidence. Do not repair production state speculatively.
