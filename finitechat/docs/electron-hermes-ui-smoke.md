# Electron Hermes UI Smoke

This smoke is the acceptance gate for the desktop path that must not be
replaced by daemon health checks alone.

It proves:

- Electron launches with fresh desktop state.
- The rendered onboarding flow can be completed.
- The visible `Paste Hermes npub or profile link` control accepts a real Hermes
  npub or profile link.
- The app creates a connected Home topic chat with Hermes.
- A message sent from the visible composer reaches Hermes.
- A non-user reply from Hermes appears in the projected chat state.

Run it from the Electron app:

```sh
cd apps/electron-chat
npm run smoke:hermes
```

By default the smoke uses `https://chat.finite.computer` and discovers the
newest running Apple `container` whose id or image looks like a Finite agent. It
reads only the public Hermes npub from `/tmp/finitechat-invite.json`.

To pin the target explicitly:

```sh
FINITECHAT_E2E_HERMES_TARGET=npub1... npm run smoke:hermes
```

Useful knobs:

- `FINITECHAT_E2E_SERVER_URL`: chat server URL.
- `FINITECHAT_E2E_CONTAINER`: Apple container id to inspect for the Hermes npub.
- `FINITECHAT_E2E_REPLY_TIMEOUT_MS`: default `240000`.
- `FINITECHAT_E2E_KEEP_APP=1`: leave Electron open for debugging.

Each run writes evidence under `target/electron-hermes-ui-smoke/<run-id>/`:

- `report.json`: step timings, room/topic/chat ids, Hermes sender, reply preview.
- `success.png` or `failure.png`: renderer screenshot.
- `success-state.json` or `failure-state.json`: daemon app state with secrets
  redacted.

