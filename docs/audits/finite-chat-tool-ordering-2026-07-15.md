# Finite Chat Tool Ordering And Timestamp Audit

Date: 2026-07-15

Status: investigation only. No product code or production configuration was
changed, and nothing was deployed.

## Additional observation: final message appears one turn late

After this audit was completed, Upgrade Canary 0715 exhibited a new symptom:
the latest chat message did not appear until the user sent another message.
This reportedly began during the day and had not yet been reproduced on other
Agents when recorded. Treat it as an unconfirmed canary observation, not an
established fleet-wide regression.

It could be related to this audit if the final message is durably accepted but
the resident bridge or dashboard subscription does not publish or apply the
last update until another event arrives. It could also be unrelated—for
example, a canary-specific connection, subscription, or refresh problem. The
tool-ordering evidence does not establish either explanation: that issue
projects received edit events into an older transcript position, whereas this
new symptom may involve the final event not becoming visible at all.

Before combining the fixes, reproduce this separately and compare four points
for the missing final message: Hermes send completion, Core acceptance and
sequence, the live subscription event, and the dashboard's local transcript.
Refreshing before sending another message is also an important discriminator:
if refresh reveals it, persistence is likely intact and live invalidation is
the narrower suspect. Do not expand the tool-ordering fix unless that trace
shows a shared cause.

## Executive summary

The reported transcript is the result of three independent bugs at the Hermes
bridge/presentation boundary. The Finite Chat delivery protocol is preserving
the durable event order.

1. Hermes 0.18.2 defaults tool progress to `accumulate`: it sends the first
   tool line as one message and repeatedly edits that message to contain every
   later tool line. Finite Chat deliberately advertises that ordinary response
   streaming is unsupported, but Hermes's tool-progress sender still notices
   the adapter's `edit_message` method and edits the original progress message.
2. When Hermes sends an interim assistant/commentary message without its
   streaming consumer, it does not reset the accumulated progress group. Later
   tools therefore keep editing the progress bubble that was created above the
   commentary. The dashboard correctly projects an edit onto the original
   bubble, so later tool work appears to move upward in the transcript.
3. The accumulated message in the supplied example begins with `📚`. Finite
   Chat's bridge does not recognize that Hermes tool icon, classifies the whole
   payload as an ordinary `message`, and bypasses the dashboard's collapsible
   `Working`/`Worked through` component.

The repeated `4:17 PM` timestamp has a separate concrete cause. The resident
`finitechat hermes serve` process opens one `FiniteChatRuntime` with
`now_unix_seconds: Some(now_secs())`. That option is a fixed clock, and the
resident service reuses the same runtime for every later send. All messages
from that service lifetime can therefore inherit its startup minute.

This is a small-to-medium runtime/bridge fix. It does not require a protocol
change or a new dashboard transcript model. It does require a new Agent Runtime
image and an existing-agent rollout because the relevant bridge, config
reconciler, and CLI live in the Runtime image.

## Evidence

### Durable ordering is intact

Finite Chat exposes an append-only edit chain: an edit is a new event carrying
`edit_of`, and Core sorts projected messages by accepted `seq`. The shared UI
then deliberately collapses the edit chain by replacing the original item in
place.

A focused execution of the current `transcriptItems` implementation used this
accepted sequence:

| Seq | Durable event |
| ---: | --- |
| 1 | assistant introduction |
| 2 | tool progress `tool A` |
| 3 | commentary `commentary one` |
| 4 | edit of seq 2 containing `tool A` and later `tool B` |
| 5 | commentary `commentary two` |

The current projection rendered:

```text
introduction
tool A + tool B   (payload from seq 4, occupying seq 2's position)
commentary one    (seq 3)
commentary two    (seq 5)
```

No event was delivered out of sequence. The bridge used one old bubble as a
mutable container for later events, and the UI honored that edit relationship.

Relevant repository paths:

- `finitechat/packages/finitechat-chat-ui/src/presentation.ts` groups adjacent
  `kind === "tool"` messages and replaces edits at the target's existing array
  index.
- `finitechat/crates/finitechat-core/src/lib.rs` sorts messages by `seq`, then
  room and message id.
- `finitechat/crates/finitechat-hermes/src/lib.rs` defines `edit_of` as an
  append-only edit chain.

### Hermes accumulates across commentary on Finite Chat

The canonical Runtime image contains Hermes 0.18.2. Its gateway resolves the
default `tool_progress_grouping` to `accumulate`. The progress sender:

- sends the first tool line;
- stores that message id;
- edits the same message with the full accumulated list for later tools; and
- resets the group only after receiving an internal `__reset__` marker.

Hermes's streaming consumer normally emits that marker when an assistant
content bubble lands. Finite Chat sets `SUPPORTS_MESSAGE_EDITING = False`, so
Hermes intentionally skips the streaming consumer. Interim commentary is then
sent directly through `adapter.send`, but that direct path does not enqueue the
reset marker.

The tool-progress sender uses a different capability check: because Finite
Chat implements an append-only `edit_message` method, it continues to edit the
old progress bubble despite `SUPPORTS_MESSAGE_EDITING = False`. This mismatch
is the main ordering bug.

The installed Hermes source already contains a comment describing the exact
failure the reset marker is meant to prevent: without the reset, later tool
lines keep editing the original progress message above new content. Finite Chat
lands in that failure mode because its non-streaming commentary path never
produces the marker.

### Tool classification is inferred from an incomplete emoji list

