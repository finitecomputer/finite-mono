# Oops I Faked It Audit

Status: active audit after the June 2026 Hermes simulator demo.

The simulator demo that replied with `Hermes local demo reply: ...` was not a
real Hermes model run. It was a local echo agent under `.state/hermes-demo` that
proved pieces of the Finite Chat transport path, but it did not prove Hermes
reasoning, tool use, or media understanding.

## Fake Or Echo Paths

| Path | What It Proves | What It Does Not Prove | Required Label |
| --- | --- | --- | --- |
| `.state/hermes-demo/demo_agent.py` | Local server, invite, app join, app send, agent-home bridge send | Real Hermes gateway, model call, tool use, non-echo response | Disposable local echo demo only |
| removed `containers/agent/echo_agent.py` | Historical container wiring and adapter importability | Hermes behavior or product chat quality | Deleted; do not restore as acceptance infrastructure |
| `tests/hermes/test_live_hermes_agent_media_e2e.py` | Real `hermes-agent` package can load the Finite Chat adapter and round-trip transport/media through a live server | Real gateway loop or LLM response, because the test installs `set_message_handler` echo callbacks | Adapter transport E2E with echo handler |
| `tests/hermes/test_live_ios_simulator_hermes_media_e2e.py` | iOS app can join an agent invite and exchange media through the bridge | Real Hermes response, because the test agent echoes text/media | iOS adapter transport E2E with echo handler |
| `tests/hermes/test_live_ios_device_hermes_media_e2e.py` | Physical phone can exercise the same bridge path | Real Hermes response, because the test agent echoes text/media | Device adapter transport E2E with echo handler |

## Real Hermes Definition

A real Hermes proof must satisfy all of these:

- run Hermes with `gateway run`;
- load `integrations/hermes/finitechat` as a Hermes plugin;
- use `finitechat hermes poll/send/edit/activity` through the plugin;
- use a normal Hermes model provider config and provider key;
- avoid `adapter.set_message_handler(...)` or any custom echo callback;
- show a non-echo response to a user message;
- include media in at least one direction before calling the demo media-ready.

The repo-local low-level runner for manual debugging is:

```sh
scripts/hermes-real-gateway-demo.sh
```

It starts a local Finite Chat server, initializes a dedicated Hermes agent home,
copies the current finitechat plugin into Hermes, sources local provider
secrets without printing them, and runs the real Hermes gateway loop.
It is not the physical-phone product canary because it may use loopback URLs
and does not gate human handoff on a preflight admission probe. The product
canary runbook is `docs/hermes-phone-canary-loop.md`.

## Typing Indicator Finding

Hermes adapter activity already sends `activity_kind: "working"`, but the app
projection previously rendered only `typing`. That meant real Hermes work could
be projected and still stay invisible in the iOS transcript. The core projection
now treats reserved `typing`, `thinking`, and `working` activity as live chat
indicators. Swift still renders the existing `is typing` row because the
current app DTO does not carry the activity kind.

## Delete Conditions

- Echo media tests keep their names/docs honest or are replaced by real gateway
  tests.
- A simulator and physical-phone proof capture a back-and-forth app
  conversation with real Hermes, including media, through
  `docs/hermes-phone-canary-loop.md` or its successor.
- A product harness asserts there is no `Hermes local demo reply`, `agent text
  echo`, or `agent media echo` string in any accepted real-Hermes proof.
