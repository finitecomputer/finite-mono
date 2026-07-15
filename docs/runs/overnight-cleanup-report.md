# Overnight Cleanup Report

Status: COMPLETE WITH BLOCKERS

Run: [`overnight-cleanup.md`](overnight-cleanup.md)

Branch: `overnight-cleanup-2026-07-13`

Base: `c18bfd88a889802fe42ed4365e301f99089b53a9` (`main`, rebased 2026-07-14)

## 1. Stale chat pane on switch — SHIPPED

What changed: The dashboard now follows a rejected selection-only mutation
response with a fresh HTTP state snapshot. The reconciliation snapshot may
replace an unchanged equal-revision state, while the existing generation and
mutation-sequence checks still prevent stale responses from rolling state
back. The browser fixture now matches the daemon's selection-only `OpenChat`
revision behavior and covers a stream update landing while `OpenChat` is in
flight.

Verification evidence:

- Dashboard unit tests: 183 passed.
- Dashboard lint: passed.
- Dashboard browser suite: 2 passed, including the forced `OpenChat`/SSE race.

Two-minute morning verification:

1. In the local stack browser, open an Agent with at least two Chats under one
   Topic.
2. Send or receive activity so the stream is live, then switch rapidly between
   the two Chats.
3. Confirm the highlighted Chat, header, and transcript all change to the last
   clicked Chat without another click or message.

Paul action: none beyond the morning browser verification.

## 2. Disaster recovery drill — BLOCKED

What ran locally: A disposable synthetic Recovery Set containing 17 opaque
Account, Device, Room, Topic, Chat, message, attachment, Project, Runtime, and
Agent identifiers was built from two valid SQLite databases and a Postgres 16
custom-format dump. `restore-hosted-web-chat-snapshot` installed the verified
set atomically onto an empty target in 306 ms. The installed databases and dump
were readable and their manifest reverified. The source remained unchanged.

Negative evidence: six attempts were refused before target mutation: missing
isolated-mode acknowledgement, unsupported format, hash-modified artifact,
missing database, corrupt SQLite with a recomputed manifest, and non-empty
target. The non-empty target's sentinel remained intact.

Read-only remote evidence on 2026-07-14 UTC:

- Both production snapshot/off-site timers were active; both health services'
  latest results were successful.
- The latest service-consistent snapshot was 293 seconds old and its manifest
  verified.
- The recovery credential directory was mode `0700`; its three named files
  were root-owned mode `0600`.
- The off-site success stamp was 764 seconds old.
- The three newest Borg archives listed successfully. The selected candidate
  is `finite-lat-1-hosted-web-chat-2026-07-14T02:37:00`; an archives-only Borg
  check of the newest archive passed.

Why blocked: No empty external isolated target was named or proven to have
public ingress, email, webhook, push, billing, and other side effects disabled.
There is also no observed fence proving the retained Agent Runtime cannot
contact both source and restored stacks. Extracting the production archive on
finite-lat-1 would write to the live production host; extracting it onto this
Mac would handle the complete production Recovery Set outside a named isolated
target. Both violate the drill boundary. Wrong-key and truncated-Borg-archive
proofs therefore remain unrun, as do service startup, identifier comparison,
attachment decryption, Runtime reconnect, and a fresh Agent turn.

Two-minute morning verification:

1. Review this entry and confirm the selected archive and timings are
   acceptable as preflight evidence only.
2. Confirm that no restore target or traffic switch was created by this run.

Paul action: Name/provision the empty isolated target, confirm its outbound
side-effect fence, name the retained Runtime and its source/restored-stack
network fence, and confirm independent Borg passphrase/key-export custody.
Then the remote drill can resume without redoing the local verifier work.

## 3. Runtime image preinstalls and launch-time audit — SHIPPED

What changed: The canonical runtime image now installs the official `gws`
0.22.5 binary for both amd64 and arm64. The build selects the architecture,
verifies its pinned SHA-256 digest before extraction, and proves the installed
version during the build. The preinstall and launch findings are recorded in
[`preinstall-audit-2026-07-13.md`](../audits/preinstall-audit-2026-07-13.md)
and [`launch-time-audit-2026-07-13.md`](../audits/launch-time-audit-2026-07-13.md).

Verification evidence:

- The canonical Apple Container arm64 build passed; an ephemeral container
  reports `gws 0.22.5`.
- Runtime image contract tests: 7 passed.
- Same-source compressed arm64 size: 211,616,285 bytes before and 218,447,047
  after, an increase of 6,830,762 bytes (3.23%).
- Matched cached lower-bound launch probes improved from a noisy 1.05-second
  median to 0.74 seconds. This is not evidence of a launch improvement; it is
  evidence that the added binary caused no material cached-start regression.
