# Production release & rollback

Consolidated 2026-08-29 (essentials task 10) from the former per-service
deploy runbooks, the platform wave, the production-CD flow, the CLI release
procedure, and the agent-runtime image promotion. Fleet facts below are dated
2026-08-29; `infra/README.md` and ADR 0007 are the topology authority —
re-check them before any wave.

- **App-plane host:** finite-lat-2 (64.34.80.19) per
  [ADR 0007](../../docs/adr/0007-finite-lat-2-emergency-app-plane-cutover.md).
  Until the lat2 cutover's Gate E closes, the app plane is in transition and
  [lat2-replacement-cutover.md](lat2-replacement-cutover.md) owns every
  app-plane mutation; this runbook is the steady-state authority before and
  after that window.
- **Kata runner hosts:** finite-lat-3 (207.188.7.157) and finite-lat-4
  (152.236.34.15). finite-lat-1 (64.34.82.77) is DOWN (thermal, 2026-08-27)
  and must never resume writing.
- **Known gap:** the production CD conductor's manifest
  (`infra/deployments/production.toml`) still names the dead lat1 scope, and
  its deploy/rollback scripts hard-require it. Re-targeting the conductor
  onto lat2 is road-to-zero item 1; until it lands, treat the conductor as
  unverified for the new app plane and use the closure-artifact flow (§4).

## 1. What ships from where

| Surface | Source of truth | Verified by |
|---|---|---|
| App-plane NixOS closure — Core, chat server, hosted-device, sites, brain, identity, Caddy edge, Postgres config, monitoring receivers | `infra/nixos/` at the deployed rev; CI closure-artifact workflow (`Lat2 NixOS Closure`; the `Lat1` workflow built the frozen pre-cutover host) | `/run/current-system` equals the artifact's exact `SYSTEM` path |
| Dashboard | digest pin in `infra/nixos/modules/dashboard.nix` | host-running digest equals the pinned `@sha256:` |
| Agent Runtime image, **new** launches | Core's promoted runtime-artifact record plus `FC_RUNNER_RUNTIME_ARTIFACT_ID` in `/etc/finite/runner.env` on every Kata host (lat3, lat4) | `scripts/finite-status` pin green per host |
| Agent Runtime image, **existing** Agents | Core's per-Runtime record — Agents pin at launch and never auto-update | `scripts/finite-status` |
| CLIs (`finitechat`, `fsite`, `fbrain`) | component source tags + rolling aliases in the public `finitecomputer/finite-releases` repository | checksum + alias verification (§8) |
| Why a version shipped, when a fleet roll completed | [`infra/deployment-changelog.md`](../deployment-changelog.md) (record, never authority) | — |

Every release and promotion edits exactly one source of truth. What is out
in the field is read from where it is pinned, never from a copy; stranding a
fielded artifact must be a deliberate, reviewed act in that source. Anything
the pin cannot express goes in the changelog. There is no hand-maintained
deployment ledger, and none may be reintroduced — `just
runbook-facts-contract` fails any runbook that names the retired one.

## 2. The wave, in order

A platform wave carries runner upgrades, a Core migration generation, the
app-plane NixOS closure, the dashboard digest, and an agent-runtime image pin
in one sitting. Component mechanics live in this file; the ORDER and the
gates between steps are the wave.

**The one invariant everything else serves: runners roll BEFORE Core
restarts.** An upgraded Core writes a control-request vocabulary and runs
schema expectations the previous runner generation cannot parse or honor
(`runtime_control_requests.status` gains values past
requested/running/succeeded/failed in Migration 0021). Reverse skew — new
runners under a not-yet-upgraded Core — is bridged and tolerated. Mixed
windows are bridged but not free: keep the whole wave inside one sitting,
hours not days.

1. **Preflight census clean.** `scripts/rollout_preflight.py` exits 0: every
   `runtime_control_requests.status` inside the legacy four values, zero
   non-terminal upgrade-kind requests, and the long-lived-running inventory
   eyeballed (those rows re-label to `launching`). Exit 1 means STOP and
   disposition rows first; Migration 0021 fails Core startup closed
   otherwise.
