# Shared, notification-only SimpleX wake

This variant supersedes the per-agent external SimpleX transport in the parent
spike. Two agents now sleep with their own SimpleX clients, chat databases,
Hermes gateways, native `hermes serve`, and desktops. One shared Kubernetes
Deployment receives opaque SMP notifications for both agents and requests their
Substrate URLs. It has no message-receive credentials, message-decryption keys,
notification-metadata decryption keys, or chat databases. Core is unchanged.

```
human -> public SimpleX messaging server -> encrypted message waits for actor
                        |
                        +-> NMSG -> shared notification listener -> actor URL
                                                                     |
                                                  Substrate restores actor
                                                                     |
                                      agent's SimpleX receives/decrypts -> Hermes
```

## What runs where

- **Inside each suspendable actor:** the existing SimpleX CLI, Hermes serve and
  gateway, desktop, and `enroll.py`. Enrollment reads the actor's live SimpleX
  agent DB read-only, enables notifications with `NKEY`, retains notification
  metadata keys locally, and registers only the notification capability with
  the shared listener. It reconciles the current queue list every ten seconds
  while awake. No SimpleX database is copied outside the actor.
- **One always-on pod, `simplex-notifications`:** `listener.py`, one SMP connection
  per server (shared across owners), and a small SQLite PVC containing scoped
  registrations, pending wake intents, and counters. It uses `NSUB` and reacts to
  `NMSG`; it neither downloads nor decodes messages. Owner bearer tokens map to
  fixed actor destinations supplied by the operator. The pod has no Kubernetes
  service-account token. It can wake agents, observe their activity, or disrupt
  notifications if compromised, but its notification credentials cannot read
  chat messages. This is a credential boundary, not protection against a cluster
  administrator who can separately access the actors.
- **Existing Substrate infrastructure:** router, control plane, snapshot storage,
  and available workers. This does not prove cluster scale-to-zero.

There are no always-on SimpleX daemons for the agents in this variant. The old
`simplex-alice` and `simplex-bob` deployments are scaled to zero, with their old
PVCs retained. External human test clients are separate from agent infrastructure.

## Why this route

The upstream notification router currently delivers through APNS and uses
PostgreSQL. Instead of adding a new push provider, this spike uses its underlying
notification capability directly. No SimpleX source fork or messaging-server
change was needed for the local proof. Hermes still uses its ordinary local
SimpleX WebSocket; the earlier adapter's contact-request compatibility fix is
inherited from the tested base image, but its relay ACK mechanism is unused here.

