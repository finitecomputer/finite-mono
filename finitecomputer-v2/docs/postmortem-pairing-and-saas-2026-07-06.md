# Post-mortem & Handoff — SaaS rebuild + hosted-agent pairing (2026-07-06)

Author: Fable (Claude). Written at handoff to a new agent. Read this top to
bottom; it is a working handoff, not a polished retro. Related working notes
live in the sibling `finite-fable/` folder (not a repo; `../finite-fable/`).

---

## TL;DR — where we are

1. **The SaaS data-layer rebuild is DONE and deployed.** Core Phases 0→2c are
   live in prod. The global-lock/whole-DB-rewrite model is gone; surrogate
   IDs, row-scoped SQL, billing ordering guard, error observability all
   shipped. Standard in `crates/finite-saas-core/PERSISTENCE.md`.
2. **A hosted agent finally boots and runs on Phala** (after 3 runner↔Phala
   CLI-schema fixes). The runtime image is complete (finitechat v0.1.2 + fsite
   v0.3.1 + fbrain v0.1.1, all on the shared identity).
3. **Pairing is the open issue.** Join works (proven end-to-end locally), but
   the FIRST message hit an MLS `SecretReuseError`. We were mid-isolation of
   whether that's a real bug or a polluted-test-room artifact when this was
   handed off. **This is the thing to pick up first — see §1.**

---

## §1. THE OPEN ISSUE — pick this up first

### Symptom timeline
- Paul's iOS app hung at **"waiting for room admission"** when pairing with a
  prod hosted agent (Fable Test 4).
- **Root cause of the hang: version skew.** Paul's phone ran an old finitechat
  (~v0.1.0); the agent runs **v0.1.2** (2 commits ahead of finitechat `main`,
  which is stale at `28448cd` — the v0.1.2 tag is `4744f7ba`).
- Proven via a local rung-2 harness (see §4): a **v0.1.2 CLI joiner** and a
  **v0.1.2 iOS simulator** BOTH pair with the same v0.1.2 agent instantly
  (`state: joined`, sim shows the room). So the agent, server, no-PIN invite,
  and MLS join path are all fine. It was purely the stale client.
- We pushed **v0.1.2 to Paulphone Air** (installed OK over USB). Paul then
  paired with the local agent — the join worked, but sending a message
  produced on the phone:
  `delivery error failed to process MLS message:
   ValidationError(UnableToDecrypt(SecretTreeError(SecretReuseError)))`

### The fork the next agent must resolve
`SecretReuseError` = the MLS SecretTree was asked for a per-message secret
that was already consumed. Two hypotheses, not yet disambiguated:
- **(A) Polluted test room.** The room it happened in (`fable-rung2`,
  "Fable Rung2") had the agent + a CLI joiner + the iOS sim + Paul's phone,
  plus several re-minted invites. Rapid multi-member MLS churn desyncs the
  secret tree. NOT the clean prod scenario.
- **(B) Real message-delivery bug** in finitechat v0.1.2 that the join test
  didn't exercise (joining ≠ messaging).

### Exactly where we left off
A **clean 2-party agent** was started to isolate this: container
`fable-rung2b` (port 18081), room **`room-55962edf0f5c662d`** ("Fable Clean"),
agent npub `npub1ml7s0w9dncjpxkdtz362ggdq0enat53epn4nn728003zausm7rssdvaxwl`.
Paul joined it (health `/invite` shows `paired:true`) but **handed off before
reporting whether a message in the CLEAN room reproduces SecretReuse.**

### Next steps for the new agent
1. **Get the clean-room result.** Have a single v0.1.2 client (Paul's phone,
   or the local CLI joiner at `scratchpad/finitechat`, or the sim UDID
   `8F5AE9C5-3BFD-486F-A75A-7B8B9993BCDE`) join ONLY `fable-rung2b`, send one
   message, and observe. If it works → SecretReuse was the polluted room (A),
   proceed to a fresh prod agent. If it recurs → real bug (B).
2. **The gateway logs are too quiet by default** — `docker logs` shows only
   the startup banner, nothing about MLS/message internals. To debug (B),
   restart the agent container with debug logging (`RUST_LOG=debug` and/or the
   finitechat log-level env) to capture the SecretReuse context, then dig into
   finitechat's message delivery / SecretTree handling in the `finitechat-*`
   crates at ref `4744f7ba`.
