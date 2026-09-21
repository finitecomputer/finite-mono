# SimpleX wake latency spike

This extends the [notification-only spike](../notifications/README.md). Core and
production runners are unchanged. All measurements used synthetic Alice/Bob
identities, real public SimpleX relays, real Hermes, and the authorized Finite
Private model on the local kind/Substrate cluster. No Google resources were used.

## What improved

| Measurement | Before | Selected experiment |
| --- | --- | --- |
| Short complete reply after sleep | Alice: 7.086–10.282 s; median 9.567 s, n=5 | Alice: 4.531–5.887 s; median 5.344 s, n=3. Bob: 4.700–5.760 s; median 4.765 s, n=3 |
| First text of longer reply while awake | Bob buffered: 4.016 and 8.346 s | Bob streamed: 1.504 and 1.546 s |
| First text of longer reply after prepared sleep | No matched baseline | Alice: 5.003–5.376 s, n=3; final text at 10.277–11.011 s |
| Listener dispatch after notification | Polling added up to roughly 1 s | Event-driven, normally a few milliseconds |
| Hermes incoming text batch | 800 ms | 50 ms |

The faster sleep numbers **require cooperative SimpleX preparation before the
provider checkpoint**, followed by native activation after restore. This is a
controlled spike result, not a production latency SLO or a finished sleep policy.
An ordinary provider suspend still measured approximately 8–9 s for short replies.
A separate 45-second idle trial completed in 4.700 s.
Small sequential samples, different relay assignments, model variability, and a
warm local worker pool do not establish p95 or GKE cold-node performance.

The original 7–22 s observations included two-second human-message polling and
whole model replies. These tests instead observe the external human client's
WebSocket and separately record first text and final text. No request from the
benchmark touches the sleeping actor during a measured wake. The shared listener
is the sole source of actor HTTP wake requests. Provider state reads are allowed.

Raw, content-free results and phase summaries are in [evidence](evidence/).
`model_turn_start` brackets `run_conversation`, not necessarily the actual first
network request. Missing model-request/first-token instrumentation is not zero
model latency. Initial cold turns are retained in raw reports and excluded from
sleep medians; initialization added several seconds on first use.

## The changes

1. **Immediate shared wake dispatch.** SQLite pending rows remain authoritative.
   A condition variable replaces the one-second poll. Up to eight different owners
   can wake concurrently; each owner has at most one request in flight. HTTP
   failure retains the intent and retries; completion deletes only the intent it
   actually handled. A newer event survives an older request's completion.
2. **A narrow authenticated resume signal.** The listener sends its existing
   owner-scoped wake credential with the fixed actor destination. A small native
   `/api/status` hook signals the actor-local gateway. Its single SimpleX reader
   issues native commands and acknowledges the signal; a missing acknowledgment
   returns 503 so the shared listener retains the durable wake intent. This is
   command acceptance, not proof that a chat message or model reply was delivered.
3. **Native SimpleX lifecycle.** The selected command is `/_app activate`.
   The fast experiment first sends `/_app suspend 0` while the actor is awake,
   then asks Substrate to checkpoint it. Native activation after restore reduced
   HTTP-ready-to-message-received from roughly five seconds to roughly one in
   successful prepared trials. No SimpleX process, chat DB, or message key moves
   into the shared listener.
4. **Shorter message batching.** 50 ms prioritizes first-message latency. Messages
   farther apart will become separate Hermes inputs; multi-message burst behavior
   needs broader qualification before changing the product default.
5. **Incremental replies through native edits.** The adapter uses correlated
   `/_send` responses to obtain a stable item ID and `/_update item` for updates.
   Hermes's existing stream consumer owns buffering and finalization. The tested
   human receives updates to one item, followed by a final edit without the cursor.
   Pairing callbacks retain the original fire-and-forget path to avoid waiting
   for a response on the same reader that must consume it. Media retains the
   original path. Group/media/mixed-version streaming are not qualified here.

No new always-running per-agent transport exists. The shared notification pod
still stores only notification capabilities and opaque wake intent. Hermes,
SimpleX, their plaintext-capable state, and the desktop all sleep with the actor.
The cluster's existing routing, control plane, and worker capacity remain shared
infrastructure. None of this strengthens the operator-blindness claims in the
original spike.

## Experiments that did not help

- Explicit `/reconnect` after restore: acknowledged quickly, but did not remove
  the subsequent roughly five-second recovery delay.
- Activate + reconnect + resubscribe after restore: same problem.
- Preparing before sleep, then activating **and reconnecting again**: regressed
  to roughly 28 s. The resubscribe command timed out; later recovery still replied.
- Suspend/reconnect/activate entirely after restore: approximately 8.3–8.8 s.
- DNS immediately after restore: 11–28 ms in the diagnostic samples, not the
  several-second bottleneck.

The selected configuration omits forced reconnect and resubscribe. Those commands
remain selectable only to reproduce the experiments. The post-restore SimpleX
recovery boundary is measured; the exact internal socket/retry mechanism causing
all of its delay is not proven. Do not present a guessed timer as a measured cause.

## Production gates and remaining latency

