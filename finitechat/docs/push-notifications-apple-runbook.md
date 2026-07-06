# Wake-Only APNs Push Runbook

This runbook is for the Friends Alpha wake-only push path. Push is only a hint:
it never advances client state, never carries plaintext, and only causes the
native app to run the normal sync action.

References:

- Apple: [Registering your app with APNs](https://developer.apple.com/documentation/usernotifications/registering-your-app-with-apns)
- Apple: [APS Environment Entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/aps-environment)
- Apple: [Establishing a token-based connection to APNs](https://developer.apple.com/documentation/usernotifications/establishing-a-token-based-connection-to-apns)
- Apple: [Sending notification requests to APNs](https://developer.apple.com/documentation/usernotifications/sending-notification-requests-to-apns)
- Apple: [Pushing background updates to your app](https://developer.apple.com/documentation/usernotifications/pushing-background-updates-to-your-app)
- Apple: [Handling notification responses from APNs](https://developer.apple.com/documentation/usernotifications/handling-notification-responses-from-apns)

## Current Implementation

- iOS registers its APNs token on physical devices and sends it through
  `AppAction::SetPushToken`.
- Sign-out best-effort removes the token through `AppAction::RemovePushToken`.
- The server stores one token per account/device and removes it on device
  revocation.
- `finitechat-server push-drain` claims `/push-wakes`, sends APNs background
  pushes, acks delivered wakes, fails retryable wakes, and compare-and-deletes
  stale APNs tokens.
- APNs payload body is wake-only:

```json
{
  "aps": { "content-available": 1 },
  "room_id": "room-id",
  "seq": 123
}
```

No message body, sender name, attachment metadata, or decrypted room content is
sent to APNs.

## Apple Setup

1. In Certificates, Identifiers & Profiles, open the App ID for
   `computer.finite.finitechat` and enable Push Notifications.
2. In Xcode signing/capabilities, ensure Push Notifications is enabled and
   Background Modes includes Remote notifications.
3. This repo tracks the matching app config:
   - `ios/FiniteChat.entitlements` has `aps-environment`.
   - `ios/Info.plist` has `UIBackgroundModes = remote-notification`.
   - `ios/project.yml` sets `CODE_SIGN_ENTITLEMENTS`.
4. Create an APNs token authentication key in Apple Developer. Download the
   `.p8` once, record the Key ID, and record the Team ID. Store the key outside
   the repo, for example:

```bash
mkdir -p .state/secrets
chmod 700 .state/secrets
mv ~/Downloads/AuthKey_XXXXXXXXXX.p8 .state/secrets/
chmod 600 .state/secrets/AuthKey_XXXXXXXXXX.p8
```

Use development/sandbox for direct debug installs. Use production for TestFlight
and App Store builds. Environment mismatches usually show up as
`BadDeviceToken`, `DeviceTokenNotForTopic`, or missing delivery.

## Local Run

Start the home server:

```bash
cargo run -p finitechat-server -- serve 0.0.0.0:8787 --sqlite .state/finitechat.sqlite3
```

Start the APNs drain against the same server:

```bash
FINITECHAT_APNS_TOPIC=computer.finite.finitechat \
FINITECHAT_APNS_TEAM_ID=JBLHZ83X6T \
FINITECHAT_APNS_KEY_ID=XXXXXXXXXX \
FINITECHAT_APNS_PRIVATE_KEY_PATH=.state/secrets/AuthKey_XXXXXXXXXX.p8 \
FINITECHAT_APNS_ENV=sandbox \
cargo run -p finitechat-server -- push-drain --server-url http://127.0.0.1:8787
```

Useful options:

- `--once`: claim and process one batch, then exit.
- `--limit N`: max wakes to claim per batch. Default: 25.
- `--lease-ms N`: server lease per claimed wake. Default: 30000.
- `--interval-ms N`: polling interval in continuous mode. Default: 1000.
- `--apns-base-url URL`: override APNs endpoint for integration tests.

## Physical Proof Checklist

1. Install a debug build on a physical iPhone signed for
   `computer.finite.finitechat`.
2. Open the app once so APNs returns a device token and Finite Chat registers it
   on the home server. Simulators intentionally skip APNs registration.
3. Put the physical phone in a 1:1 or group room with another user/device.
4. Lock the physical phone. Do not force-quit the app; iOS may suppress
   background delivery for force-quit apps.
5. Send a new chat message from the other device.
6. Confirm the pusher logs a claimed/sent/acked batch.
7. Wake/unlock the phone and confirm the message is present after sync.

Record the first passing proof in the Friends Alpha notes with:

- iOS build configuration and signing environment.
- APNs environment used by `push-drain`.
- Server URL and commit hash.
- Pusher log line for the claimed wake.
- Confirmation that the APNs request body did not include plaintext.

## Troubleshooting

- No APNs token: use a physical iPhone, check `aps-environment`, check the App
  ID has Push Notifications enabled, and reinstall after provisioning changes.
- `BadDeviceToken`: debug build talking to production APNs, TestFlight build
  talking to sandbox APNs, wrong bundle topic, or stale token.
- `DeviceTokenNotForTopic`: `FINITECHAT_APNS_TOPIC` does not match the app
  bundle identifier in the provisioning profile.
- `InvalidProviderToken` or `ExpiredProviderToken`: check Team ID, Key ID, `.p8`
  contents, server clock, and APNs key status.
- Wakes are claimed but retried: inspect APNs response reason in the pusher
  logs, then confirm the token, topic, environment, and APNs key.
- Locked phone does not wake: iOS can throttle background pushes; keep the app
  recently opened, avoid Low Power Mode during proof, and verify the app was not
  force-quit.