3. **finitechat `main` is behind the `v0.1.2` tag** — worth fast-forwarding
   main (or confirming intent) so "latest" isn't ambiguous.

### Secondary bug found along the way (worth fixing)
**Premature "paired".** A single-use invite consumed by a join that never
completes still marks the agent `paired:true`. Fable Test 4's prod agent
reports `paired:true` though Paul never actually joined its room. The health
server / `hermes invite-status` treats "invite session consumed" as "paired,"
which is wrong when the client's MLS join never finished. See
`containers/agent/health_server.py` (finitechat) + `hermes invite-status`.

---

## §2. What's deployed in prod RIGHT NOW

Host: `finite-lat-1` (ssh alias), k3s namespace `finite-system`. Core/dashboard
built on-host with podman (NOT via CI — a known asymmetry).

| Component | Version | Notes |
|---|---|---|
| finitecomputer-v2 `main` | `862e6bf` | Phala endpoint fix (runner-only) atop Phase 2c |
| Core image | `localhost/finite-saas-core:6138718` | Phase 2c (data-layer rebuild complete) |
| Dashboard image | `localhost/finite-saas-dashboard:b4982e9` | Phase 2b; 2c was Core-only so not rebuilt |
| Runner binary (on lat1) | built `Jul 6 18:14` @ `862e6bf` | both Phala-CLI-schema fixes in |
| Promoted runtime artifact | `finite-agent-runtime-2026-07-06.1` | digest `sha256:0026db195bb7fb9d43025297fe7bcb4e92cf5d5c95101adb1264ec88dd370a80` |
| Runtime image contents | finitechat `v0.1.2` (4744f7ba), fsite `v0.3.1` (768a0b8), fbrain `v0.1.1` (8e1033c), Hermes 0.18 | fbrain now on shared identity + nip-05 |
| Finite Private model | `glm-5-2` @ `https://kimi-k2-6.finite.containers.tinfoil.dev/v1` | domain name historical; serves GLM |
| Phala runner | systemd timer active, backend `phala`, node `prod5` | run-once every ~21s |

The GHCR runtime package is **public**. Core runs migrations on boot.

### The 3 runner↔Phala fixes that got the agent booting (all in `862e6bf`)
The runner's Phala integration was written against a `phala` CLI schema that
doesn't match the installed 1.1.x. Fixed in sequence (each a real launch
failure): (1) `apps --search` returns the list under `items` (not
`dstack_apps`), items use `cvmName`/`appId`; (2) node name is at
`node_info.name` (not `teepod.name`), and `cvms get` has no `data` wrapper;
(3) **use the CLI-provided URL in `endpoints[].app` verbatim** instead of
reconstructing the hostname. Tests pin the real shapes in
`crates/finite-saas-runner/src/lib.rs`.

---

## §3. The SaaS data-layer rebuild (DONE) — context for the new owner

Triggered by a prod `store error: db error` on agent creation. Root cause:
`agent_creation_entitlements` lacked `UNIQUE(customer_org_id)` that an
`ON CONFLICT` required → deterministic failure for every standard-billing
create, invisible because errors were swallowed and untested on that path.

Aligned decisions (quiz answers, 2026-07-06): rebuild the data layer / finish
the row-scoped migration / test both ways (PG-in-CI + golden E2E) / verify
identity per request / surrogate IDs. Plan + progress in
`../finite-fable/saas-rebuild-plan.md`; audit in
`../finite-fable/saas-audit-2026-07-04.md`.

Shipped (all merged + deployed):
- **Phase 0** (`3d302f6`): `tracing`, `as_db_error` capture, `CoreError::Database`
  + generic user message + correlation id, the constraint hotfix.
- **Phase 1** (`77f61d0`): ephemeral-Postgres-per-test harness
  (`with_isolated_postgres`), golden-path E2E, **`PERSISTENCE.md`** (the
  enforced standard — read it before touching store code).
- **Phase 2a** (`426f3ac`): surrogate random IDs + natural-key lookup;
  idempotency via `UNIQUE(owner_user_id, idempotency_key)`. Wipe→re-signup now
  gets fresh keys (kills the original collision class).
