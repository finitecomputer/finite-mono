# Boss Hosted Chat Recovery Post-mortem

Date: 2026-07-16

Status: incident fixed in production; follow-up actions proposed, not
implemented by this report.

## Executive summary

Boss was not stranded by a bad Agent Runtime upgrade. Its Kata container and
durable state were present, and its `/contact` endpoint returned `200`,
`ready: true`, and the expected Agent Principal. The failure was in the
dashboard-to-Hosted-Device binding bootstrap for an Agent created before the
current authorization flow.

Hosted Web Device correctly refused to create Boss's canonical Agent
conversation without a one-time authorization tied to the original Project
creation request. The dashboard had an explicit recovery action for that
legacy state, but it did not recognize the real service error:

- the browser fixture modeled `409` with
  `first-time binding bootstrap was not authorized by Project creation`;
- production returned `503` with
  `canonical Agent conversation requires recovery: first-time binding bootstrap was not authorized by Project creation`.

My first fix, PR #106, accepted `503` but continued matching the shorter
message. I then wrote a unit test that constructed the same incorrect synthetic
error. The browser fixture was not updated, so the green test suite proved the
assumption in the patch instead of the production contract. We deployed it and
Boss remained broken.

After the failed deploy, production logs continued to show the prefixed `503`.
I initially speculated about JavaScript `instanceof` behavior instead of first
comparing the literal producer response with the dashboard matcher. Reading the
Rust error producer made the actual mismatch obvious. PR #108 corrected the
exact contract and changed the browser fixture to emit the production status
and message. That version was deployed as dashboard `2026-07-16.4` and Boss's
Runtime remained healthy throughout the final fix.

The central lesson is different from the Runtime lifecycle post-mortem: a bot
can be functionally stranded even when its compute and data are healthy if a
new control-plane protocol is not tested against old durable/control state. A
fresh canary is insufficient for migrations like this because fresh Agents
receive the new authorization during creation and never exercise the legacy
recovery path.

## Impact

- Skyler could not open Boss chat and repeatedly saw a “Getting ready” or
  recovery/loading state.
- A supported Boss restart briefly interrupted the Agent but did not address
  the binding problem.
- The first dashboard hotfix was ineffective, extending the incident and
  requiring another build and deploy.
- Open browser tabs also produced stale Next.js Server Action identifiers after
  dashboard replacement, requiring a hard refresh and adding a second failure
  signal during verification.
- The nominal dashboard-only deploy restarted Core, Hosted Web Device, Finite
  Chat, Sites, and Brain because the Nix Rust packages depend on the complete
  monorepo source revision.
- The separate report that a newly paid user could not advance through Agent
  creation was not proven to have this same cause and should not be
  retrospectively collapsed into the Boss binding incident.
- No Agent Runtime image was rolled for this fix. Boss's canonical compute,
  `/data`, and Agent Principal were not replaced.

## Timeline

All times below are UTC.

### 2026-07-13 — the compatibility gap ships

PR #32 introduced the durable canonical Agent binding and the explicit
recovery action. New Agent creation authorized binding bootstrap using the
successful Core creation request. Existing Agents such as Boss had been
created before that authorization record existed.

The intended legacy recovery was safe: load one fresh Core snapshot, prove the
signed-in WorkOS user and current Project, require exactly one matching
creation request, authorize that exact Project/request pair, then retry binding
against the same Runtime contact.

However, the browser fixture modeled the missing-authorization response as the
old short `409`. The real Rust service mapped `AgentBindingInvalid` to `503`
and serialized the full `thiserror` display string, including the
`canonical Agent conversation requires recovery:` prefix. The recovery UI was
therefore not reachable from the actual production response.

### 2026-07-16 — Boss appears offline

Boss was restarted through the supported audited Admin Runtime operation. The
restart succeeded, the canonical container was healthy, and `/contact`
returned the Agent Principal. This separated Runtime health from the remaining
chat setup failure, but the restart itself could not create Hosted Device
binding authorization.

### 20:47–21:08 — the first fix is merged and deployed

PR #106 recognized that production used `503`, but the patch accepted either
`409` or `503` only when the message equaled the short suffix. Its new unit test
manually constructed a `503` with that same suffix. The browser fixture still
returned the short `409`.

All CI checks passed because no test passed the real Rust response through the
dashboard classifier. Dashboard `2026-07-16.3` was deployed.

### 21:38–22:44 — production proves the first fix failed

Dashboard logs repeatedly recorded:

```text
canonical Agent conversation requires recovery: first-time binding bootstrap was not authorized by Project creation
```

Because the error had not been converted into the dashboard's typed
`binding_authorization_required` response, the state route treated it as an
unexpected infrastructure failure. The UI could not reliably offer the
working recovery action.

### 23:29 — stale browser code adds noise

The dashboard logged `Failed to find Server Action` for a request carrying an
action identifier from another dashboard build. This did not cause the Hosted
Device binding mismatch, but it meant a user could continue interacting with
stale JavaScript after a hotfix and see a different failure until reloading.