2. **Stage and apply the runner generation on every Kata host (lat3, lat4)
   — before Core.** New runner binaries applied and
   `FC_RUNNER_RUNTIME_ARTIFACT_ID` present EXPLICITLY in
   `/etc/finite/runner.env`. There is no implicit default;
   `scripts/finite-status` renders a missing pin as RED/absent — a halt
   condition. Never hand-edit `runtime_artifact_id` records or bypass the
   promotion path in §2(c). Confirm via finite-status: both hosts show pin
   matched/green and the service active.
3. **Runtime image promotion** (if the wave carries a new digest) — see
   §2(c); serial existing-Agent rolls follow in §2(d).
4. **App-plane closure switch** (Core + chat + sites + brain + identity +
   dashboard + edge) via the production branch conductor (ADR 0006) or, until
   it is re-targeted, the closure-artifact flow in §4. Mid-switch watches:
   - A nonzero switch is triaged by WHICH units failed before any retry (a
     monitoring-only failure is not a rollback trigger; a failed core/chat
     unit is).
   - **Boot-loop signature — kill the wave on sight:** a gateway process
     SIGKILLed roughly every ~41 seconds, healthcheck red, while control
     requests report success. Bridge/readiness budgets: cold starts take
     tens of seconds and readiness commonly lags "active" by 30–80 seconds —
     give every gate the generous deadline before declaring failure.
   - After ANY restart of a chat-affecting unit, explicitly ensure the
     Hosted Web Device unit started again and answers its `/healthz`
     (restart coupling has silently dropped it before).
5. **Serial existing-Agent upgrades** — complete §2(d). Replies landing in
   home-chat on freshly restarted surfaces are the reply-route fallback
   doing its job after mass restarts — flag for the VERIFY round-trip check
   rather than treating as delivery loss.

### 2(c) Runtime image promotion — the two-step order

The runner does not read an image tag. The pin is
`FC_RUNNER_RUNTIME_ARTIFACT_ID`: product launches fetch the promoted
artifact **kind, reference, and state schema from Core** by that ID.
Promotion is two steps, **in this order**:

1. Register the new pinned image as an artifact in Core (service-authenticated
   runtime-artifact registration endpoint: OCI kind, immutable digest
   reference, source revision, state schema) and promote it for the Kata
   class. Recovery support is immutable artifact material: set
   `recoverKnownGoodChat=true` only for an image whose exact digest passed
   the one-shot recovery receiver tests (the field defaults `false`, so N-1
   artifacts and rollbacks do not inherit a control their image cannot
   execute).
2. Edit `FC_RUNNER_RUNTIME_ARTIFACT_ID` in `/etc/finite/runner.env` on every
   Kata host (lat3 and lat4). That operator file is the only place the pin
   exists; the unit fails closed at start if it is missing. No restart
   needed — the 20 s timer re-invokes the runner with the new env (set
   `FC_RUNNER_DRAIN=true` first to let in-flight launches settle).

**Order matters (learned 2026-08-27):** finish step 1 before touching any
host pin. A pin referencing an id Core does not know makes every runner
cycle fail closed with `HTTP 404 {"error":"runtime artifact is not
configured"}` — reverting the pin to the previous registered id restores
capacity immediately.

The image itself is built once by `.github/workflows/runtime-image.yml`,
which smokes the exact saved image (durable Add/Welcome chat + `/home/node`
restart) before publishing and prints the pinned canary ref — the only thing
you promote. Rung-ladder discipline: local proof → Docker proof → Kata →
Phala/Tinfoil; no confidential-lane promotion without a recorded proof at
the rung below. `.github/workflows/hermes-runtime-smoke.yml` is optional
source preflight (it rebuilds), never promotion evidence. When introducing
the artifact-capability column: drain new Kata creation before the
Core/Runner generation switch, register the new digest through that
generation, update the pin, then clear the drain.

### 2(d) Serial existing-Agent upgrade (§4a)

Promotion changes only future launches. Existing compute moves only through
an explicit Runtime Upgrade request bound to one artifact id (never the
`destroy` endpoint — it offboards the Runtime and revokes its credentials).
First use is staged across two Core generations: the first ships the new
schema/parser with `FC_CORE_ENABLE_RUNTIME_UPGRADES=false` (Nix default) and
must be live as the known-compatible rollback target; only a later
config-only generation sets the gate `true`. Before activating, this
preflight must return no rows (the migration fails closed rather than guess
which provider operation to cancel):

```sql
SELECT agent_runtime_id, count(*)
FROM runtime_control_requests
WHERE status IN ('requested', 'launching', 'compute_up', 'ready')
GROUP BY agent_runtime_id
HAVING count(*) > 1;
```