- Existing production journal evidence shows one successful cached launch in
  about 3.45 seconds, but lacks phase markers. No package install or build
  occurs in the launch script.

Two-minute morning verification:

1. Build or use the branch's local runtime image.
2. Run `gws --version` in an ephemeral runtime container and confirm 0.22.5.
3. Ask a test Agent to check `gws auth status`; confirm it finds the binary and
   does not attempt a download. Do not authorize a production credential for
   this check.

Paul action: Decide separately whether the 6.8 MB compressed increase is worth
keeping. The final full SaaS smoke passed with this image content.

## 4. Google scopes per PR #34 — SHIPPED

What changed: The dashboard OAuth constant, installed dashboard contract, and
managed-skill contract now request Google Docs read/write and no longer request
Apps Script project or deployment scopes. The skill states the boundary
explicitly: approved Docs writes can use `gws`, while its current Python Docs
wrapper is read-only and has no Apps Script action. No live Google setting or
credential was touched.

Verification evidence: All 183 dashboard unit tests passed, including the test
that requires the dashboard constant and managed-skill JSON to remain identical.

Two-minute morning verification:

1. Inspect a branch OAuth URL and confirm it contains
   `https://www.googleapis.com/auth/documents`.
2. Confirm it contains neither `script.projects`, `script.deployments`, nor
   `documents.readonly`.
3. Do not complete the flow with a production account until the console-side
   scope configuration is reconciled.

Paul action:

1. In the Google Cloud OAuth consent screen's Data Access configuration, remove
   the Apps Script project and deployment scopes and add the Google Docs
   read/write scope (`.../auth/documents`). Leave the other current scopes as
   listed in the branch contract.
2. Expect the next dashboard Connect flow to show consent again: the route
   deliberately uses `prompt=select_account consent` and does not include prior
   grants.
3. Every existing authorized Google Workspace connection must disconnect and
   reconnect after the console change. Existing refresh tokens do not acquire
   the new Docs write grant merely because the repository list changed; this is
   also how the obsolete Apps Script grants are removed from the active token.

## 5. Honest working indicator — SHIPPED

What changed: Client-side Agent-turn and activity state are now 15-second
leases, matching the Finite Chat adapter's activity TTL. A pending turn carries
its local start time. It stops presenting as “working” when its lease expires
without a final response, and both pending and server-projected activity fail
closed whenever the update stream is disconnected. Fresh stream snapshots
renew live activity. No protocol field or daemon revision behavior changed.

Verification evidence:

- Dashboard unit tests: 183 passed, including exact lease-boundary and
  disconnected-stream cases.
- Dashboard lint: passed.
- The real-browser fixture passed after sending a fresh turn, removing the
  synthetic server activity without delivering a final response, and requiring
  “is working” to disappear at the lease boundary.

Two-minute morning verification:

1. Send a harmless request to a local test Agent and confirm “is working”
   appears while activity refreshes.
2. Stop or disconnect the local test stream and confirm the label disappears
   rather than remaining latched.
3. Reconnect, send another request, and confirm a final Agent response clears
   the label normally.

Paul action: none beyond the morning local-stack browser verification.

## 6. Skills audit — SHIPPED

What changed: The dashboard catalog now parses valid folded/literal YAML
descriptions instead of showing `>`/`|` or an empty description. Google
Workspace, meme, FAL editing, and X search skill text was reconciled with the
hosted runtime where the evidence was exact. Full findings are in
[`skills-audit-2026-07-13.md`](../audits/skills-audit-2026-07-13.md).

Verification evidence: The source inventory found 47 skills and exactly two
block-scalar descriptions. All 47 skills passed the static checks; all 183
dashboard unit tests and dashboard lint passed. Runtime probes confirmed Pillow
and `gws` present, and `xai-sdk` and `fal-client` absent.

Two-minute morning verification:

1. Open the branch dashboard Skills page and search for `tufte-viz-finite` and
   `llm-wiki-finite`.
2. Confirm each card shows its real multi-line description rather than `|`,
   `>`, blank text, or “No description yet.”
3. Inspect Google Workspace and confirm its card says the pinned `gws` CLI is
   available.

Paul action: Loudly: production still falls back to the archived
`finitecomputer/finite-skills` GitHub repository because infra does not set
`FC_FINITE_SKILLS_SOURCE_DIR`. The correct mono tree is baked into the runtime,
but the dashboard source contract is not wired to it. Keep the existing
parking-lot item and schedule the infra/image-layout reconciliation separately.

## 7. AEON image detection — AUDIT-ONLY

Observed path: Image handling is automatic before the Hermes message handler.
The adapter removes supported image media from the ordinary attachment list and
splices either AEON's normalized text or an explicit
`capability_unavailable` result into the event text. Agentd accepts an AEON
vision offer only after configuration read-back, a Hermes restart, and the
fixed red-square probe returning exactly `RED`. Image can be enabled while
audio/video stay disabled.

