# Finite next work: ten bounded handoffs

Status: **PROPOSED PLANNING INDEX — NOT EXECUTION AUTHORITY**

Owner: Paul  
Opened: 2026-07-23

This is the central handoff document for the next ten workstreams. It combines
the three Finite Sites concerns into one plan. It is intentionally not an
ACTIVE run under [`runs/README.md`](runs/README.md): each implementation task
claims exactly one numbered section, and production mutations still require
their own explicit approval.

## Working agreement

- Paul owns priority, thread creation, merge, release, and production approval.
- Straightforward implementation tasks use **GPT-5.6 Sol, high reasoning**.
- A task starts from current `origin/main` in its own worktree and does not
  reuse a stale investigation branch.
- The task first restates its numbered scope and any unresolved **Paul gate**.
  It must stop before building if that gate is unanswered.
- Prefer an existing product boundary over a new platform feature. Deleting an
  unnecessary layer is a valid result.
- Every task ends with a small ready PR, focused evidence, explicit non-goals,
  and remaining live acceptance. It does not merge or deploy itself.
- Read-only production evidence is allowed when the task calls for it.
  Restart, migration, deployment, spend, data movement, or destructive work
  requires Paul's fresh explicit approval.

## Suggested order

This is not a serial dependency chain.

| Track | Work |
|---|---|
| Finish current | 1. Sites |
| Small independent product slices | 4. Audio, then 2. message queue |
| Measurement and operations | 8. Finite Private load; 5. lat1 go/no-go in its approved window |
| New capability canaries | 6. Phala; 3. Buzz |
| User continuity and product shape | 7. legacy migration; 9. Brain |
| Evidence-driven follow-up | 10. Rust performance |

---

## 1. Finite Sites: truthful publishing, automatic viewing, human sharing

### Outcome

A normal publish followed by a normal reload shows the newly active version.
An authenticated Finite human who asks an agent to initialize a Project can
view every declared output without the agent remembering a flag. Share emails
make immediate sense to humans and retain a compact agent handoff.

### Existing leverage and current state

