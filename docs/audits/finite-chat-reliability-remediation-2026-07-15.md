# Finite Chat Reliability Remediation

Date: 2026-07-15

Status: scoped implementation and local validation complete. This work must be
reviewed as one pull request and must not be deployed from this run.

## Scope ledger

| Symptom | Evidence-backed cause | This PR |
| --- | --- | --- |
| Tool calls collect above later chat text | Hermes accumulates progress by editing its first progress message even though Finite Chat disables editable streaming | Give only Finite Chat append-only progress defaults and classify the pinned Hermes tool vocabulary |
| Tool progress sometimes renders as ordinary prose | The bridge does not recognize several Hermes 0.18.2 tool icons | Expand the pinned, bounded compatibility vocabulary; do not classify arbitrary emoji as tools |
| Every message shows the same minute | The resident Rust bridge opens Core with a fixed startup clock | Open the resident runtime with its live clock |
| The Agent can be working with no visible activity | Finite's typing override delays refresh, does not bound bridge calls, and remembers only one route per room | Match Hermes's immediate/refresh/clear lifecycle and track exact topic/chat activity routes |
| Users cannot create a topic | The unified dashboard sidebar omitted the existing `CreateTopic` UI while protocol and Core support remained | Restore the small existing interaction against the canonical Agent room |
| Replies or status create an opaque extra topic | The adapter promotes an unknown Hermes thread id into a Finite conversation id | Preserve only explicit or remembered Finite routes; let unscoped traffic use Core's Home fallback |

## Deliberately deferred

- The report that a final message sometimes appears only after refresh remains
  tracked in `finite-chat-tool-ordering-2026-07-15.md`. A refresh repaired the
  observed UI, and there is not yet evidence that it shares the bridge cause.
- Existing phantom topics are not hidden, renamed, migrated, or deleted. This
  change prevents new invented routes; production durable state is untouched.
- This run does not automatically choose a first Finite conversation as a
  Hermes home channel. The canonical room is not an ordering or sort choice,
  and no change should override an explicit Finite or Telegram preference.
- There is no protocol change, editable-message implementation, new durable
  status message, or new topic model.

## Local acceptance

The focused gates must prove:

1. Pinned Hermes progress uses append-only records and never edits an older
   progress bubble across later output.
2. A real Agent turn shows scoped Working activity from handoff until final
   completion, including across multiple tool calls.
3. Two consecutive requests to publish a minimal Finite Site stay in one
   topic/chat, preserve monotonically increasing durable order, complete with a
   final delivery, and leave Working cleared.
4. Creating a topic through the dashboard selects the topic and its first chat.
5. Unknown Hermes thread metadata produces no new top-level topic and falls
   through to Home/Home-chat.
6. Reloading produces the same transcript order and non-frozen timestamps.

The existing Apple-container SaaS smoke is the canonical real-runtime path. It
should be extended or observed at its existing Hosted Web Device boundary;
this work must not create a second local stack or test-only chat protocol.

### Completed validation

The opt-in `DEVFINITY_CHAT_SITES_ACCEPTANCE=1` path passed on a fresh Apple
Container Agent on 2026-07-15. It exercised two real Hermes turns that built
and published public Sites, required append-only tool records in strictly
increasing durable order, observed scoped Working activity, fetched both
server-reported public URLs without viewer authentication, created and selected
a new topic plus its canonical first chat, restarted Finite Chat, Hosted Web
Device, and Finite Sites between turns, then restarted the Agent Runtime and
received a final reply without changing the Agent Principal.

This does not prove an existing-runtime artifact upgrade. The Apple Container
runner advertises `runtime_upgrade=false`, so the acceptance above launched the
candidate image directly and later exercised a same-image restart. A future
local upgrade fixture should seed chat state under an older image, replace only
the Apple compute while retaining the exact `/data` bind, and compare the
historical transcript before and after replacement. Until that exists, the
production Kata canary path remains the required old-artifact-to-new-artifact
upgrade acceptance gate.

The final deterministic gates also passed:

- 55 pinned Hermes 0.18.2 adapter tests and 8 runtime reconciler tests;
- Ruff check and format check for all touched Python files;
- 201 dashboard unit tests, dashboard ESLint, and the Chrome-backed dashboard
  agent-creation/topic browser fixture;
- the targeted resident-runtime live-clock Rust test and `cargo fmt --check`;
- `bash -n scripts/devfinity-saas-smoke` and `git diff --check`.

The focused Apple path skips the unrelated Brain and private-Sites cookie
checks. The default comprehensive SaaS smoke continues to run those checks
unless the explicit opt-in is set.

## Live-platform risk

- Append-only progress and timestamp changes live in the Agent Runtime image;
  they affect Agents only after a later, separately approved image rollout.
- Topic creation is a low-risk dashboard restoration over an existing action.
- Ambiguous route fallback is low-to-medium risk: only unverified thread-only
  metadata changes behavior, from manufacturing a topic to Core's established
  Home fallback. Explicit and remembered topic/chat routes remain unchanged.
- The working-status change is ephemeral only; it does not add durable chat
  records or alter model execution.