Verification evidence:

- Sixteen focused tests passed inside the built runtime: exact-red probe,
  wrong-result refusal, configured image specialization, unconfigured fallback,
  mixed media, retries/deadlines, and caption/ack behavior.
- Five focused `finite-agentd` reconciliation tests passed, including exact
  rollback and stale-semantics refusal.
- An ephemeral branch runtime with no `auxiliary.vision` configuration returned
  status 1 and only the sanitized marker
  `{"success":false,"analysis":null}`. It did not pretend to see the image.

Why audit-only: The AEON worker is deployed on clawland, not in local
devfinity. The protected local inference credential was sufficient for the
final SaaS smoke, but it does not provide an AEON worker or an existing vision
offer. Standing up a local GPU worker or changing an offer would be CREEP.

Two-minute morning verification:

1. In an already-configured disposable canary Agent, attach a known image (for
   example, a red square) and ask for its dominant color without naming it.
2. Confirm the Agent answers from the image and that the attachment required no
   manual tool selection or skill prompt.
3. Read the canary logs and confirm the specialization result names the image
   capability/model and a request ID; do not copy credentials or image bytes.

Paul action: Name a disposable canary Agent whose existing offer already has
`auxiliary.vision` image capability enabled and whose reconciliation probe is
healthy, then run the three steps above. If no such Agent exists, applying an
offer is a production mutation and needs a separately authorized test. Do not
use Sol 2 or customer state for this proof.

## 8. FAL generation and media display — SHIPPED

What changed: Chat attachment cards now render `Video`/`video/*` media with a
native video player and `VoiceNote`/`audio/*` media with a native audio player.
Both retain the authenticated attachment proxy, filename, download, and share
controls. Other files still use the file card.

Actual dispatch behavior: A generic “draw me …” request reaches Hermes'
standard `image_generate` tool. Hermes 0.18.2 ships registered image providers
for FAL, Krea, OpenAI, OpenAI Codex, OpenRouter, and xAI; explicit
`image_gen.provider` configuration wins, otherwise the registry chooses an
available single provider or its legacy FAL fallback. Therefore “default” is
not intrinsically FAL and is not guaranteed to be OpenAI/xAI. The FAL image
editing and music skills are separate direct `fal_client` workflows.

Verification evidence:

- All 183 dashboard unit tests and lint passed.
- The real-browser fixture passed with proxied image, video, and audio
  attachments and asserted visible native controls for the latter two.
- The built Hermes image contains `image_generate` in the standard toolset and
  all six provider plugins named above.
- Live generation was not attempted: `FAL_KEY` and `FAL_API_KEY` are unset,
  and `fal_client` is absent. Installing it into the image is excluded by this
  run's dependency fence.

Two-minute morning verification:

1. With a disposable local Agent and configured test credentials, ask “draw me
   a red square”; confirm the resulting image appears inline and record which
   configured provider handled it.
2. Ask the FAL editing skill to make a harmless edit to that image; confirm the
   edited image appears inline.
3. Attach known tiny MP4 and MP3 fixtures and confirm each has inline playback,
   plus working download controls.

Paul action: Supply test-only local inference/FAL credentials through the
existing devfinity environment, then run the morning verification. Do not use
or rotate production credentials for this UI proof. If FAL editing is expected
to work without a task-time install, schedule the separately pinned
`fal-client` image addition identified in the preinstall audit.

## 9. Telegram audit — SHIPPED

What changed: Both runtime image definitions now install the pinned
`hermes-agent[messaging]` 0.18.2 extra instead of bare `hermes-agent`. This
supplies Hermes' Telegram, Discord, and Slack gateway dependencies at image
build time. The runtime-image contract now rejects either Dockerfile if the
messaging extra is dropped. No Telegram token, bot, config offer, or live Agent
was created.

Configuration boundary: mono configures Telegram through agentd config offers,
not the legacy environment-variable path. Only `enabled`, `token`,
`home_channel`, `reply_to_mode`, `gateway_restart_notification`,
`typing_indicator`, and `extra` pass the current allowlist. Any Hermes setting
outside that set must be placed under the reviewed `extra` map or handled by a
separately reviewed config change.

Verification evidence:

- The canonical Apple Container arm64 build passed in 82.5 seconds.
- An ephemeral container imported `telegram` and
  `telegram.ext.Application`; the installed version reported 22.6. The
  same-source pre-change runtime reported the module absent.
- Runtime image contract tests: 7 passed.
- The focused `finite-agentd` Telegram config test passed.
- Compressed arm64 size increased from 218,447,047 to 231,225,520 bytes:
  12,778,473 bytes (5.85%) for the complete Hermes messaging extra.

Two-minute morning verification:

