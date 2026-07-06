# Friends Alpha Self-Build

Use this when a friend is building Finite Chat on their own Mac and installing
it on their own iPhone for Friends Alpha testing.

The normal Friends Alpha app should use `https://chat.finite.computer` without
any server field or launch override. Local server URLs are for branch
validation only.

## Prerequisites

- Xcode installed, with command-line tools selected.
- A paired physical iPhone with Developer Mode enabled.
- An Apple Developer account/team that can sign an iOS development build.
- Homebrew available if `protoc` or `xcodegen` are not installed.
- GitHub access to `finitecomputer/finitechat`.

Check the basics:

```sh
xcodebuild -version
xcode-select -p
xcrun devicectl list devices
```

The phone should show as `connected`. If it is unavailable, unlock the phone,
trust the Mac, enable Developer Mode, and reconnect the cable.

## Clone And Prepare

```sh
git clone git@github.com:finitecomputer/finitechat.git
cd finitechat
git checkout codex/friends-alpha-hardening
```

Generate the Rust iOS bindings, XCFramework, and Xcode project:

```sh
ios/ci_scripts/ci_post_clone.sh
```

Then run a quick local health check:

```sh
cargo run -q -p finitechat-rmp -- doctor
cargo run -q -p finitechat-cli -- http --server https://chat.finite.computer health
```

The deployed server response must include `status: "ok"`, `server_version`,
`source_commit`, and `source_dirty: false`. For this Friends Alpha branch, the
server behavior gate is recorded in `docs/friends-alpha-integration-runbook.md`.

## Build From Xcode

Open the generated project:

```sh
open ios/FiniteChat.xcodeproj
```

In Xcode:

1. Select the `FiniteChat` scheme.
2. Select the physical iPhone as the run destination.
3. In `FiniteChat` target signing settings, choose the local Apple team.
4. Let Xcode create or download the development provisioning profile.
5. Build and run.

Normal Xcode launches do not persist server or device launch overrides. If the
phone has old pre-alpha app data, a clean install can still be useful to reset
chat state, but it should not be necessary just to get the deployed server
default.

After launch, check the app's Settings/diagnostics and confirm the server is:

```text
https://chat.finite.computer
```

If an old pre-alpha install still shows a local or staging server, open
Settings, expand Developer diagnostics, and tap **Use Deployed Server**.

Do not add `FINITECHAT_SERVER_URL`, `--finitechat-server`, or an Xcode launch
argument for normal friend testing. The only code path that may persist a
launch-time server override is an explicit development or harness run with
`--finitechat-persist-launch-config`.

## Command-Line Build Option

This is useful when Xcode signing is already configured and the friend wants a
repeatable install command.

Find the two device identifiers:

```sh
xcrun xctrace list devices
xcrun devicectl list devices
```

Use the hardware UDID from `xctrace` for `xcodebuild` and the CoreDevice
identifier from `devicectl` for install/launch.

Paul's local team id for the canonical `computer.finite.finitechat` debug build
is `JBLHZ83X6T`. Friends building from their own Apple account should use their
own team id for `DEVELOPMENT_TEAM`.

```sh
export IOS_HARDWARE_UDID="<udid-from-xctrace>"
export COREDEVICE_ID="<identifier-from-devicectl>"
export DEVELOPMENT_TEAM="<apple-team-id>"

xcodebuild \
  -project ios/FiniteChat.xcodeproj \
  -scheme FiniteChat \
  -configuration Debug \
  -destination "platform=iOS,id=$IOS_HARDWARE_UDID" \
  -derivedDataPath .state/xcode-device-derived-data \
  -allowProvisioningUpdates \
  DEVELOPMENT_TEAM="$DEVELOPMENT_TEAM" \
  build

xcrun devicectl device uninstall app \
  --device "$COREDEVICE_ID" \
  computer.finite.finitechat || true

xcrun devicectl device install app \
  --device "$COREDEVICE_ID" \
  .state/xcode-device-derived-data/Build/Products/Debug-iphoneos/FiniteChat.app

xcrun devicectl device process launch \
  --device "$COREDEVICE_ID" \
  computer.finite.finitechat
```

If command-line launch reports the device is locked, unlock the phone and retry
or tap the app icon manually.

## Signing Notes

`ios/project.yml` has the canonical product bundle id
`computer.finite.finitechat` and the project default signing team used by Paul.
A friend's local generated `.xcodeproj` may use their own team for a debug
build.

Basic chat testing needs a build that installs and talks to the deployed server.
Push notification proof is stricter: the provisioning profile must include
`aps-environment`, and the push drain must use APNs credentials and topic that
match the installed app's signing team and bundle id. If the friend uses a
different team or bundle id for local testing, do not count that as APNs proof.

## First Manual Proof

Once the app launches:

1. Create or enter the user's Nostr key.
2. Confirm the server diagnostic is `https://chat.finite.computer`.
3. Join or create a room with Paul.
4. Send one text message in each direction.
5. Leave the app installed and signed in for the group/agent tests in
   `docs/friends-alpha-integration-runbook.md`.