The protocol separation is native SimpleX:
[NKEY / NSUB / NMSG](https://github.com/simplex-chat/simplexmq/blob/27a37387be98d9c7ec0e62373e125539675d0095/protocol/simplex-messaging.md).
Source inspected: simplexmq `27a37387be98d9c7ec0e62373e125539675d0095` and
simplex-chat `fd160485984dc7522d8e08035e18a6a6bec8a33e`.

`smp.py` is deliberately a narrow spike implementation, not a replacement SMP
library. Public servers in this run rejected v6 (one advertised versions 14–21),
so it negotiates v14, authenticates commands with Ed25519, and pins the queue's
CA, validates its certificate chain and dates, and verifies the TLS session
binding. It uses TLS 1.2 and the protocol's proxy flag to omit the extra encrypted
block layer *inside* TLS. It does not proxy traffic. Upstream disables TLS secure
renegotiation, so this client permits the initial legacy-server connection.
It uses CPython's private certificate-chain API. Replace this transport with
upstream's library before production; do not grow an independent crypto stack.

## Measured acceptance

See checked-in [evidence](../evidence/) JSON files:

- `private-wake-alice.json`: two real SimpleX sleep/wake/model-recall cycles,
  6.775s and 11.025s from message send to reply.
- `private-wake-bob.json`: same for the second owner, 11.178s and 21.688s.
- `private-notification-restart.json`: actor suspended, shared listener scaled
  to zero and pod deleted, message sent, actor confirmed still suspended, then
  listener restarted from its notification-only PVC. A real model reply with
  prior conversation context arrived 15.245s after restart. The test made zero
  actor HTTP requests.
- `private-wake-rotation.json`: native `/_switch` rotates an existing receive
  queue; shared registration changes and settles to its original queue count;
  subsequent SimpleX sleep/wake/model-recall cycles pass.
- `private-notification-isolation.json`: both actors suspended; messaging Alice
  wakes only Alice and leaves Bob suspended.
- `notification-capabilities.json`: all four current notification credentials
  are rejected for `GET` with `ERR AUTH`; incorrect server CA pins are rejected.
- `notification-http.json`: bad token rejected; client-selected owner rejected;
  registration containing a decryption-key field rejected.
- `notification-desktop.png`: a fresh screenshot from the restored actor
  showing the actual Xvfb/Openbox desktop.
- `private-notification-native-chat.json`: both owners retain native Hermes
  WebSocket model conversations across suspend and URL wake.

The SimpleX harness checks suspended state twice, three seconds apart, then
sends/polls only the synthetic human endpoint and reads provider state. It makes
no actor requests during each measured message wake. Test model remains the
real Finite Private `glm-5-3-flash`, using the previously authorized key file.

## Reproduce against the existing local spike

Use the parent's Nix environment and isolated kubeconfig. Build with
`SPIKE_BASE_IMAGE` set to the prior tested relay image digest,
`SPIKE_IMAGE` set to a local registry tag, and `bash notifications/build.sh`.
Render private fixtures with `notifications/render.py --state-dir STATE
--image DIGEST --key-file KEY_FILE`. STATE must contain the baseline actor
fixtures. The renderer never prints or commits credentials.

Apply `notification-service.json`, create both `*-notify.template.json` templates,
wait for golden tags, create `alice-notify` and `bob-notify`, then grant their
synthetic egress policies using the parent's `egress-fixture.go`. Apply
`notification-ingress.json`, restart ingress, and forward its native API to
loopback 18080. Forward notification status to loopback 18767. Port-forwards
must be restarted after the selected pod is replaced.

Run `notifications/qualify.py --state-dir STATE --output REPORT --user alice
--human-container finite-simplex-test-user`, and similarly Bob with
`finite-simplex-test-bob`. Use `--paired` on repeat runs. `--rotate` rotates the
agent's receive queue before repeating the wake checks. These commands depend
on the synthetic human clients described in the parent setup, not personal
contacts. Test contact names are `alice-notify` and `bob-notify`.

Run `notifications/qualify-restart.py --previous-proof ALICE_REPORT --output
REPORT` after Alice's qualification. Run `check-capabilities.py` and
`check-http.py` inside the listener pod; they print only sanitized results.
Run the parent's `qualify-chat.py` with `--owners notifications/owners.json`
for the native WebSocket proof.

Exact tested images (ARM64, local registry):

- actors: `localhost:5017/finite-hermes-spike@sha256:72801d2e4cfc25b71af3ca2e1266178a3d29d80fbf2dd817884891038cdcb03f`
- shared listener: `localhost:5017/finite-hermes-spike@sha256:1253007727349d7f2339f24a3e96ba6a119d134b95b9e10b545676ba03398bb0`

The second image corrects the v14 `NSUB` response (`OK`, rather than newer
`SOK`). Actor runtime and enrollment are unchanged between these images.

## Remaining production gates

1. Replace read-only SQLite schema coupling with a SimpleX library/API enrollment
   hook, and use the upstream transport. Block sleep until all current receive
   queues are registered. Native notification mode must remain off with this
   prototype: it deliberately refuses to overwrite native notification keys.
2. Notifications are wake hints, not a durable delivery guarantee. The short
   outage proof passed, but notifications can expire; a crash between socket
   receive and storing wake intent can lose a hint. Add a bounded reconciliation
   policy for queued messages and test prolonged outages. Listener success means
   the actor's HTTP service resumed, not that Hermes durably completed a turn.
3. Qualify idle-suspend arbitration, load, rate limits, cross-owner admission,
   per-owner resource fairness, and failure recovery. The current wake loop is
   serial and registration caps each owner at 128 queues. Pod memory/cost at
   fleet scale is unmeasured. Notification timing is visible to the listener.
4. Restore the recovery set onto an empty target: agent snapshots hold all chat
   data/identity and local enrollment state; listener PVC holds registrations
   and pending hints. Registrations can be reconstructed by waking intact
   actors, but that reconstruction has not been qualified after volume loss.
5. Production TLS/network isolation for internal registration, enrollment token
   rotation, GKE deployment, and public DNS/TLS remain unqualified. This ran only
   in local Kind. Current Substrate hostPath requirements still make ordinary
   GKE Autopilot a poor fit. No Google Cloud resources were created.

Rollback for this disposable spike: stop the notification listener and use
native URL wake; this preserves actor data. The previous transport variant and
its retained PVCs are separate identities and can be restored separately. Do
not treat switching ingress between variants as an identity migration.

## Retained local demo

Both `alice-notify` and `bob-notify` are left suspended with no worker assigned.
The one shared notification deployment remains at one replica. Both old
per-agent transport deployments remain at zero replicas. Native ingress routes
`alice.agents.test` and `bob.agents.test` to the new agents and is forwarded only
to localhost:18080; no public DNS was changed. All prior synthetic actor
snapshots and old transport PVCs are retained. No production state was touched.