### 23:49–00:05 — the production-shaped fix lands

PR #108 changed the `503` matcher to the complete production message, retained
the legacy short `409` compatibility case, and changed the browser fixture to
emit the actual `503` plus prefix. The full browser suite, 205 unit tests,
lint, build, and repository CI passed.

PR #109 pinned dashboard `2026-07-16.4`. It was merged before its checks had
completed. Its dashboard job later failed in an unrelated onboarding browser
step where `Continue` remained disabled. I deployed after reviewing that the
same dashboard source had passed PR #108 and the production-shaped local suite,
and after the Nix evaluation passed. The deployed code was correct, but
shipping from a red deploy PR was still a process failure and should not be
normalized.

The final deployment replaced the dashboard and restarted the wider service
spine. Public health, relevant systemd units, the exact dashboard digest, the
Boss container, and Boss `/contact` were verified afterward.

## What I got wrong

### 1. I fixed one field of the response without inspecting the whole contract

I focused on `409` versus `503`. The production log and Rust producer contained
the other half of the mismatch: the message prefix. A response contract is the
status, body shape, stable code, and compatibility behavior together.

### 2. I wrote a test from the patch's assumption

The new unit test instantiated `HostedDeviceRequestError` with the string the
dashboard wanted to receive. It did not derive that error from an actual Hosted
Device response or even copy the current Rust producer's full output. This was
a tautological test.

### 3. I missed that an existing browser test was also synthetic and stale

The end-to-end dashboard browser fixture already exercised the legacy recovery
button, which looked reassuring. But its fake Hosted Device still returned the
short `409`. I did not update or challenge the fake in PR #106, so browser
coverage concealed rather than caught the compatibility bug.

### 4. I used a production restart as a diagnostic before fully separating layers

The restart was supported, audited, and successful, but it could not repair a
Hosted Device authorization record. Once `/contact` was healthy, further
Runtime action was irrelevant. The layer check should have been explicit
before the restart: canonical compute, `/contact`, binding open/ensure, then UI
projection.

### 5. I speculated after the failed deploy instead of reading the producer

I considered a bundled-class/`instanceof` mismatch. That was possible in the
abstract but unsupported by the evidence. The literal production message and
the `AgentBindingInvalid` `IntoResponse` implementation were enough to identify
the prefix mismatch directly.

### 6. I accepted a red deploy PR

PR #109 merged immediately when I attempted to enable auto-merge, rather than
waiting for required checks. When its unrelated browser test failed, I used the
green source PR plus local evidence to justify the deploy. That evidence was
strong enough to assess this patch, but the release process should never make
an operator reinterpret a red pin PR during an incident.

### 7. I did not account for stale clients in the acceptance plan

Replacing a Next.js deployment invalidated Server Action identifiers held by
already-open tabs. “Hard refresh and try again” was operationally effective but
is not an acceptable product-level upgrade strategy.

## Root causes

### Technical root cause

Hosted Device exposed a human-readable Rust error string as the machine
contract. The dashboard duplicated that string, and its browser fake duplicated
a different historical response. There was no stable machine-readable error
code shared across the HTTP boundary.

### Compatibility root cause

The new binding authorization was applied automatically only during new Agent
creation. Existing Agents required recovery, but release acceptance used fresh
or already-bound canaries. It did not carry a pre-authorization Project into
the new control plane and prove recovery.

### Test root cause

Unit and browser tests terminated at a TypeScript fake rather than the actual
Rust Hosted Device boundary. The fake was treated as a specification even
after the producer behavior changed.

### Release root cause

Dashboard build/deploy acceptance proved the new bundle loaded and ordinary
product flows worked. It did not test:

- an Agent created before the binding authorization migration;
- an already-bound existing Agent;
- an open browser tab surviving a deployment; or
- the exact set of host services a dashboard-only pin would restart.

## What worked

- The authorization check failed closed. Hosted Device did not create or bind
  a canonical Agent conversation without a proven creation request.
- The explicit recovery action was narrowly scoped and did not create another
  Project, guess by sort order, or bind an arbitrary Runtime.
- Boss's compute and durable state were healthy and remained intact.
- Production logs preserved the exact server error needed to diagnose the
  failed first fix.
- The second patch corrected both the implementation and the browser fixture,
  and retained only the exact legacy compatibility case.
- The final deploy was digest-pinned and verified against the running image.

## Required improvements

### P0 — before another Hosted Chat or dashboard compatibility rollout

#### 1. Replace prose matching with a stable error code

Hosted Device should return a bounded response such as:

```json
{
  "code": "agent_binding_authorization_required",
  "error": "canonical Agent conversation requires recovery: first-time binding bootstrap was not authorized by Project creation"
}
```

The dashboard should branch on the exact code and documented status. Keep the
current message fallback for one compatibility window, then remove it after the
fleet no longer needs the old response. Human copy may change without changing
control flow.