- [PR #194](https://github.com/finitecomputer/finite-mono/pull/194) makes viewer
  and collaborator emails human-first. It has passed independent review and CI.
- [PR #195](https://github.com/finitecomputer/finite-mono/pull/195) makes
  mutable public output responses `no-store`, waits for Git reconciliation
  before a successful push returns, and hashes complete rendered Document
  representations for ETags.
- Production evidence from `agentcamp.finite.chat` and
  `laguna-s-2-1.finite.chat` established the cache cause: the origin sent
  `max-age=0`, while Cloudflare changed cacheable JS/CSS to a four-hour browser
  TTL. Fresh HTML could therefore load old assets. `no-store` survives the edge
  unchanged.
- [PR #196](https://github.com/finitecomputer/finite-mono/pull/196) is
  **not ready to merge**. Its first implementation used
  `HERMES_SESSION_ID`, which is absent on cached later turns. The corrected
  design must use the every-turn session key and an authenticated Finite-turn
  marker; Finite currently appears as Hermes `LOCAL`, so a platform-name check
  alone is insufficient.

### Bounded implementation

1. Land the email and cache/deploy PRs after normal review.
2. Repair or replace PR #196 without adding a Sites-to-Chat dependency:
   - Finite Chat authenticates the per-turn human.
   - A bounded local turn context reaches the `fsite` subprocess.
   - `fsite project init` infers that requester only while the authenticated
     turn is active.
   - The Agent Principal still signs Project Init; Sites gains no signer and
     knows nothing about Hermes, WorkOS, Dashboard, or Core.
   - Explicit standalone `--requesting-user-npub` remains supported. If an
     explicit value disagrees with a live authenticated context, fail closed.
3. Update the agent-facing help so the automatic behavior is discoverable and
   the old prompt-memory requirement disappears.

### Acceptance

- Publish v1, retain its response, publish v2, and request the same URL:
  response is v2 without a query parameter, private window, or hard refresh.
- Public HTML, JS, CSS, and arbitrary assets all arrive through the production
  edge with `Cache-Control: no-store`.
- A successful Git push returns only after every matching output has its new
  active Version. A failed deploy states that the Git ref was accepted and a
  correcting commit is required.
- Changing a sibling Document title invalidates the rendered page ETag because
  navigation changed.
- Dry-run and apply resolve and report the same authenticated requester; apply
  atomically shares every declared output with that human.
- A cached second Hermes turn works; a local shell, non-Finite turn, expired
  context, background process, restart, or mismatched sender does not infer a
  requester.
- Viewer and collaborator emails put human action first and preserve canonical
  URLs, `/llms.txt`, and edit commands.

### Must not

- Do not add manual cache-busting parameters to the publishing skill.
- Do not add a CDN purge API or a second publishing workflow.
- Do not make Chat, Sites, and WorkOS share a signer or authorization database.
- Do not broaden this into cross-output atomic page loads or a new asset
  fingerprinting build system.

### Paul gate before a subagent builds

**No remaining product clarification.** Paul already confirmed that every
declared output initialized during an authenticated human turn should be
shared back to that human, while existing Project ACLs remain explicit.

The implementation task must acknowledge that PR #196 is technically blocked
until the cached-turn/session-key problem is corrected; it must not merely make
the current test suite green.

### Later live approval

- Approve the component release and Sites deployment.
- Expect one transition caveat: browsers that already received the old
  four-hour asset TTL may need one hard reload or may retain that entry until
  it expires. Afterward, ordinary reloads remain fresh.

---

## 2. Finite Chat: minimal durable follow-up queuing

### Outcome

While one turn is running, a second ordinary text message in the same session
waits durably and becomes the next turn. A process restart before admission
does not lose it. Control inputs needed by the active turn remain immediate.

### Existing leverage

- The Rust Hermes inbox already persists events by room, sequence, and message
  and redelivers them until exact-event ACK.
- Hermes 0.18.2 has an in-memory busy-input queue, but enabling it alone is
  unsafe because the adapter currently ACKs a message as soon as Hermes accepts
  it.
- `codex/hermes-queue-ui-investigation` contains no implementation and is
  behind main. Do not build on it.

### Bounded implementation

1. Add an ephemeral per-session admission gate in the Finite Hermes adapter.
2. For an ordinary same-session text follow-up, leave the event unacknowledged
   in the existing Rust inbox until the current owner task has released the
   session.
3. Preserve inbox sequence order, then hand the event to Hermes and ACK it.
4. Let approvals, clarification replies, `/stop`, and existing steer/interrupt
   control paths bypass the gate.
5. Make UI pending state message/turn-specific only if the existing single
   pending marker can falsely clear the queued message.

### Acceptance

- Block turn A, submit B, and prove B is neither handed to Hermes nor ACKed.
- Kill and restart before A releases; B is the first next ordinary turn and is
  handed once.
- A live approval or `/stop` still reaches A while B waits.
- Separate sessions do not block one another.
- Duplicate stream delivery does not create duplicate turns or an unbounded
  in-memory queue.

### Must not

- No second durable queue, new database table, cancellation redesign, reorder
  UI, queue editor, or exactly-once tool-execution claim.
- Do not globally pause the inbound stream.

### Paul gate before a subagent builds

Confirm both:

1. V0 queues **ordinary same-session text only**; cancellation, reorder, and
   queue management are out.
2. The guarantee is **durable until Hermes begins the turn**, not exactly-once
   completion after tool execution has started.

Recommended answer to both: **yes**.

### Later live approval

Approve the Runtime image canary and one-Agent rollout after the real
restart/bypass smoke passes.

---

## 3. Buzz: external relay, bounded Finite connection

### Outcome

One Finite agent participates in an internal Buzz community through the normal
ACP boundary. Finite can connect to a Block-hosted or independently hosted
relay without owning Buzz's server model.

### Architecture boundary

- Buzz relay/server code and deployment remain upstream or in a separate
  deployment repository, just as Telegram clients and servers do not live in
  `finite-mono`.
- `finite-mono` owns only:
  - runtime integration for `buzz-acp` to talk to the already shipped
    `hermes-acp`;
  - a bounded Connections UI/record for relay URL and connection state; and
  - tests proving the adapter boundary.
- No Buzz tables in Core, WorkOS coupling, Runner provisioning, or Finite-owned
  multi-tenant Buzz service.

### Bounded implementation

1. Manually prove one internal community and one Agent identity:
   `Buzz relay ↔ buzz-acp ↔ hermes-acp`.
2. Prove text send/receive, channel scoping, distinct human and Agent Buzz
   identities, and restart continuity.
3. Only then add a minimal Buzz connection to the existing Connections UI:
   relay URL, connected/disconnected status, and the smallest invite/pairing
   action upstream requires.
4. Keep customer self-hosting identical: the customer supplies a relay URL;
   Finite does not provision or operate it.

### Acceptance

- The same adapter works against the selected internal community and an
  independently supplied relay URL.
- Agent and human use distinct Buzz keys.
- Restarting the Agent restores the connection from Agent-owned durable state.
- Removing the connection stops Buzz traffic without affecting Finite Chat.
- Buzz remains usable independently of Finite's platform.

### Must not

- No Buzz fork, vendored server, global Finite/Buzz identity, dashboard-hosted
  Buzz client, or generic external-service orchestration layer.
- Do not build Connections UI before the manual ACP proof.

### Paul gate before a subagent builds

Choose the internal canary:

1. **Block-hosted community (recommended)** — fastest and proves the actual
   product integration; or
2. independently hosted upstream Buzz — durable external service, but its
   deployment remains outside `finite-mono`.

Also confirm the V0 connection model: **one relay connection per Finite
organization, with the Agent's Buzz private key kept in Agent-owned `/data` and
never in Core**. If that is not the intended custody model, decide it before
the Connections UI is built.

### Later live approval

Approve creation of the external community/account and the Runtime image canary.

---

## 4. Web and Electron audio recording

### Outcome

A human records a short audio message in Web or Electron, sees it as a normal
pending attachment, and sends it through the existing attachment path. No
transcription is introduced.

### Existing leverage

- The Dashboard composer already uploads arbitrary files.
- Hosted Device and transcript rendering already accept and play common audio
  MIME types.
- Electron already declares microphone usage/entitlement, but its permission
  handler currently denies every request.

### Bounded implementation

1. Add a microphone button using `MediaRecorder`.
2. Tap once to start and once to stop; create a `File` and stage it through the
   existing attachment UI. The user still presses Send.
3. Negotiate a supported MIME type (`audio/webm` or `audio/mp4`) and reuse
   existing file-size/error handling.
4. In Electron, permit microphone media only for the exact trusted Dashboard
   origin and main frame. Keep camera and all untrusted origins denied.

### Acceptance

- Chrome/Safari-compatible Web recording stages, removes, sends, downloads,
  and plays like an uploaded audio file.
- Electron prompts for microphone access and records on the production
  Dashboard; camera remains denied.
- Permission tests cover trusted origin, subframe, wrong origin, camera, and
  combined media requests.
- Existing image/file attachments are unchanged.

### Must not

- No transcription, waveform editor, voice activity detection, local macOS
  speech API, Telegram-style hold gesture, or new audio storage path.

### Paul gate before a subagent builds

Confirm the interaction: **tap to start / tap to stop / normal Send
(recommended)** rather than press-and-hold or automatic send.

### Later live approval

Approve the next Electron alpha release after Web and packaged-Electron smoke.

---

## 5. lat1: RAID rebuild go/no-go and maintenance

### Outcome

lat1 is rebuilt onto an exact reviewed RAID layout without losing Agent,
Hosted Web Chat/Core, Sites, Brain, or secret-bootstrap state, and with a named
rollback boundary.

### Existing leverage and warning

- lat3 provides a proven exact-size RAID1, dual-ESP pattern to reuse.
- [`runs/finite-lat-capacity-and-redundancy.md`](runs/finite-lat-capacity-and-redundancy.md)
  already lists the remaining recovery and ordering gates.
- [`infra/runbooks/lat1-nixos-reinstall.md`](../infra/runbooks/lat1-nixos-reinstall.md)
  is historical and explicitly says not to repeat it. It is not wipe
  authority.

### Bounded sequence

1. Read-only inventory: by-id disks, sector geometry, partition tables, UUIDs,
   md state, NVMe health, mounted filesystems, free space, current generation,
   and writer/service inventory.
2. Close the complete Recovery Set and perform the missing synthetic restore
   proofs.
3. Reuse lat3's exact-size RAID and dual-ESP guards; build the closure on lat2.
4. Fix `/data` root fallthrough and Runner/Core startup ordering before the
   window.
5. Produce one go/no-go packet naming backup artifacts, hashes, restore steps,
   rollback target, expected downtime, and stop conditions.
6. In the separately approved evening window: stop writers, take final
   backups, rebuild, restore, verify, and keep new creation drained until all
   checks pass.

### Acceptance

- Every named Recovery Set restores on an empty target before the wipe.
- Root and `/data` are RAID1 with the reviewed geometry; both ESPs boot.
- Core, Hosted Web Chat, Sites, Brain, existing Agent lifecycle, Chat, and
  backups pass after restore.
- No service silently writes Agent data to root when `/data` is unavailable.

### Must not

- No speculative repair, inferred disk selection, reuse of the historical
  reinstall transcript, or wipe based only on one representative Agent
  restore.
- No unrelated platform upgrade in the maintenance window.

### Paul gate before a subagent starts

This plan is deliberately parked until Paul says **“start the lat1 go/no-go”**
in the afternoon. That authorizes read-only evidence collection and preparation
only.

### Required destructive approval

After reviewing the completed go/no-go packet, Paul must separately approve the
exact disks, closure, backup set, rollback target, maintenance window, and
destructive reinstall. Silence or an earlier general instruction is not
approval.

---

## 6. Phala as a Runner

### Outcome

One internal Agent runs on Phala through the existing provider-neutral Runner
contract, survives restart, and can be recovered onto a replacement target.
Pricing and onboarding follow only after the infrastructure proof.

### Existing leverage

- Typed Phala API/inventory code, a drained hard-cap-one Nix worker, read-only
  preflight, and a runbook already exist.
- Runtime retirement and empty-target recovery close an important prerequisite.
- Two technical blockers remain: reviewed environment
  encryption/signature/test vectors and a typed Core in-flight reservation
  count/ack that can be combined with provider inventory.

### Bounded implementation

1. Run the existing read-only preflight without provider spend.
2. Close only the two named blockers; do not generalize provider orchestration.
3. Activate one internal launch-code canary with hard cap one and the canonical
   Agent Runtime image.
4. Prove create, Chat, Sites, restart, stuck/failed detection, and recovery to a
   replacement CVM.
5. Use the same canonical image with a recovery boot intent or previous
   known-good digest. Add a distinct rescue image only if this proof shows a
   concrete capability the canonical image cannot provide.
6. After acceptance, separately plan Stripe tier, price, copy, and onboarding.

### Acceptance

- Core and provider inventory agree before and after every operation.
- A lost worker process cannot create beyond cap one.
- The Agent Principal and restored `/data` survive replacement.
- Recovery requires no arbitrary SSH shell or provider-specific state in Core.
- Stop/destroy never implies data purge.

### Must not

- No Stripe/onboarding work before the one-CVM canary.
- No second rescue artifact by default, provider selector UI, arbitrary remote
  shell, or rewrite of the Runner contract.

### Paul gate before a subagent builds

Confirm the recovery decision: **reuse the canonical Runtime image in recovery
mode (recommended)** unless the canary proves a separate image is necessary.

The code/preflight task may proceed without choosing pricing. Before a live
CVM is created, Paul must name the canary Agent and approve provider spend,
create/restart/destroy authority, and the hard cap.

### Later product gate

After the canary, Paul chooses the customer-facing tier name, included capacity,
price, and whether Phala is visible or simply the implementation of that tier.

---

## 7. Legacy-to-new-platform migration

### Outcome

A verified legacy user signs in at `finite.computer`, sees and talks to the same
legacy Agent through the new platform, and can later move that Agent and its
Sites without data loss. The legacy source remains the rollback authority until
acceptance.

### Existing leverage

- [`finitecomputer-v2/docs/existing-user-import-bridge.md`](../finitecomputer-v2/docs/existing-user-import-bridge.md)
  already describes reconciliation by legacy host/machine, verified-email
  claim, a v2 Project/runtime link, and source-host relay without moving
  compute.
- The backend bridge remains even though imported Projects are currently
  hidden from the Dashboard.
- The legacy `core-import-manifest` producer is absent from the current legacy
  default branch even though it existed historically. Before account work,
  inspect the deployed `finited` version and help output read-only. If the
  command is absent there too, recover the smallest historical producer change
  instead of redesigning migration.
- `paul-finite-2` is the proposed first real canary.

### Bounded phases

1. **Account continuity first:** inspect the deployed legacy producer/version,
   reconcile one verified user and Agent, expose the imported Project, and keep
   the Agent running on its legacy source through the existing relay.
2. **Synthetic relocation proof:** inventory exact paths and state, freeze a
   synthetic source, export a versioned bounded archive/manifest, restore on an
   empty new Runtime, and compare identity plus checksums.
3. **`paul-finite-2` relocation canary:** final backup, source write fence,
   restore, two-way Chat and data checks, then retain the source rollback copy.
4. **Sites separately:** inventory old Sites, publish verified replacements,
   and install redirects only after new URLs pass.
5. Telegram pairing preservation is a later optimization, not the minimum bar.

### Acceptance

- No data loss by archive manifest and checksum.
- New-platform account and Agent association are deterministic from verified
  identity, not selected UI state.
- Agent Principal and durable files survive relocation; unexpected path
  differences are handled by one documented compatibility mapping if needed.
- Rollback to the frozen legacy source remains possible until Paul accepts the
  new Runtime.
- Old Site redirects point only to verified new publications.

### Must not

- Do not send a large migration archive through Chat.
- Do not move data in the first account-continuity slice.
- No fleet-wide migration, Telegram re-pair project, generalized path
  virtualization, or deletion of the legacy source in the canary.

### Paul gate before a subagent builds

Confirm the first deliverable is **account/dashboard continuity while compute
stays on legacy (recommended)**, not immediate relocation.

Also confirm verified email is the intended human claim boundary for the
canary. If another account mapping is required, decide it before restoring the
hidden imported-Project UI.

### Required live approvals

Separate approvals are required for:

1. read-only inventory of `paul-finite-2`;
2. account reconciliation;
3. source write freeze and archive;
4. restore/cutover; and
5. any legacy Site redirect.

---

## 8. Finite Private: five-minute capacity signal

### Outcome

Produce the first recorded answer to whether Finite Private can sustain
20–40 simultaneous short Chat inference requests for five minutes with
acceptable first-byte latency, completion latency, errors, and accounting.

### Existing leverage and current truth

- `infra/runbooks/finite-private-ops.sh load-canary` already performs 32
  concurrent short authenticated calls and enforces p99 first byte below
  90 seconds.
- The exact scripted 32-request canary has **no recorded execution**. It was
  added on 2026-07-21 for an explicitly unapproved limiter cutover; the
  associated satellite has not changed since 2026-07-02.
- No new load framework is needed.

### Bounded implementation

1. Parameterize or wrap the existing canary for concurrency and a five-minute
   constant-load duration without replacing its request/accounting checks.
2. Use one dedicated synthetic grant and unique request IDs.
3. Ramp 20, then 32, then 40 only if the prior rung stays healthy.
4. Record HTTP errors, TTFT p50/p95/p99, total latency, throughput, limiter
   readiness, GPU/model health, settlement latency, and reservations left
   `reserved`.
5. Start with the direct Finite Private API. Add an end-to-end Agent/Chat load
   only if the direct result passes and the remaining question is demonstrably
   outside inference.

### Acceptance

- Five continuous minutes at each accepted rung.
- No authorization bypass, stuck reservation, or silent failed response.
- Results are preserved in a redacted report with exact revision, endpoint
  class, model/release, concurrency, duration, and stop reason.
- The report distinguishes capacity, limiter behavior, and upstream model
  latency rather than collapsing them into one number.

### Must not

- No k6 stack, general load-testing service, production user key, unbounded
  prompt, or immediate platform rewrite based on one run.

### Paul gate before a subagent builds

Confirm the first question is **direct Finite Private inference capacity
(recommended)**. If the intended question is full end-to-end Finite Chat
capacity, say so before the script is changed because that is a different
topology and test.

### Required live approval

Paul must approve the production test window, synthetic grant/quota spend,
20/32/40 ramp, and stop thresholds before any requests run.

---

## 9. Brain CLI and cross-product identity shape

### Outcome

Ship a current Brain CLI to Agents and prove that Brain, Sites, and Chat share
the same bounded identity pattern without being coupled into one platform
service.

### Existing leverage

- `fbrain/v0.1.3` is behind current main.
- [`adr/0004-products-own-bounded-identity-adapters.md`](adr/0004-products-own-bounded-identity-adapters.md),
  [`finite-brain` ADR 0020](../finite-brain/docs/adr/0020-keep-personal-vaults-user-owned-and-grant-agents-folder-scoped-access.md),
  and [`finite-sites` ADR 0023](../finite-sites/docs/adr/0023-shared-finite-identity.md)
  already define the intended shape.
- The goal is proof and release, not a new identity architecture.

### Required Austin sync before code

Paul asks Austin:

1. What exact Brain commit is the CLI release candidate?
2. Which user-visible capability is actually blocked today?
3. What complexity was added because platform basics were missing, and which
   part can now be deleted?
4. What release/test remains before Agents can use current Brain?
5. Does any current design require Brain to trust Core, Chat, or Sites
   authorization rather than its own explicit grants?

### Bounded implementation after the sync

1. Select and release the accepted current CLI.
2. Add one small conformance document/test asserting:
   - human and Agent identities are distinct;
   - sharing is explicit and product-native;
   - the standalone CLI works without Core/Dashboard;
   - platform code may automate bootstrap/custody but supplies no global signer;
   - authorization in one product grants nothing in another.
3. Remove or defer any layer whose only purpose was an assumed shared platform
   identity.

### Acceptance

- A normal Agent installs/uses the released Brain CLI at the pinned version.
- Human-owned vault/folder data remains human-owned; the Agent receives only
  explicit folder-scoped capability.
- An agent outside Finite can use Brain with its native protocol.
- Chat, Sites, and Brain tests demonstrate the same shape without shared
  authorization state.

### Must not

- No global identity service, shared product ACL, Dashboard Brain client,
  hosted-Brain rollout, or speculative rewrite in this slice.

### Paul gate before a subagent builds

The Austin conversation is the gate. Paul supplies its notes plus the selected
release candidate and confirms **CLI release + conformance proof only
(recommended)**.

### Later live approval

Approve the `fbrain/vX.Y.Z` tag and Runtime skill/package update.

---

## 10. Rust hot-path performance audit

### Outcome

Find and rank concrete CPU, RAM, I/O, lock, query, and algorithmic risks in the
Rust paths that current load evidence identifies, with reproductions and small
fix recommendations.

### Existing leverage

- [`finitechat/docs/perf-audit.md`](../finitechat/docs/perf-audit.md) and
  [`finitechat/docs/perf-log.md`](../finitechat/docs/perf-log.md) already found
  and fixed full-state serialization and whole-server cloning. Do not repeat
  that audit as if it were new.
- The Finite Private load result should select the first target.

### Bounded implementation

1. Read the load report and choose at most two hot paths, likely among:
   limiter reserve/settle, Core Postgres/store operations, Hosted Device state
   serialization, or locks/blocking I/O around request handling.
2. Establish a baseline with an existing benchmark or one narrow reproducible
   harness.
3. Inspect for:
   - O(users), O(messages), or O(state) cloning/serialization per request;
   - N+1 queries or missing bounded pagination;
   - blocking disk/network work while holding locks;
   - unbounded queues, retained buffers, and accidental task fan-out;
   - repeated parsing/allocation on the hottest path.
4. Produce a ranked audit. A tiny obvious correction may receive its own PR;
   larger fixes get separate plans.

### Acceptance

- Every finding names the path, trigger, asymptotic or measured cost, evidence,
  user impact, and smallest corrective boundary.
- At least one baseline is repeatable before and after any proposed fix.
- “No material issue found” is an acceptable result.
- The audit distinguishes measured problems from code-reading hypotheses.

### Must not

- No broad Rust rewrite, new observability platform, database replacement,
  async-runtime migration, or bundling unrelated fixes into the audit.

### Paul gate before a subagent builds

Choose one:

1. **Wait for the Finite Private load result, then audit the observed hot path
   (recommended)**; or
2. start a one-day static audit now, limited to two named crates.

Also decide whether the task is report-only or may open separate PRs for
obvious low-risk fixes. Recommended: report first, then one PR per accepted
fix.

### Required live approval

Any production profiler, trace capture, or load-generating benchmark needs a
separate approved window. Read-only code and existing-test analysis do not.

---

## Thread handoff template

When spawning a task, Paul can use:

> Implement plan **N** from
> `docs/next-work-plans-2026-07-23.md`. Use GPT-5.6 Sol with high reasoning.
> First restate the bounded outcome, answer or stop on the Paul gate, and name
> your files. Work from current `origin/main` in a dedicated worktree. Do not
> add adjacent features. Finish with focused evidence and a ready PR; do not
> merge, release, deploy, spend, or mutate production.