After a canary passes, prepare and execute the reviewed plan
(`scripts/rollout-lat1-runtime-artifact`; single-host by construction —
re-hosting the wrapper for the post-lat1 fleet with `--host lat3|lat4` is a
road-to-zero item):

```sh
scripts/rollout-lat1-runtime-artifact \
  --prepare \
  --roll-runtime-artifact finite-agent-runtime-YYYY-MM-DD.N \
  --roll-admin-email operator@example.com \
  --roll-admin-workos-user-id user_operator \
  --roll-all \
  --roll-canary-project-id project_canary
```

Review the counts, exclusions, target digest/schema, and plan hash, then run
the copy-paste `--execute-plan-hash <approved-64-hex>` command it emits.
Execution recomputes the whole plan and provider snapshot before mutation,
rechecks each exact Runtime before enqueueing, and verifies target artifact,
image, schema, writable `/data` bind, topology, and unchanged Agent
Principal afterward; it stops on the first drift, failure, timeout, or failed
postcondition. Serial pace is ~2 minutes per Agent — budget linearly. If a
run halts, RESUME THE SAME PLAN HASH; the `.local-state/runtime-rollouts`
event stream summarized by finite-status is the progress authority. Stopped
agents' ports get squatted while down — an identity-bound guard refusing a
step is protecting you. Do not edit a prepared roster to make it pass;
prepare a fresh plan after resolving the concrete drift. Never use this path
to reconstruct missing compute.

**Lifecycle probe gate:** before enqueueing each entry the wrapper consults
the runner's read-only `lifecycle-probe`. A non-operable verdict
(`degraded`, `inoperable`, `unknown`) is a deliberate skip, not a failure —
fix the underlying finding and re-execute the same hash. For an
exactly-one-agent emergency with an understood verdict, `--probe-override`
requires exactly one `--roll-project-id`, hard-refuses `--roll-all`, records
the full probe report, and keeps every other check; a malformed probe report
still fails closed. Running `finite-saas-core runtime-artifact-rollout`
directly by hand bypasses the probe, drift checks, and postflight —
break-glass only.

## 3. Preconditions that are still human

- The change-set is merged to `main` and CI is green at exactly that
  merge-commit SHA.
- **RuntimeSpec generation:** the Core Nix module carries the same
  non-secret `FINITE_SITES_API`, `FINITE_BRAIN_SERVER_URL`, and
  `FINITE_BRAIN_PUBLIC_BASE_URL` values in `FC_CORE_RUNTIME_ENV_JSON` that
  previously lived only in Runner config (Runner's
  `FC_RUNNER_RUNTIME_ENV_JSON` is N-1 fallback only). `FINITECHAT_OWNER_NPUBS`
  is the one spec-env key Core adds beyond that map: it is per-request state
  (the owner's hosted-chat account id, submitted by the dashboard at agent
  creation) injected into the RuntimeSpec environment at lease time — never
  an operator-set value in Core or Runner env files.
- **Schema generations:** a fresh `pg_dump` of the Core database captured to
  a named location immediately before the closure switch; record its path
  and checksum (the `schema-change` classification in §6 requires them).
- **Rollback targets recorded before touching anything:** `readlink -f
  /run/current-system` on every touched host, copied off-host; the previous
  closure artifact confirmed still downloadable (14-day retention — re-plan
  if it ages out mid-day); the previous runtime-artifact digest and dashboard
  `@sha256:` from the pins their authorities name.
- **Drift hygiene:** every finite-status drift exception dispositioned
  BEFORE the wave (named in the changelog entry or fixed). A silent upsert
  reactivating a retired runtime's link has cost consecutive waves their
  `--roll-all` passes.
- `FC_RUNNER_DRAIN` is explicitly `false` (or deliberately `true`); never
  unset, never inherited from a prior incident — it silently pauses ALL new
  agent creation.
- **Single-writer posture:** for the whole window nobody runs mutating or
  write-dispatching one-shot CLI invocations beside resident processes
  (diagnostic containers pass `--entrypoint`). Second writers have poisoned
  shared durable state before.
- **Host sanity:** `LimitNOFILE=65536` still declared for the long-running
  services (a 1024 default once produced a full chat outage).
- **Secrets posture:** nothing in logs, tickets, or notes may quote
  environment dumps from containers or units; sealed manifests are checked
  by SHA-256 only.
