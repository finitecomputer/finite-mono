# Overnight Cleanup

Status: COMPLETE WITH BLOCKERS — see `overnight-cleanup-report.md`

Owner: Paul

Opened: 2026-07-13

Expires: 2026-07-16

Sequence note: Paul queued these ten items on 2026-07-13 evening for an
unattended overnight session, with Stripe Production Activation remaining the
priority run for daytime work. Item 2 executes the drill defined by
[hosted-web-chat-disaster-recovery.md](hosted-web-chat-disaster-recovery.md);
Paul's queue message is the authorization for its local half and for the
remote half **only within the fences below**.

Acceptance: Paul, next morning, reads one report at
`docs/runs/overnight-cleanup-report.md`, reviews one draft PR whose commits
map one-to-one to shipped items, and executes each shipped item's listed
2-minute verification in the local stack browser. Items not shipped appear in
the report as BLOCKED or CREEP with enough context to act on without
re-investigation. `main` has no new commits from this run.

## Hard fences (violating any of these ends the session)

1. **`main` is untouched.** All work happens on one branch,
   `overnight-cleanup-2026-07-13`, one commit per item, prefixed
   `cleanup(N):`. Open a single **draft** PR at the end. No merges, no
   deploy pins, no image promotions, no compat-matrix edits, no pushes to
   any branch that auto-deploys.
2. **No production mutation.** No prod service restarts, config changes,
   secret changes, DNS/Caddy changes, or traffic switches. The one remote
   exception is inside item 2's own fence.
3. **No protocol or schema changes.** Finite Chat proto, agentd config
   offers, Core APIs, and daemon rev semantics are frozen. If the right fix
   appears to live there, that is a CREEP note, not work.
4. **No new dependencies** except the two named in item 3/9 (the pinned
   `gws` binary; the `hermes-agent[messaging]` extra), both build-time image
   changes.
5. **Timebox: 90 minutes per item**, one revisit pass at the end for
   anything close to done. When the box expires: revert that item's partial
   work (`git reset` the item commit off the branch), write the report
   entry, move on. Never leave half-applied work on the branch.
6. **Local verification is the bar.** Every shipped UI item is verified in
   a real browser against the local Apple Container stack (`just dev up`,
   inference key from `FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY`). Image
   changes are verified by a local devfinity build + `just dev saas-smoke`.
   Before the PR: `just dev smoke` and `just dev saas-smoke` green, plus
   dashboard lint/unit and Rust fmt/clippy on touched crates.
7. **Report as you go.** Update `docs/runs/overnight-cleanup-report.md`
   after every item with: status (SHIPPED / BLOCKED / CREEP / AUDIT-ONLY),
   what changed, the 2-minute morning verification steps, and anything Paul
   must do. New audits go to `docs/audits/<name>-2026-07-13.md`, Status:
   PROPOSED. Discoveries outside the queue go to `parking-lot.md`, one line.

## Queue (work top-down; recon pointers were verified 2026-07-13 evening)

1. **Stale chat pane on switch.** Repro: two or three Chats across multiple
   Topics; switch Chats; the pane keeps the previous Chat until interaction.
   Recon diagnosis: `OpenChat` is selection-only so the daemon does not bump
   `rev`; the mutation response is the only carrier of the new selection,
   and `shouldApplyMutationHostedChatSnapshot`
   (`finitecomputer-v2/apps/dashboard/src/lib/hosted-web-chat-snapshots.ts:66-83`)
   silently drops it whenever `highestRev` advanced mid-flight (stream
   heartbeat, MarkRoomRead); no stream fallback exists, so the client keeps
   the old `selected_chat_id` until the user does something that bumps
   `rev`. Fix client-side only — e.g. on a rejected selection snapshot,
   refetch the HTTP snapshot to reconcile (request capture at
   `hosted-chat-provider.tsx:186-201`). Changing daemon rev semantics is
   CREEP (fence 3).