#### 2. Add one real cross-service compatibility test

The test must run the actual Rust Hosted Device and dashboard classifier, not a
TypeScript imitation:

1. seed a Project/Runtime/creation request without binding authorization;
2. call dashboard chat state and require the typed recovery code;
3. perform explicit recovery;
4. require the canonical binding and chat state; and
5. prove a different Project/request or changed Agent Principal fails closed.

The browser fake can remain for fast UI tests, but it must not be the only
contract proof.

#### 3. Add N-1 control-state canaries

Every migration that changes authorization, identity, durable metadata, or
protocol state needs three fixtures:

- a fresh Agent created entirely on N;
- an existing healthy Agent already migrated/bound; and
- an N-1 Agent missing the new state and using the supported recovery path.

Runtime image canaries and control-state canaries are different. A fresh
Runtime canary cannot prove an old dashboard/Hosted Device migration.

#### 4. Require green release pins

- Do not merge the Nix/image pin PR before all required checks complete.
- Do not deploy a red pin PR, even when the underlying source PR was green.
- Rerun an identified flaky job to green or create a clean replacement pin.
- Configure branch protection so `--auto` cannot merge immediately while
  required checks are pending.

#### 5. Make deploy impact explicit

Before activation, compare the current and candidate Nix closures and fail a
dashboard-only deployment if unrelated service units or executables would
change. Narrow the Nix Rust source inputs so dashboard and documentation
changes do not rebuild the entire service spine.

#### 6. Verify the affected state, not only global health

A chat recovery hotfix is complete only after checking:

- the target Agent's canonical container and `/contact` identity;
- the exact failing state route response before the fix;
- the recovery button and recovery request;
- chat state after recovery; and
- a second reload proving the binding is durable.

Generic `/healthz` and a newly created Agent are insufficient.

### P1 — improve diagnosis and deployment continuity

#### 1. Project typed stage telemetry

Log a bounded stage and stable code for `binding_open`, `runtime_contact`,
`binding_ensure`, `binding_authorize`, and `binding_reopen`. Include only safe
Project/Runtime identifiers and status; never log credentials or chat content.
The user can still see plain product copy, while operators can distinguish
“Runtime not ready” from “authorization recovery required.”

#### 2. Handle stale dashboard clients deliberately

Add an acceptance case that opens the dashboard under version N, deploys N+1,
then attempts a critical action. Prefer stable route handlers for recovery and
chat mutations. Where a stale Server Action cannot be honored, show one clear
“Finite was updated; reload” path or perform one bounded reload instead of
leaving the UI apparently stuck.

#### 3. Inventory legacy migration state before rollout

Before an authorization/state migration, report how many active Agents are:

- already on the new durable state;
- expected to recover on first use; or
- unable to satisfy the recovery preconditions.

This is read-only release planning. It prevents a single modern canary from
standing in for the actual fleet.

## Correct build and deploy checklist for this class of change

1. Identify the exact failing layer with read-only evidence.
2. Capture the real producer status and response body.
3. Reproduce that response through the consumer classifier.
4. Add a stable code if control flow currently depends on prose.
5. Test fresh, already-migrated, and N-1 recovery state.
6. Build the dashboard image from the green merged revision.
7. Open a pin PR and wait for every required check to pass.
8. Review the Nix changed-service set before switching.
9. Deploy once.
10. Verify the exact affected Agent and one stale browser session.
11. Only then call the incident fixed.

## Relationship to Agent Runtime upgrade safety

The broader [`Agent Runtime upgrade and rollout post-mortem`](agent-runtime-upgrade-rollout-2026-07-16.md)
addresses missing canonical compute, durable `/data`, artifact replacement,
backup/restore, and generic relaunch. Those are real risks, but they were not
the cause of Boss's chat failure.

Boss demonstrates a separate requirement: rollout safety includes backwards
compatibility for control-plane and durable protocol state. An Agent is usable
only when all of these remain compatible:

- Runtime image and mounted state;
- Core RuntimeSpec and lifecycle records;
- Hosted Device identity and binding records;
- dashboard API classification and recovery UI; and
- browser code still open during the deployment.

We should not describe a release as successfully rolled out merely because the
container is healthy and on the latest digest. The old Agent's actual user path
must work.

## Related evidence

- PR #32: Hosted chat continuity and explicit legacy binding recovery
- PR #106: ineffective first `503` matcher
- PR #108: production-shaped status/message fix
- PR #109: dashboard image pin and red browser rerun
- [`hosted-web-chat.ts`](../../finitecomputer-v2/apps/dashboard/src/lib/hosted-web-chat.ts)
- [`finitechat-hosted-device/src/lib.rs`](../../finitechat/crates/finitechat-hosted-device/src/lib.rs)
- [`runtime-rollout-gotchas-2026-07-16.md`](../audits/runtime-rollout-gotchas-2026-07-16.md)