- **Sites startup reconciliation:** switching sites from the disabled runner
  starts every active App Output, not only newly published ones. Record the
  pre-switch active-output inventory and host memory headroom; name an
  operator-owned disposable canary App Output explicitly — never select by
  identifier or display order. Prove `containerd.service` is active and the
  `containerd-shim-kata-clh-v2` executable is present before switching
  (`/etc/kata-containers/configuration-clh.toml`: `default_vcpus` 1,
  `default_memory` 512).

## 4. Deploy mechanics — the shared closure procedure

Deploying a release IS pinning the flake: the mono rev you build is the rev
the host runs (binaries + config together). The dashboard is the exception —
a digest-pinned GHCR container, bumped by editing
`infra/nixos/modules/dashboard.nix`. Production evaluation/build happens
only in the CI closure workflow; never evaluate or build the closure on the
Mac, on a prod box, or in rescue mode.

1. From a reviewed checkout, select the full commit, prove it is on
   `origin/main`, and dispatch the closure workflow for the app-plane host
   (`lat2-nixos-closure.yml`; `lat1-nixos-closure.yml` built the frozen
   host):

   ```sh
   set -euo pipefail
   git fetch origin --prune
   REV="$(git rev-parse HEAD)"
   [[ "$REV" =~ ^[0-9a-f]{40}$ ]]
   git merge-base --is-ancestor "$REV" origin/main
   gh workflow run lat2-nixos-closure.yml --ref main -f rev="$REV"
   ```

   `REV` must be exactly 40 lowercase hex characters — never a tag, branch,
   abbreviation, or dirty tree. Wait for success, then download and inspect:

   ```sh
   RUN_ID="$(
     gh run list --workflow lat2-nixos-closure.yml --commit "$REV" \
       --json databaseId,conclusion \
       --jq '.[] | select(.conclusion == "success") | .databaseId' \
       | head -1
   )"
   test -n "$RUN_ID"
   ARTIFACT_DIR="target/lat2-nixos-closure-$REV"
   rm -rf "$ARTIFACT_DIR"
   gh run download "$RUN_ID" \
     --name "lat2-nixos-closure-$REV" \
     --dir "$ARTIFACT_DIR"
   python3 -m json.tool "$ARTIFACT_DIR/manifest.json" >/dev/null
   ```

2. Deploy only that artifact:

   ```sh
   just deploy-lat2-closure "$ARTIFACT_DIR"
   ```

   The deploy script validates the manifest, proves the rev is on
   `origin/main`, realizes the exact `SYSTEM` path on the host from the
   manifest-pinned `finite` Cachix cache with local builds disabled, takes
   the pre-deploy recovery snapshot, installs `SYSTEM` as the boot profile,
   activates it, asserts `/run/current-system` is exactly the artifact's
   `SYSTEM` path, and verifies the activated host declares the same Cachix
   trust. A Cachix miss or trusted-key mismatch fails before activation.
   Steady-state activation refuses unexpected product-unit starts; the
   import-mode → product go-live switch is the one `--expect-startup`
   exception and belongs to the cutover runbook, not routine deploys. The
   equivalent `just deploy-lat1-closure` served the frozen lat1 host.

3. **Dashboard image bump:** edit `image = "...@sha256:..."` in
   `infra/nixos/modules/dashboard.nix`, commit to `main` — the committed
   digest is the deploy record and the rollback target — then repeat steps
   1–2 for the new rev. podman pulls the pinned digest.

4. Config-only changes (listen flags, `--app-runner`, env references, Caddy
   vhosts) live in `infra/nixos/modules/` — never edit units on the box; a
   hotfix survives only until the next switch. The sites edge certificate is
   the Cloudflare Origin CA pair at
   `/etc/finite-saas/certs/finite-chat-origin.{pem,key}` (no ACME; the zone
   is Cloudflare-proxied Full-strict — do not "fix" cert errors by switching
   to ACME).