**Cooperative sleep must serialize against wake before shipping the faster path.**
The benchmark deliberately sends only after preparation and checkpoint complete.
A notification arriving between preparation and checkpoint could otherwise be
acknowledged against an actor that is about to sleep. Production needs one owner
of the sleep/wake transition, durable wake intent through that transition, and
fault/race tests for messages during preparation, checkpoint, and controller
restart. The local `--prepare-simplex` flag is a test fixture, not that coordinator.
Do not add independent Core and listener lifecycle state machines to solve it.

The runtime's wall-clock gap detector is also a spike heuristic. A provider resume
notification should replace it: a long event-loop stall is not proof of restore.
The shared dispatcher race tests cover pending wake delivery, not the separate
prepare/checkpoint race above. Existing relay-notification expiry/offline recovery
and full Recovery Set restore gates from the parent spike still apply.

Other remaining costs:

- Public relay notification delivery is outside our controller. The pinned SMP
  server source defaults to a 1.5 s notification batch; that does not establish
  the configuration of every public relay. Hosting/tuning a relay could reduce
  it, but adds operations and was not done here.
- Warm local snapshot restore generally took about one second. GKE node startup,
  image pulls, snapshot storage, and placement need their own measurements. Keep
  a small shared warm worker pool if low wake latency matters; do not promise the
  same result from a cold cluster.
- SimpleX resubscription and model first text still take network round trips.
  Region placement is worth measuring; no cloud latency estimate is claimed.
- Cold Hermes/tool initialization still costs several seconds. Existing upstream
  startup warm-up does not eliminate every first-turn cost. Do not hide it by
  mixing cold and warm samples or silently removing tool capabilities.
- Streaming improves time to useful text, not model completion time. Edits should
  eventually be integrated into the upstream adapter with failure, cancellation,
  long-answer splitting, and older-client tests.

## Reproduce

Use the parent spike's pinned toolchain and local cluster setup. The source pin is
Hermes `29112bef099274229cadff79cdff7bf7b99c4b77`; the latency builder extracts pristine
files from that commit, applies the existing compatibility patch, then the narrow
latency changes. It patches both Nix package copies, including the copy imported
by `hermes serve`. Patching only the other copy silently misses the HTTP hook.

```sh
SPIKE_STATE_DIR=/private/tmp/finite-substrate-state \
SPIKE_HERMES_SOURCE=/private/tmp/finite-substrate-hermes \
SPIKE_BASE_IMAGE=localhost:5017/finite-hermes-spike@sha256:1253007727349d7f2339f24a3e96ba6a119d134b95b9e10b545676ba03398bb0 \
SPIKE_IMAGE=localhost:5017/finite-hermes-spike:latency-qualified \
  bash infra/spikes/substrate-hermes/latency/build.sh
```

The measured final runtime was image `ae10215865c90be52c5989d5f3b05b8a3368efadac01bb50150cfd576c093235`
with explicit mode settings matching the final defaults. The reproducible final
build is `5160f84c5949436b6106fd0333e9eb7c9b7a79ad4d053c17244bc06b6b37116c`;
its selected command/default streaming settings changed, not the exercised
selected command path. Changing the Nix closure requires updating the pinned module paths and requalifying; this Dockerfile intentionally targets the measured closure.

Render private fixtures with `notifications/render.py --suffix resume --service
simplex-resume`, using the same private baseline fixture inputs as the parent
spike. The renderer writes an `owners.json` alongside the private credentials.
Apply templates/service/ingress, wait for golden snapshots, create both actors,
and grant local-fixture egress. Bootstrap each through native Hermes pairing with
`notifications/qualify.py --bootstrap-only --owners "$fixtures/owners.json"`.
Never commit rendered templates or credentials. The key remains in the authorized
Downloads file; its value is not in this tree or the images.

Copy `latency/human-turn.py` into each synthetic human container at
`/home/agent/human-turn.py`. Set `KUBECONFIG`, `ATE_CLI`, and use the Nix Python
with requests/websockets, as in the parent instructions.

```sh
python latency/set-mode.py --state "$state" --owners "$fixtures/owners.json" \
  --streaming --wake-command '/_app activate'
python latency/benchmark.py --state "$state" --owners "$fixtures/owners.json" \
  --service simplex-resume --label prepared --prepare-simplex \
  --warm 1 --cycles 3 --output "$state/result.json"
# Repeat with --user bob. Add --long for first-text/final-edit measurements.
# Omit --prepare-simplex to measure an ordinary provider suspend.
# --idle-seconds 45 checks beyond the three-second short-cycle fixture.
python latency/summarize.py "$state/result.json"
```

`latency/test-wake-concurrency.py` runs inside the image against real HTTP and
SQLite. It proves concurrent owners, durable startup recovery, failed-request
retry, and a new notification arriving during an older in-flight wake. The native
qualification scripts exercise auth isolation, WS chat, RAM/filesystem restore,
and desktop screenshots against the same two owners.

Final checks passed: both native Hermes conversations recalled their verification words after URL wake (4.511 s and 3.840 s); native auth/owner isolation, live RAM state, files and X11 screenshots survived restore. Eight long-response trials used one message ID and a final edit. A real message sent with the shared listener down recovered after restart and woke only Alice; Bob stayed suspended. Wake credentials were rejected as native chat authentication. Both accepted actors were left suspended and only `simplex-resume` was left active among the notification/transport variants.