2. **Disaster recovery drill.** Execute the local drill per
   `docs/runs/hosted-web-chat-disaster-recovery.md` first. The remote half
   may proceed overnight ONLY if every step satisfies: snapshot creation
   uses the already-accepted off-host job; restore lands on an empty
   isolated target; zero writes to live services; no traffic switch. The
   moment a step would mutate live prod state, stop that step, document
   exactly what needs Paul, and leave the remote half BLOCKED. Record
   observed restore time and any gaps in the report.
3. **Runtime image preinstalls + launch-time audit.** The mono image lacks
   the `gws` binary legacy shipped (legacy: `googleworkspace/cli` 0.22.5
   via `finitecomputer/nix/agent-runtime/gws.nix`; mono
   `runtime.Dockerfile` has only the Python Google libs at lines 44-49),
   while the bundled skill says to prefer `gws`
   (`finite-skills/skills/productivity/google-workspace-finite/SKILL.md:253-280`)
   — that mismatch is why agents install it on the fly. Add the pinned gws
   binary to `finitecomputer-v2/deploy/finite-computer/images/runtime.Dockerfile`
   at build time. Write `docs/audits/preinstall-audit-2026-07-13.md`
   listing other runtime-installed tools worth baking (evidence from agent
   transcripts/skills, not speculation). Write
   `docs/audits/launch-time-audit-2026-07-13.md`: recon already confirmed
   nothing installs/builds at launch (first-boot work is local copies —
   `run_hermes_gateway.sh:116-147`), so measure where time actually goes
   (image pull, Kata boot, Hermes init, Core polling) using local devfinity
   timings and existing prod logs. Report image size before/after the gws
   addition; if it grows launch time materially, say so and let Paul
   decide.
4. **Google scopes per PR #34.** The PR is documentation-only; its finding:
   the current grant carries full Drive + Apps Script scopes with no
   corresponding action, while Docs is read-only. Apply the repo-side
   reconciliation (scope constants/config + skill docs): drop Apps Script,
   add Docs read/write, per the PR doc's acceptance list. Do NOT touch live
   Google Cloud credentials or the consent screen. The report must list
   Paul's side precisely: console scope changes, re-consent expectations,
   which existing authorized users must re-auth.
5. **Honest "working" indicator (minimal).** Recon: the label is the OR of
   server `typing_members` and client `pendingAgentTurns`
   (`hosted-web-chat.tsx:200-214`); pending turns clear ONLY when a remote
   message arrives with `final_delivery === true`
   (`presentation.ts:183-230`), so a dead stream or lost final latches
   "working" forever; a stale server-side `working` member also never
   expires client-side. Reference behavior: Hermes platforms refresh typing
   every 2s and stop in `finally` (`hermes-agent/gateway/platforms/base.py:1440-1484`);
   the finitechat adapter sets activity with a **15s TTL**
   (`adapter.py:632-642`). Minimal client-side fix: treat both signals as
   leases — expire stale ones on the client (align with the adapter's 15s
   TTL), reconcile `pendingAgentTurns` against stream liveness. No protocol
   changes. If honesty needs more than client-side lease logic, write
   `docs/audits/chat-working-state-audit-2026-07-13.md` and move on.
6. **Skills audit.** Three checks: (a) dashboard skills view — find why
   some skills render without a description (catalog parse vs missing
   front-matter — fix whichever is small); (b) the view reflects the true
   `finite-skills/` tree — note that prod currently falls back to the
   archived GitHub repo because `FC_FINITE_SKILLS_SOURCE_DIR` is unset in
   infra (parking-lot item from 07-10; fixing infra env is CREEP, but say
   so loudly in the report); (c) sweep every `finite-skills/skills/*` for
   legacy references — tools, paths, or binaries that don't exist in the
   mono runtime image (the gws-CLI section in the google-workspace skill is
   confirmed; item 3 fixes the binary — update the skill text to match
   whatever ships). Write findings into
   `docs/audits/skills-audit-2026-07-13.md`; apply only front-matter and
   skill-text fixes.