- **Phase 2b** (`b4982e9`): billing ops row-scoped; Stripe event-ordering
  guard (`last_stripe_event_created`); `billing_overview` is now a pure read.
- **Phase 2c** (`6138718`): remaining ~32 ops row-scoped, lease queue
  partitioned (`target_source_host_id`, currently always NULL/inert), and the
  **global advisory lock + `persist_state` DELETED** (structural guard test).
  Adversarial review confirmed the metering path matches the in-memory spec.

**NOT done — remaining plan:**
- **Phase 3**: per-request WorkOS JWT verification in Core (today Core trusts
  `x-finite-workos-*` headers behind the shared `FC_CORE_API_TOKEN` — token
  holder can impersonate anyone incl. admin).
- **Phase 4**: reaper for stuck launches (fail expired leases, time out
  unclaimed `requested` requests, free entitlement quota, user escape hatch
  beyond `failed`). Relevant: the "waiting to launch forever" trap.

**Open decision for Paul:** prod's default Finite Private profile
`finite-private-generous` has burst=50M, **weekly=NULL (uncapped)**. 2c's code
sets 25M only on fresh DBs (`ON CONFLICT DO NOTHING` won't touch the existing
row). So real GPU inference is currently **weekly-uncapped** in prod. Backfill
is a one-line UPDATE but would start denying anyone already over 25M/week —
Paul's call, not done.

**Resolved 2026-07-14:** local and fresh Core databases match the deployed
profile: burst=50M and weekly=NULL. Existing databases are aligned by the
idempotent `0010_align_finite_private_generous.sql` migration.

---

## §4. Local rung-2 test harness (running on Paul's Mac — throwaway)

This is the harness that isolated the pairing bug in ~20 min. Two containers
running the EXACT prod image:

- `fable-rung2` — port 18080, "Fable Rung2" room `room-ea7d83f9fd6e319a`.
  POLLUTED (agent + CLI joiner + sim + phone, multiple invites). Where
  SecretReuse first appeared.
- `fable-rung2b` — port 18081, "Fable Clean" room `room-55962edf0f5c662d`.
  Clean 2-party target for isolation.

Both use env faithful to the runner's `docker_runtime_env` (see
`scratchpad/rung2.env` / `rung2b.env`), pointed at prod `chat.finite.computer`
and the real GLM key. Full `docker logs` visibility (but gateway is quiet —
raise log level to debug).

Scratchpad dir:
`/private/tmp/claude-501/-Users-futurepaul-dev-finite/1fd33a2d-75d8-4980-a5e0-ed7cf7db7242/scratchpad`
contains: the v0.1.2 `finitechat` binary (headless joiner), `joiner-home/`,
`rung2*.env`, `sim-shot.png` (proof the sim joined).

iOS: sim UDID `8F5AE9C5-...BCDE` (iPhone 16, v0.1.2 app installed); Paulphone
Air (hardware UDID `00008150-0010149A26F0401C`, team `JBLHZ83X6T`) now has
v0.1.2. Drive an auto-join with:
`xcrun simctl launch <sim-udid> computer.finite.finitechat --finitechat-auto-join <URL>`
(terminate first). Physical rebuild:
`cargo run -p finitechat-rmp -- run ios --udid 00008150-0010149A26F0401C --development-team JBLHZ83X6T`
from the `sim-v0.1.2` worktree.

**Teardown when done:** `docker rm -f fable-rung2 fable-rung2b`; the finitechat
worktree `../finite-chat-darkmatter-worktrees/sim-v0.1.2` (detached at v0.1.2)
can be removed with `git worktree remove`.

---

## §5. The process lesson (the "where did we go wrong")

We shipped straight to **rung 5** of `docs/hermes-runtime-test-matrix.md`
(prod Phala) having never green-lit rungs 1–4. The matrix's rule is "if a
lower rung fails, do not climb," and acceptance at every rung is "a
human-usable chat from iOS + machine-readable evidence." We produced that
evidence at NO rung — "it boots" was all we ever checked. Consequences:
- Debugging the confidential Phala VM was near-blind (logs disabled,
  `public_logs=false`); we burned hours guessing through 2 HTTP endpoints.