**Protected-branch conductor (ADR 0006):** the normal steady-state path is
the `main` → `production` pull request — `Open Production Deploy PR` opens
it, `Production Deploy Plan` reviews it (validates
`infra/deployments/production.toml`, classifies changed paths, verifies the
tip's `CI gate`, builds the closure, comments the plan), and `Production
Deploy` is the only workflow that can cross the Mutation Boundary (waiting
on the GitHub `production` environment approval first, capturing
`finite-status-before/after` artifacts and the deployment record). Merge the
PR only after the staged SHA's `CI gate` is green, the plan is green, and
the plan has been reviewed; approve the environment deployment only after
confirming the intended source revision. Bootstrap and mutation enablement
were one-time setup (completed; `mutation_enabled = true` in
`production.toml`) — ADR 0006 is the authority, not a repeated procedure.
Until the conductor is re-targeted off the dead lat1 scope (§ Known gap),
deploy through steps 1–2 above.

## 5. VERIFY — state, not process

Each layer gets a machine check AND one human-feelable product probe; a
layer is green only when both agree.

1. **Closure identity:** `readlink -f /run/current-system` equals the exact
   built `SYSTEM` path on every touched host — not exit codes, not
   generation numbers. Spot-check running executables resolve into that
   closure for the long-running services (a restorer race has resurrected
   stale binaries post-switch before).
2. **Serving, not just active:** poll endpoints past `systemctl is-active`
   green until they answer, within generous budgets. Per service:

   | Service | Machine check | Human-feelable check |
   |---|---|---|
   | Core | `curl http://127.0.0.1:4200/healthz`; edge: `https://finite.computer/internal/finite-private/v1/health` → 401 invalid-token; invalid bearer → 401 from both `/api/core/v1/finite-private/usage` and `.../usage/reset` while `/api/core/v1/admin/runtimes` does NOT reach public Core | canary Finite Private key status + reset from a mode-0600 env file (never raw key in argv/logs) |
   | Chat | contract gate (below): exact `source_fingerprint`, `source_dirty: false`; `http://127.0.0.1:8788/health` + `/readyz`; hosted-device `http://127.0.0.1:38918/healthz` | one real message round trip |
   | Sites | `https://api.finite.chat/api/v1/healthz`; unit carries `--app-runner kata` + exact Nix-store nerdctl/CNI/sudo paths; stateful canary sentinel (write → stop container → wake → read back the SAME sentinel); `Cache-Control: no-store` probe (below) | load one published `*.finite.chat` site + one `*.docs.finite.chat` vhost |
   | Brain | `http://127.0.0.1:3015/health` + `https://brain.finite.computer/health`; dashboard `/client` requires a WorkOS session; signed `fbrain` `/_admin/*` request reaches Brain without one; `fbrain doctor` + write/read proof | authenticated browser: embedded Product Client completes a real `/_admin/*` request; for invite changes, one disposable Brain + Folder invitation email drill (code + instructions, no URL fragment/secret; both report `sent`) |
   | Identity | `just identity-edge-contract` (static) + `finite-identity/scripts/identity-edge-contract-gate.py` (live): private routes 404, resolution route 401, both NIP-05 origins byte-identical | one normal managed-agent creation end-to-end |
   | Dashboard | digest byte-equality vs `modules/dashboard.nix` (compare digests; an exit-0 command proves nothing) | dashboard loads and logs in |

   Chat contract gate, run from a mono checkout at the release commit
   (evaluation only; no rebuild):

   ```sh
   set -euo pipefail
   export FINITECHAT_SOURCE_FINGERPRINT="$(
     nix eval --option builders '' --raw \
       .#packages.x86_64-linux.finitechat-server.sourceFingerprint
   )"
   finitechat/scripts/server-contract-gate.py \
     --server https://chat.finite.computer \
     --expected-fingerprint "$FINITECHAT_SOURCE_FINGERPRINT"
   ```

   Sites no-store probe — `no-store` is the application correctness
   boundary while URLs remain mutable (2026-07-23: Cloudflare's default
   four-hour Browser Cache TTL rewrote origin headers and mixed fresh HTML
   with stale JS; keep Browser Cache TTL on "Respect Existing Headers" and
   no Cache Rule overrides):

   ```sh
   for url in \
     'https://<published-site>.finite.chat/' \
     'https://<published-site>.finite.chat/<real-asset>.js'
   do
     headers="$(curl -fsSI "$url")"
     grep -Eiq '^cache-control:[[:space:]]*no-store([[:space:]]|$)' <<<"$headers"
     ! grep -Eiq '^cache-control:.*max-age=[1-9]' <<<"$headers"
   done
   ```

3. **Pins:** finite-status shows the artifact pin matched/green on BOTH Kata
   hosts; RED/absent → halt per §3.
4. **Control-plane census:** rerun `scripts/rollout_preflight.py` — legacy
   vocabulary preserved unless the new one is deliberately exercised; no
   unexpected non-terminal rows.
5. **Launch one throwaway fresh agent** end-to-end (create → chat turn →
   reply in-thread). Pin flips affect new launches only, so this is the only
   check that exercises what you actually shipped.
6. **The human round-trip:** a real user message through the hosted web
   device and one through another client, answered by a real agent, timed
   and recorded. On 2026-08-11 every machine check passed while hosted chat
   was dark — this row exists because of that day.
7. **Load shape:** host quiet afterward (low load1, PSI ≈ 0). Sustained hot
   loops after a "successful" switch have meant a boot loop or a
   thundering restart pile-up.
8. **Record:** diff `scripts/finite-status --json` (saved BEFORE the wave)
   against a fresh AFTER run into the changelog entry: what shipped, when
   the roll finished, compatibility promises still owed, named exceptions.
   Close the wave only when the record is written.

Failure triage: chats gone dark → [incident.md](incident.md) §2 (read-only
first); hosted device unresponsive → [recovery.md](recovery.md); general box
access → [incident.md](incident.md) §1.

## 6. Classification and risky paths

Deployments carry a classification from `infra/deployments/production.toml`
(ADR 0006): `ordinary`, `schema-change`, `forward-only`.

- `schema-change` records a fresh backup path + checksum and a rollback
  target before crossing the boundary (§3 captures them).
- `forward-only` requires explicit production approval and makes no
  automatic rollback promise. Risky-path detection compares
  `production...HEAD` — the question is what production newly receives.
- **One-way binary boundaries (Finite Private epochs):** once an epoch-aware
  Core generation accepts traffic, the previous N-1 closure is not a safe
  live binary rollback target (N-1 ignores epochs: it can charge a freshly
  reset window from a late settlement, and N-1 rows undercount after
  re-upgrade). Prefer a forward fix on the epoch-aware generation. An
  emergency N-1 nevertheless requires an explicitly approved Finite Private
  maintenance window: disable all reserve/settle/status/reset callers, prove
  every `reserved` row resolved, capture a database backup, and document how
  epoch>0 grants plus N-1-written rows reconcile before re-upgrade — tested
  on synthetic restored state first. Before enabling reset epochs, query
  production read-only for reservation cardinality and active `reserved`
  rows (including age) and `EXPLAIN` the grant/epoch/status/window usage
  sum; record recent reservation counts separately from historical rows and
  never rewrite either as part of deployment.

## 7. ROLLBACK

Freeze further rolling first. Classify what broke using the VERIFY
artifacts; the levers below compose in this order.

- **R1 — Services/closures:** redeploy the previous closure artifact (or
  `nixos-rebuild switch --rollback` to the recorded generation path), verify
  `/run/current-system` against the selected path, re-run the scaled-down
  VERIFY battery, and reconcile git to match what is running within a day
  ([incident.md](incident.md) §1 rule). Chat delivery is deliberately
  bidirectional-format-compatible across the hermes ownership swap boundary:
  reverting it costs bounded duplicate turns while leases drain (TTL default
  45 min, env-overridable per host) — designed degradation, not corruption.
  Sites rollback stops serving App Outputs but never deletes
  `/var/lib/finite-sites/apps` or any Sites durable state.
- **R2 — Core rollback REQUIRES its data-migration reversal FIRST:** once
  Migration 0021 has applied, the previous Core generation cannot write
  control requests. Execute
  `migrations/runtime_lifecycle_reverse_remap.sql` (idempotent,
  refusal-guarded, audit-logged; refuses if non-terminal upgrade-kind
  requests exist — finish or retire them first), then
  `runtime_upgrade_rollback_rescue.sql` if rolling past that change too, and
  only THEN start the previous-generation binary. Verify with the census
  (legacy-vocabulary-only view) before declaring R2 done. Until the
  rehearsal drill against scratch production state exists, R2 is break-glass
  assisted by the census tool, not routine.
- **R3 — Agents/runtime pins:** restore the previous
  `FC_RUNNER_RUNTIME_ARTIFACT_ID` on each host (timer applies without
  restart). Existing agents keep their launch-time digest either way. For a
  Kata Runtime that adopted a bad image, explicitly request an upgrade to
  the previous promoted, same-schema artifact — never destroy as the first
  leg. Leave the bad tag in GHCR (immutability beats tidiness) and note it
  in the changelog so nobody promotes it again.
- **R4 — Dashboard:** revert the digest in `modules/dashboard.nix`, rebuild
  the closure, re-verify byte-equality.
- **R-last — Data restore:** only via [recovery.md](recovery.md), with the
  coordinated empty-target proof. Recovery authority precedes operator
  blindness (ADR 0001): user data availability is the invariant that
  survives even a botched wave.
- **Failure after the Mutation Boundary:** MVP has no automated
  lockout/reconciliation — inspect the host directly, decide whether
  production is on the intended closure, partially switched, or rolled
  back, rerun the same deploy when safe or roll back, and record the
  observed system path, action, and reason in the deployment
  record/changelog before approving another production deploy.
- **Rolling Core back across Runtime Upgrade first use:** before first use,
  roll back only to the already-live compatibility generation (new schema
  understood, gate still false). After an Upgrade row exists, an
  older-binary rollback is a fail-closed rescue: set
  `FC_CORE_ENABLE_RUNTIME_UPGRADES=false`, stop the runner timer and service
  so no lease can move, reconcile every active upgrade-kind operation to one
  healthy canonical handle (verify `/healthz`, `/contact`, expected digest,
  single `/data` writer), prove the active-upgrade query returns zero, run
  `runtime_upgrade_rollback_rescue.sql` (rewrites only the legacy
  parser-facing kind to `restart`, audit-logged), verify the audit rows and
  that no `kind='upgrade'` row remains — only then activate the old closure,
  and keep Runtime Upgrade disabled until the compatibility generation is
  restored.

## 8. CLI releases (finitechat / fsite / fbrain)

GitHub `finitecomputer/finite-mono` is the source authority; GitHub Actions
builds from component-scoped tags and publishes to the public
`finitecomputer/finite-releases` repository, which never contains product
source. Asset names are product contracts. Installers use the per-component
rolling alias, never GitHub's repository-wide `releases/latest`:

| Component | Source tag | Workflow | Rolling alias |
|---|---|---|---|
| finitechat | `finitechat/vX.Y.Z` | `.github/workflows/release-finitechat.yml` | `finitechat-latest` |
| fsite | `fsite/vX.Y.Z` | `.github/workflows/release-fsite.yml` | `fsite-latest` |
| fbrain | `fbrain/vX.Y.Z` | `.github/workflows/release-fbrain.yml` | `fbrain-latest` |

Preconditions: the exact commit is on `main` with `CI gate` green;
`FINITE_RELEASES_GITHUB_TOKEN` is scoped to Contents write on
`finite-releases`; variable `FINITE_RELEASE_PUBLISH_ENABLED` is exactly
`true` (unset for shadow runs); the version is newer than every existing
`<component>/v*` tag.

1. Pick `vX.Y.Z` against the latest existing tag. If the release changes the
   server-compatibility story, record that promise in
   `infra/deployment-changelog.md` in the same PR.
2. Tag the exact merge commit and push: `git tag finitechat/vX.Y.Z <main-sha>`
   && `git push origin finitechat/vX.Y.Z` (same shape for `fsite/`, `fbrain/`).
3. Watch the workflow: it derives version + source SHA from the tag, records
   `release.json`, checksum-verifies every asset after upload, then refreshes
   the alias. A retry reuses verified immutable assets.
4. Verify: checksum the versioned archive from `finite-releases`; repeat
   through the rolling alias; run the component README's clean-install block
   away from this checkout and confirm `--version`; confirm `release.json`
   names the source SHA and run ID:

   ```sh
   base=https://github.com/finitecomputer/finite-releases/releases/download/finitechat-latest
   curl -fsSLO "$base/finitechat-linux-x86_64.tar.gz"
   curl -fsSLO "$base/finitechat-linux-x86_64.tar.gz.sha256"
   sha256sum -c finitechat-linux-x86_64.tar.gz.sha256
   ```

Versioned releases are immutable — prefer a patch release. Alias-only
rollback moves the alias without rebuilding:

```sh
gh workflow run release-finitechat.yml \
  --repo finitecomputer/finite-mono \
  --ref main \
  -f publish=true \
  -f alias_only=true \
  -f release_tag=finitechat/vPREVIOUS
```

Never delete a versioned release or overwrite its assets.
