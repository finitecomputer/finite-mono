# Finite Chat TestFlight Runbook

Finite Chat ships as its own App Store Connect app record.

## Product Identity

- App name: `Finite Chat`
- Bundle ID: `computer.finite.finitechat`
- SKU: `computer.finite.finitechat` or `finitechat-ios`
- Primary language: English
- Xcode project: `finitechat/ios/FiniteChat.xcodeproj`
- Scheme: `FiniteChat`
- Signing team in `finitechat/ios/project.yml`: `JBLHZ83X6T`

This keeps the App Store Connect record aligned with the repo, bundle ID,
keychain/runtime naming, and product metadata.

## App Store Connect Setup

1. In App Store Connect, create a new iOS app record.
2. Select or create the Bundle ID `computer.finite.finitechat`.
3. Use `Finite Chat` as the app name.
4. Add beta test information before external testing:
   - beta app description
   - feedback email
   - contact information
   - "What to Test" notes
5. Complete required compliance metadata:
   - encryption/export compliance: answer that the app uses encryption, then
     follow Apple's questionnaire for standard/open encryption where no
     documentation is required
   - privacy nutrition labels
   - age rating
   - content rights
6. Start with an internal TestFlight group, then add an external group after the
   first uploaded build is viable.

External TestFlight should be treated as a review gate. The first build added to
an external testing group is sent to Beta App Review; later builds for the same
version may not require a full review.

## Xcode Cloud

The GitHub-backed Xcode Cloud lane is currently paused with the other macOS
builds. GitHub remains the source repository. Resume repeatable TestFlight
delivery only after a dedicated macOS execution, signing, notarization, and
qualification plan has been designed and reviewed.

The former workflow configuration is retained below as non-authoritative
reference for that follow-up:

- Files and folders: `finitechat/**`, plus root Cargo/Nix/build inputs used by
  the native bridge
- Project: `finitechat/ios/FiniteChat.xcodeproj`
- Scheme: `FiniteChat`
- Actions:
  - Test on current iOS Simulator
  - Archive for iOS
  - Prepare for `TestFlight (Internal Testing Only)`
  - Distribute to the internal Finite team group

The repo does not track generated iOS build artifacts. Xcode Cloud must run
`finitechat/ios/ci_scripts/ci_post_clone.sh`, which:

1. Installs Rust, `protoc`, and XcodeGen if missing.
2. Adds the iOS Rust targets.
3. Runs `cargo run -q -p finitechat-rmp -- bindings swift --clean`.
4. Regenerates `finitechat/ios/FiniteChat.xcodeproj` from
   `finitechat/ios/project.yml`.

Apple requires Xcode Cloud custom scripts to live in a `ci_scripts` directory at
the same level as the Xcode project or workspace. Because the project path is
`finitechat/ios/FiniteChat.xcodeproj`, the script lives at
`finitechat/ios/ci_scripts/ci_post_clone.sh`.

The checked-in marketing version is `1.0`. Xcode Cloud assigns its own
monotonically increasing integer build number and App Store Connect uses that
number for distributed builds. Do not hand-edit the generated Xcode project.

`finitechat/ios/TestFlight/WhatToTest.en-US.txt` supplies the internal beta
notes that Xcode Cloud publishes with each build.

## Preflight Checks

For direct physical-device debug builds before TestFlight, use
`docs/friends-alpha-self-build.md`.

Before the first uploaded build:

```sh
just ios-cloud-preflight
cargo run -q -p finitechat-rmp -- test ios-simulator
```

`just ios-cloud-preflight` enters the pinned repo environment, checks the Rust
version and required tools, regenerates the bridge and project, verifies the
Release bundle/version/production WorkOS settings, and performs an unsigned
Release Simulator build. It neither signs nor uploads anything.

Use `finitechat-rmp test ios-simulator` for simulator test preflight instead
of an ad hoc `xcodebuild test` command. It erases the dedicated RMP simulator,
uses `.state/xcode-derived-data`, replaces its explicit `.xcresult` bundle, and
shuts the simulator down after the run so repeated preflights do not inherit a
busy SpringBoard or polluted app container.

Also verify:

- The deployed server passes `docs/server-deployment-gate.md`; do not upload or
  distribute a build that depends on server changes until
  `https://chat.finite.computer/health` reports the expected finite-chat
  `source_fingerprint` and `source_dirty: false`.
- `MARKETING_VERSION` in `finitechat/ios/project.yml` matches the App Store
  version. Xcode Cloud owns the distributed build number.
- App icon assets are present in
  `finitechat/ios/Sources/Assets.xcassets/AppIcon.appiconset`.
- Privacy permission copy in `finitechat/ios/Info.plist` matches actual
  behavior.
- The default app server is `https://chat.finite.computer`; local server URLs
  are explicit development/test overrides only.
- `finitechat/ios/Info.plist` does not declare
  `NSLocalNetworkUsageDescription` or `NSAllowsLocalNetworking` for
  TestFlight/App Store builds.
- Review notes explain that Finite Chat uses end-to-end encryption, a linked
  account key stored in Keychain, microphone recording, speech transcription,
  and optional photo-library saves initiated by the user.