1. In BotFather, run `/newbot` and create a disposable test bot.
2. Open the disposable Agent's **Connections**, paste that token, and click
   **Connect**.
3. Message the bot in Telegram and copy the eight-character pairing code.
4. Back in **Connections**, click **Refresh**, enter the code, and click
   **Approve**.
5. Click **Use this chat** for the approved person.
6. Send one message in each direction; confirm typing/final delivery and the
   selected reply-to mode.
7. Disconnect the connection and delete the disposable bot in BotFather.

Paul action: Run the disposable-bot verification after the branch image is in
a local stack. Do not reuse a production bot token. Decide whether the 12.8 MB
compressed image increase is acceptable for preinstalled messaging support.

## 10. Previous Conversations and wrong-room replies — SHIPPED

Cause: Fresh binding creation is already fail-closed: it accepts exactly the
one intended profile-chat Room and persists an empty `associated_room_ids`.
The regression was in dashboard presentation. A retained associated Room was
rendered under **Previous conversations**, and the chat view treated every
associated Room as an allowed current selection and message target. A persisted
legacy selection could therefore keep the UI and outgoing replies in that old
Room.

What changed: The sidebar now presents only Topics and Chats from the binding's
canonical Room. The chat view likewise accepts only that canonical Room as a
current selection and send target. Associated Rooms remain untouched in the
binding and state projection for recovery evidence; they are no longer
interactive current conversations. No Sol 2 state, production data, binding,
protocol, schema, or daemon behavior was changed.

Verification evidence:

- The real-browser fixture synthesized an associated legacy Room, its Topic,
  Chat, transcript, and a persisted selection pointing into it. The dashboard
  rendered neither the **Previous conversations** heading nor legacy Topic,
  fell back to the current canonical Chat, hid the legacy transcript, and sent
  the next reply to the canonical Room/Topic/Chat. Both browser tests passed.
- All 183 dashboard unit tests, lint, and the production Next.js build passed.
- The focused hosted-device test proved a newly created binding starts with no
  associated Rooms, retains one canonical Room across duplicate selection and
  restart, refuses a legacy Room as a new-chat target, and reopens canonical.

Two-minute morning verification:

1. Create a disposable Agent in the branch stack and open Chat. Confirm the
   sidebar contains only its current Topics and Chats and has no **Previous
   conversations** section.
2. Select two current Chats in turn, send a uniquely worded message in the
   second, and confirm it appears in that selected Chat.
3. Run `npm --prefix finitecomputer-v2/apps/dashboard run test:browser` and
   confirm the associated-Room regression case remains green.

Paul action: After this branch is deployed, Sol 2 may be removed through the
normal product deletion path if it has no data worth retaining. Do not manually
rewrite its binding or associated Rooms. Any attempt to preserve or repair Sol
2 first needs a read-only binding snapshot plus a named backup and rollback
boundary as a separate authorized recovery task.

2026-07-15 clarification: the removal UI and retained-data offboarding
transition landed, but Runtime Retirement remains intentionally unadvertised
and is rejected by Core policy. There is still no supported deletion path for
Sol 2. The production-baseline audit left it untouched rather than bypassing
that capability gate.

## Final gates

Passed:

- Dashboard `web-check`: 183 unit tests, lint, TypeScript, and production build.
- Dashboard real-browser suite: 2 passed.
- Rust formatting check; focused Core, Runner, devfinity, and hosted-device
  tests; and the canonical-binding test.
- Managed-skills static checks: 47 skills.
- Runtime-image contract: checker plus 7 tests.
- Canonical arm64 runtime image build, `gws 0.22.5` proof, and Telegram 22.6
  import proof.
- The services-only integration smoke passed from an equivalent short state
  root. The worktree's long path exceeds macOS's Unix-domain socket path limit,
  so the unmodified default state path cannot bind its process-compose socket.
- The full real SaaS smoke passed from an isolated short state root and Apple
  container namespace without stopping or mutating the existing Paul Local 1
  runtime. It proved dashboard form onboarding, trusted Core-side local Apple
  placement while production remains Kata by default, digest-pinned artifact
  registration and local digest verification, a healthy Hermes runtime, three
  real model replies through the Hosted Web Device, Finite Chat and Hosted Web
  Device restart recovery, Apple runtime restart, and stable Agent Principal.

Remaining blockers outside the merge gate:

- The production disaster-recovery restore drill still needs the named empty
  isolated target, side-effect fence, retained-Runtime network fence, and
  independent recovery-key custody described in item 2.
- The Google OAuth console scope update and user re-consent described in item 4
  remain manual deployment work; repository code cannot perform them safely.

Paul action for final gates: none. Keep the recovery drill and Google OAuth
console work as explicit post-merge operational follow-ups rather than claiming
they were completed by this code run.