- Running **rung 2 (local Docker, same image, real client, logs)** isolated
  the actual bug (version skew) in ~20 minutes.

**Recommendation for the new owner:** make rung 2 the gate before any promote.
The harness in §4 is reusable. Do not treat "CI green + it boots" as done.

---

## §6. Cross-repo state (the wider system, all shipped this week)

- **finite-identity** (public repo `finitecomputer/finite-identity`, rev
  `54a6936`): shared Nostr identity contract. `~/.finite/identity/identity.json`
  (or `$FINITE_HOME`), `FiniteIdentity::load/load_or_generate/import`,
  `CLI-CONVENTIONS.md` (`auth status`/`auth import`, stdin-not-argv).
  Open: nip-05 identification design in issue **#1**.
- **fsite** `v0.3.1`: shared identity, `auth` verbs, file-backed git
  credential store (no OS keychain). LIVE on finite-lat-2.
- **finitechat** `v0.1.2`: shared identity, `auth status`/`import`, single-use
  no-PIN invites, agent-first README + release binaries. Person-oriented
  invites design filed as issue **#2** (not built).
- **finite-brain** `v0.1.1` (`8e1033c`): shared identity (#73 merged) +
  nip-05 member resolution. Now baked into the runtime image.

---

## §7. Outstanding worktrees

### Mine, from this session (act on these)
- **`finite-chat-darkmatter-worktrees/sim-v0.1.2`** — detached at `4744f7b`
  (finitechat v0.1.2). ACTIVE / KEEP: this is the exact-version checkout used
  for the iOS sim + phone builds and is what the pairing debug (§1) runs from.
  Remove only after the SecretReuse issue is resolved
  (`git -C ../finite-chat-darkmatter worktree remove finite-chat-darkmatter-worktrees/sim-v0.1.2`).
- **`finite-brain-worktrees/finite-identity-adoption`** — branch
  `finite-identity-adoption` @ `984fcfb`. STALE: this was PR #73, now MERGED
  into finite-brain main (v0.1.1). Safe to remove
  (`git -C ../finite-brain worktree remove finite-brain-worktrees/finite-identity-adoption`
  then delete the branch).

All my finitecomputer-v2 worktrees (phase0/1/2a/2b/2c, runtime-refresh,
phala-endpoint-fix, admin-ops-v0, vertical-slice, etc.) were removed after
merge — `finitecomputer-v2` has only `main` checked out. Good state.

### Pre-existing (NOT mine this session — other agents/sessions; left as-is)
Do not assume these are current; verify remote/branch/ancestry before using.
- finite-chat-darkmatter: `hermes-plugin-legacy-collision`,
  `hermes-sidecar-hardening`, `no-pin-agent-profile`, `pika-audit`,
  `pr-1-skyler-review` (codex/review-pr-1), `profile-images`
  (codex/tinfoil-debug-canary), `self-hosted-runner-ci`, `v2-deploy-docs`.
- finite-sites: `bare-repos-skills`, `hard-cut-engineering-style`.
- finite-brain: `hard-cut-engineering-style`.
- confidential-kimi-worktrees: `glm52-private-model`.

---

## §8. Quick reference — IDs & endpoints

- Prod hosted agent (Fable Test 4, likely leave/destroy): runtime
  `runtime_598d1ca82eecf9e5a810`, Phala CVM base
  `https://add7e6f763dab8683f4ee5809164f8d06bae96d7-8080.dstack-pha-prod5.phala.network`
  (`/healthz`, `/invite`). Reports `paired:true` prematurely (see §1).
- Prod DB: `kubectl exec -n finite-system finite-core-postgres-0 -- psql -U finite -d finite_core`.
- Phala CLI (on lat1, source `/etc/finite-computer/runner.env` for the key):
  `phala apps --search <name> --json`, `phala cvms get <appId> --json`,
  `phala cvms delete <appId> --yes`. **Check for orphaned CVMs**
  (`phala apps`) — failed launches leak running CVMs (cost); we cleaned the
  known ones but verify.
- Mint a fresh invite from a running agent container:
  `docker exec <container> finitechat hermes --agent-home /data/agent invite --room-id <room> --json`
