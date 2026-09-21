# Local evidence — 2026-09-21

Acceptance is **partial**, not complete: the actual model-backed conversation
has not run because the Finite Private test credential is unavailable. The
previous session's referenced key file no longer exists. No substitute key or
mock model was used.

## Live qualification

Two complete cycles against the real Kind/Substrate/gVisor/Hermes stack passed.
The repeat cycle additionally compared the exact ID and start timestamp of a
live, unused Hermes session, rather than merely looking for historical chat.

| Check | Result |
| --- | --- |
| Two separate owner actors and native credentials | PASS |
| Anonymous rejection and cross-owner bearer rejection | PASS |
| Client-supplied actor routing header overwritten | PASS |
| Native Hermes JSON-RPC WebSocket, shell, X11 screenshot | PASS for both |
| Alice suspended with no worker assigned | PASS |
| Bob remains usable while Alice sleeps | PASS |
| Same Alice URL wakes the suspended actor | PASS |
| Home marker and live in-memory session survive | PASS |
| X11 screenshot and SimpleX daemon WS handshake after wake | PASS |
| Actual model chat and conversation continuity | BLOCKED on test key |
| SimpleX paired-user message delivery and incoming-message wake | NOT PROVEN |

Measured suspend: 0.873s and 0.818s. Request wake plus native shell marker read:
3.039s and 2.918s. These are two local observations, not an SLA or load test.
Reports: [first](evidence/lifecycle.json), [repeat](evidence/lifecycle-repeat.json).
The transport probe does not establish successful model inference or whether a
model chooses the correct screenshot tool.

![Alice's real X11 desktop](evidence/alice-desktop.png)

## Reproduced upstream failure

Full snapshot succeeded but subsequent cleanup of a read-only Hermes skill
directory failed with permission denied. The worker dropped all capabilities.
The regression failed before the one-line upstream cleanup change and passed
with it, running the Linux ARM64 Go test binary in Docker with `--cap-drop=ALL`.
The existing root-removal and populated-volume tests also passed. Both lifecycle
cycles above used the patched atelet, not stock upstream.

## Exact local images

- Hermes: `localhost:5017/finite-hermes-spike@sha256:14c199557770d3ad3b63f3b73eec8a5f8d35614189bd5bbc7a56e7c8664fa7fd`
- Worker: `localhost:5017/ateom-gvisor-715889664656de67e44382a8d6ab981d@sha256:30f456cf61710f70331e071d0b06f8b63898846619a27b5afba0f7bd5f5eea46`
- Patched atelet: `localhost:5017/atelet-89dbecdd4e8d5cd4d125a2de341f399c@sha256:d1d9f15981555e40c80ae8031d90ab7724286a50cbf6103f31a7c6d1fe1fe79c`

These registry digests describe this machine's disposable proof, not published
release artifacts. State and private fixture credentials remain outside git in
`/private/tmp/finite-substrate-state`. The separate kubeconfig points only to
`kind-finite-hermes-spike`. No cloud resources or production state were changed.

## Remaining acceptance run

Supply the original test key by its local file/secret reference, configure the
native Hermes model settings securely for both owners, then run a real prompt
for each through `probe.py --prompt`. Suspend one after a successful turn, wake
through its URL, resume the same conversation and verify its earlier content.
Pair a real SimpleX client separately to qualify delivery/reconnection; a daemon
handshake is insufficient. Incoming SimpleX wake needs an external trigger or
an explicit polling contract before this can replace the runner product.