The bridge infers `kind = "tool"` from the first line's icon. Its recognized
set currently covers icons such as `📖`, `💻`, `🔎`, and `🔧`, but Hermes 0.18.2
also emits at least:

- `📚` for `skill_view`;
- `📋` for `todo`;
- `✍️` for `write_file`; and
- `👁️` for `vision_analyze`.

The supplied progress payload starts with `📚 Reading skill`, so
`_infer_finitechat_kind` returns `message`. The shared UI's fallback regular
expression is also incomplete, but it is not consulted when the bridge has
already supplied the explicit kind `message`. `MessageRow` therefore displays
the entire progress transcript as raw prose instead of routing it through
`ToolRollup`.

This explains why the collapsible experience appears sometimes: it works when
the first progress line happens to use a recognized icon and fails when a
newer Hermes tool icon comes first.

### The resident service freezes its clock

`open_agent_runtime` in `finitechat/crates/finitechat-cli/src/hermes.rs` passes
`now_unix_seconds: Some(now_secs())`. `Some` is the deterministic fixed-clock
mode in `FiniteChatRuntime`; `None` is the live-clock mode.

CLI-per-command execution hid the problem because each invocation opened a
fresh runtime. The current resident service opens the runtime once in
`prepare_hermes_service` and retains it in `HermesServiceState`. Every send and
edit during that service lifetime therefore observes the same fixed time. The
dashboard merely prints the `display_timestamp` produced by Core; it is not
rewriting the time to `4:17 PM`.

## Recommended tight fix

Keep `SUPPORTS_MESSAGE_EDITING = False` so ordinary token streaming remains
disabled for the clients that motivated that compatibility setting. Make three
bounded changes:

1. **Use separate progress records for Finite Chat.** Reconcile the
   platform-specific Hermes setting
   `display.platforms.finitechat.tool_progress_grouping: separate`. Hermes will
   emit one durable record per tool instead of rewriting a bubble across later
   commentary. The shared UI already groups adjacent tool records into one
   collapsible block; an assistant commentary message naturally ends one group,
   and later tools start another below it.
2. **Classify the complete Hermes 0.18.2 tool vocabulary in the bridge.**
   Expand the adapter classifier to include every icon in the pinned Hermes
   registry, with tests for the four currently missed icons above. Because the
   bridge will then send the explicit `kind = "tool"`, the dashboard fallback
   does not need to change for the immediate fix. Its incomplete fallback can
   be expanded as defense-in-depth during a future dashboard change. Longer
   term, Hermes should send an explicit semantic progress marker in metadata so
   Finite Chat does not have to infer message type from decoration, but that is
   not required for this fix.
3. **Use the live clock in the resident bridge.** Open the resident
   `FiniteChatRuntime` with `now_unix_seconds: None`. Retain fixed timestamps
   only in deterministic tests that explicitly request them.

Likely production files:

- `finitechat/containers/agent/reconcile_hermes_config.py`
- `finitechat/integrations/hermes/finitechat/adapter.py`
- `finitechat/crates/finitechat-cli/src/hermes.rs`
- focused tests beside those components

The dashboard component itself should not need behavioral changes.

## Alternative fixes

### Patch or upgrade Hermes's reset behavior

Hermes can enqueue `__reset__` whenever the direct interim-commentary path
sends a content message, or make the progress sender honor
`SUPPORTS_MESSAGE_EDITING` consistently. That preserves accumulated editable
bubbles while fixing their segmentation. It is architecturally cleaner but is
a broader dependency change than a Finite Chat platform override and needs
revalidation across Telegram and other Hermes platforms.

### Reposition edits at their latest sequence in the dashboard

The UI could move an edited item to the edit event's newest `seq`. That would
move the growing tool bag downward, but it would still be one non-interspersed
bag, would make bubbles jump around during live work, and would change normal
message-edit semantics for every client. This treats the symptom and is not
recommended.

### Add protocol-level progress groups

The protocol already carries accepted sequence, `kind`, `status`, and
`edit_of`; no information is lost in delivery. Adding a new progress-event
protocol is unnecessary for this incident and would expand scope across all
clients. Do not do this for the immediate fix.

## Evaluation design

The fix is complete when all of these pass:

1. **Bridge classification:** table-driven adapter tests prove every tool icon
   emitted by pinned Hermes 0.18.2 becomes `kind = "tool"`; ordinary emoji-led
   prose remains `kind = "message"`.
2. **Linearization fixture:** feed `tool A`, commentary, and `tool B` through
   the configured Finite Chat progress path. The accepted transcript must be
   `ToolRollup(A)`, commentary, `ToolRollup(B)`, with monotonically increasing
   durable sequence.
3. **Collapse behavior:** adjacent tools without commentary become one
   collapsible block; a commentary message splits the blocks; the block is open
   while running and collapsed after completion.
4. **Resident timestamp test:** keep one resident service alive across two
   controlled clock instants and send two messages. Their raw and display
   timestamps must differ and remain ordered.
5. **Compatibility:** ordinary assistant token streaming remains disabled for
   Finite Chat, append-only edits still project correctly, and Electron/iOS
   parity tests remain green.
6. **Runtime canary:** on one disposable Agent, run a multi-minute task that
   uses `skill_view`, `todo`, file writes, terminal calls, vision, and interim
   commentary. Observe interspersed commentary and collapsible tool groups,
   then refresh the dashboard and confirm the persisted transcript is
   identical.

After the canary, publish one canonical Runtime image and use the existing
runtime rollout path. No Core, Sites, Finite Chat server, or dashboard deploy
should be necessary unless tests expose a separate regression.