7. **AEON image detection, zero-effort.** Recon: vision is an automatic
   media-interception seam, not a tool — `_specialize_event_media`
   (`finitechat/integrations/hermes/finitechat/adapter.py:603-630`)
   intercepts image attachments and splices the vision model's text into
   the event, but ONLY when `auxiliary.vision` is configured, which agentd
   reconciles after a health probe (`daemon.rs:253-281`,
   `probe_hermes_vision.py`); unconfigured → `unavailable_results`
   placeholder. Audio/video capabilities are currently `false` in the
   active fragment. Locally: send an image in chat, observe which path
   fires. If the specialization worker isn't reachable locally (it deploys
   on clawland), verify probe/reconcile/fallback behavior, and write
   exactly what a prod test needs from Paul. Do not stand up a local GPU
   worker (CREEP).
8. **FAL image/video generation + display.** Recon: "draw me X" is NOT FAL
   — it's Hermes' built-in `image_generate` tool (OpenAI/xAI providers,
   bundled in default toolsets); FAL is reached only via the image-editing
   and music skills, gated on `FAL_KEY`. Verify locally: text-to-image via
   the default path, image editing via the FAL skill, and that results
   display in chat. Known display gap: only `kind === "Image"` renders
   inline — video/audio degrade to a bare file link
   (`hosted-web-chat.tsx:844-846`); the attachment proxy already forwards
   correct content-types, so adding `<video>`/`<audio>` branches in
   `AttachmentCard` is a contained frontend fix. Document the actual
   generation-dispatch behavior in the report so Paul knows what "default"
   really is.
9. **Telegram audit (timebox 45 min, audit-first).** The dumb mistake is
   already found: `finitechat/containers/agent/Dockerfile:14` installs
   `hermes-agent==0.18.2` WITHOUT the `[messaging]` extra, so
   `python-telegram-bot` is absent and enabling telegram would fail at
   import — legacy explicitly bundled it. Fix: add the extra (or the
   pinned dep) at image build; verify import succeeds in a local image
   build. Also note in the report: mono uses agentd config offers (not
   legacy's env vars), and only `{enabled, token, home_channel,
   reply_to_mode, gateway_restart_notification, typing_indicator, extra}`
   pass the allowlist (`finite-agentd/src/config.rs:556-607`) — anything
   else needs `extra` passthrough. List the exact manual steps Paul should
   run for his test so he wastes zero time.
10. **“Previous Conversations” regression and wrong-room replies.** Sol 2
    still shows the retired “Previous Conversations” group (screenshot
    supplied 2026-07-13) and reportedly replies only in those Chats. Do not
    delete or mutate Sol 2 during this run; use its symptom only as evidence.
    Reproduce the state with synthetic local data, trace whether the group is
    coming from associated-room retention, binding/bootstrap, or dashboard
    presentation, and fix the smallest repo-side cause that prevents a newly
    created Agent from inheriting the condition. Prove in the local browser
    that a fresh Agent shows only its intended current conversation tree and
    that a reply lands in the selected current Chat. A durable-state repair,
    migration, daemon/protocol change, or production mutation is CREEP; report
    the read-only evidence and exact Paul-owned cleanup instead.

## Out of scope

Anything touching `main` or prod (fences 1-2); daemon/proto/Core changes;
new user-facing surfaces or features; Electron; Stripe (its own ACTIVE run);
Phala; infra env changes (report them instead); dependency additions beyond
the two named; fixing anything discovered but not on this queue.

## Governing documents

- [`docs/runs/README.md`](README.md)
- [`docs/runs/hosted-web-chat-disaster-recovery.md`](hosted-web-chat-disaster-recovery.md)
- [`docs/monorepo-doctrine.md`](../monorepo-doctrine.md)
- [PR #34 — connector platform exploration](https://github.com/finitecomputer/finite-mono/pull/34)
