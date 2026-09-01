# Incident response (incl. host failure)

Consolidated 2026-08-29 (essentials task 10) from break-glass,
chats-appear-missing, stripe-billing, and the exercised emergency patterns
(ADR 0007 lat2 cutover, lat4 runner install). Host facts (services, ports,
secrets locations) live in `infra/hosts/<name>/README.md` and
`infra/nixos/hosts/<name>/` — read the host's docs before touching a box.

> **The rule:** any manual change made on a host must land back in `infra/`
> (or be reverted) **within a day**. The whole value of this tree is that it
> matches reality; an undocumented hotfix is drift, and drift is how the
> pre-mono mess happened. Note the change in your PR even if it is
> embarrassing.

## 1. First five minutes

1. Run `scripts/finite-status` — the only platform/fleet status command
   (read-only; exits 0 all green, 1 any red, 2 undetermined). It observes
   service/HTTP health, verifies the sealed recovery manifest by SHA-256
   only, summarizes Borg results and the runtime-rollout event stream, and
   never opens a snapshot SQLite database as a database. If an incident
   tempts you to type an ad-hoc probe, that probe becomes a PR to
   `finite-status` and its contract test instead.
2. Know which host answers for which role **today** (dated 2026-08-29;
   authority: `infra/README.md` + ADR 0007):

   | Host | Role today |
   |---|---|
   | **finite-lat-2** (64.34.80.19) | Replacement single app server per ADR 0007 — Core, Postgres, chat, hosted-device, sites, brain, identity, dashboard, search, Caddy, backups, litestream, and the `wg-finite` overlay hub at `10.254.3.1`. Until the cutover's Gate E closes, [lat2-replacement-cutover.md](lat2-replacement-cutover.md) owns every app-plane mutation. |
   | **finite-lat-1** (64.34.82.77) | **DOWN** (thermal failure 2026-08-27). Frozen point-in-time record. Must never resume writing: a split-brain here means two Postgres and two chat writers. |
   | **finite-lat-3** (207.188.7.157) | NixOS Kata runner accepting new creation, hard 42-guest ceiling, `FC_RUNNER_DRAIN=false`. |
   | **finite-lat-4** (152.236.34.15) | Third NixOS runner host; lat1's agent state imported 2026-08-29; drained pending the separate owner admission decision. |
   | **smoke** (15.204.56.61) | Legacy Nix-fleet box; Brain rollback source only — not a replica, never selected implicitly. |
   | **clawland** (15.204.108.57) | Legacy `finite.vip` fleet box (k3s + Traefik + oauth2-proxy). |
   | Tinfoil | Measured enclaves (`infra/tinfoil/`). |

3. Get on the box you need (key-only SSH everywhere):

   - **lat2:** `ssh root@64.34.80.19`. Declarative — fix forward in
     `infra/nixos/hosts/finite-lat-2/` and redeploy via the closure flow in
     [release.md](release.md) §4; roll back with `nixos-rebuild switch
     --rollback`. **Import mode:** while `finite.importMode.enable` is true
     the product units stay down by design — bringing the product up means
     deploying the go-live closure, never `systemctl start` by hand. Logs
     (all native systemd/journald): `journalctl -u caddy` (the single edge),
     `-u finite-saas-core` (:4200), `-u podman-finite-saas-dashboard`
     (:3000), `-u postgresql` (native Postgres 16, `finite_core`),
     `-u finitechat-server` (:8788), `-u finite-saas-sites` (:8787),
     `-u finite-brain-app` (:3015), `-u finite-identity` (:8790),
     `-u finitechat-hosted-device`, `-u finite-postgres-backup` (the 6-hourly
     dump timer), `-u finite-litestream-finite-chat-server` /
     `-u finite-litestream-finite-brain` (per-db replicators). Restart the
     named unit after the config
     fix lands; **do not edit units on the box** — a hotfix survives only
     until the next switch.
   - **lat1 (dead):** rescue mode + IPMI for **read-only diagnosis only**.
     Preserve every disk and state source, fence writers if needed, and
     escalate to the
     [finite-lat capacity/redundancy recovery plan](../../docs/runs/finite-lat-capacity-and-redundancy.md)
     before any mutation. Never boot it back into service.
   - **lat3 / lat4 (runners):** `ssh root@207.188.7.157` /
     `ssh root@152.236.34.15`. Declarative like lat2
     (`infra/nixos/hosts/finite-lat-3|4/`). The runner unit is
     `finite-saas-runner.service` driven by `finite-saas-runner.timer`; logs
     `journalctl -u finite-saas-runner`; storage health is the
     `finite-storage-health` unit (RAID1 + dual ESPs + ESP-write guard). To
     pause new work: `FC_RUNNER_DRAIN=true` in `/etc/finite/runner.env` or
     `sudo systemctl stop finite-saas-runner.timer`. Overlay path to Core:
     `10.254.3.1` (lat3) / local `10.254.3.4` (lat4) on `wg-finite`.
   - **smoke:** `ssh root@15.204.56.61` — no ssh alias exists (the
     `ovh-rescue` alias points at clawland). NixOS managed by the legacy
     `finitecomputer` repo: fix forward there (`just host-deploy`), or any
     manual change is silently reverted at the next switch. Logs:
     `journalctl -u finite-brain-app` (the brain :3015),
     `journalctl -u fc-agent-cluster-http-bridge -u fc-agent-cluster-https-bridge`
     (socat edge → k3s Traefik NodePorts), `journalctl -u k3s` (oauth2-proxy
     lives in-cluster). **Traps:** NO backups on this host — the brain
     SQLite at `/var/lib/private/finitebrain/` is the only copy; take a
     manual copy before anything risky. `/_admin` bypasses oauth at the
     edge.
   - **clawland:** `ssh ovh-rescue` (= `root@15.204.108.57`). Legacy fleet
     box — coordinate with the legacy `finitecomputer` repo (workspace
     `ovh-fc-1`); same fix-forward caveat as smoke. `finitechat-server`
     here is **DISABLED** — chat is single-writer (§4) and the live writer
     is the app-plane host; do NOT re-enable it. `journalctl -u
     fc-offsite-backup` (legacy borg) is the relevant log.

4. Escalate before any production mutation (§6).

## 2. Chat availability incident ("chats appear missing")

Treat an empty or smaller sidebar as an availability incident. Do not create
a Room, run `/fresh`, delete state, rewrite selection, or mutate production
while diagnosing.

1. Keep the user on the current Hosted Web Device. Record only count totals
   and purpose-scoped pseudonyms; do not paste live ids, email, message
   bodies, or attachment contents into logs or tickets.
2. Read service health and recent errors:

   ```sh
   systemctl status finitechat-hosted-device finitechat-server
   journalctl -u finitechat-hosted-device -u finitechat-server --since -30min
   ```

3. Confirm the Hosted Web Device identity file and `client.sqlite3` either
   both exist or both do not. A partial pair is recovery-required and must
   not mint a replacement identity.
4. Through the authenticated dashboard, use `Retry load`. A valid binding
   must open retained history without Agent Runtime contact and without
   rewriting the binding. `Retry load` cannot authorize a missing binding.
   `Retry claim` is only for mutation authority and must not gate reading.
5. If the Agent replies in only some retained Rooms, inspect the generated
   Hermes platform configuration read-only. The normal adapter serves every
   Room joined by the Agent Device; an explicit `extra.room_id` or
   `FINITECHAT_ROOM_ID` is the only supported Room filter. Record only
   whether a filter exists, not its live value. A Hermes home-channel is a
   routing preference and is not a subscription filter.
6. Compare count-only Room, Topic, and Chat totals in the client store with
   the canonical plus `Previous conversations` projection. Any
   retained-versus-visible mismatch blocks release/admission.
7. If the binding is missing, corrupt, or membership-invalid, do not choose
   a Room from `selected_room_id`, display position, timestamps, identifier
   order, or the fact that only one candidate is currently visible. There
   is no automatic Room reconciliation or legacy binding migration. A
   corrupt or invalid existing binding is immutable to ordinary product
   flows; stop and use the separately authorized recovery workflow
   ([recovery.md](recovery.md)).
8. A missing binding may show the user `Finish chat setup` only for the
   narrow case where Core committed creation but the dashboard lost the
   authorization response. Ordinary load must not invoke it automatically.
   If the user invokes it, the product must use a fresh Core read and
   require the exact Account-owned Project plus exactly one durable creation
   request in `requested`, `launching`, or `running` state before writing
   the omitted bootstrap authorization. This action never scans or chooses
   Rooms; retained candidate Room state still fails closed. If the action
   is absent or refuses the state, stop rather than manufacture an
   authorization.
9. Reproduce the symptom without mutating the affected state, then
   reproduce it with synthetic state before proposing a repair. Record
   which observations support the cause and which explanations remain
   hypotheses.
10. Escalate before any production migration, repair, restore, service
    restart, deploy, or traffic change. Obtain explicit authorization and
    name the preserved backup, rollback procedure, and stop boundary first.

The implementation invariant, proved with synthetic state rather than by
decrypting a production journal during diagnosis, is that a legitimately
authorized first bootstrap seals the exact Room create request, including
its intended Room id and MLS group id, before any server mutation; seals the
claimed Agent KeyPackage before Room creation and the exact prepared
add-member commit before submit; and replays the exact journaled request
and group id on restart. Ordinary durable protocol sync may process
already-authorized membership and messages after a reconnect — do not
disable or describe that convergence as a migration or recovery repair.

**Resolution requires the same retained conversations to be reachable
through the product. A green timer, database integrity check, or
confirmation that rows exist is not resolution.**

## 3. Billing incident

One live `Finite Computer Hosted Agent` subscription path. **Stripe owns
payment state; Core owns entitlement state.** Never repair a billing
incident by editing Core's database, changing a Subscription to match a
selected row, or deleting a Customer, invoice, runtime, or recovery
material.

- **Safety boundary:** before any mutation, record the deployed
  revision/system closure and switch the dashboard to
  `FC_DASHBOARD_RUNTIME_MODE = "canary"` if new Checkouts must stop.
  Existing Stripe and Core records remain evidence, not rollback debris.
- **Evidence hygiene:** Stripe request/event ids, Customer/Subscription ids,
  Core organization id, Price id, timestamps, status, and the deployed
  closure may be recorded. Customer email, payment details, webhook bodies,
  API keys, signing secrets, and database credentials never enter tickets,
  logs copied off-host, or this public repository.
- **Readiness audit (read-only):** temporary live restricted key (Account,
  Product, Price, Tax settings, Portal config, Event Destination reads) →

  ```sh
  STRIPE_READINESS_SECRET_KEY='<temporary-rk_live>' \
  STRIPE_EXPECTED_ACCOUNT_ID='<approved-acct-id>' \
  STRIPE_EXPECTED_PRICE_ID='price_1TsqWWA50jhCdjMEhQLEBpvR' \
    npm --prefix finitecomputer-v2/apps/dashboard run stripe:readiness
  ```

  The report contains no secret or customer values; expire the key after
  the run. A failure keeps Checkout dark.
- **Paid but not synchronized:** read the Checkout Session, current
  Subscription, and event delivery in Stripe; confirm Customer, live Price,
  and `finite_customer_org_id` metadata agree (never infer identity from
  email). Read the dashboard journal and Core's authenticated billing
  overview — do not print environment files. If delivery is pending, wait;
  if failed, fix the proven cause and use Stripe's redelivery of the
  original event (the handler fetches current state and Core rejects stale
  ordering — never synthesize an event or update a row). If payment
  succeeded but metadata/Customer/Product/Price differ: stop, keep
  admission dark, escalate for a reviewed refund or reconciliation.
- **Duplicate/out-of-order delivery:** repeated and stale events should be
  harmless; if either changes entitlement incorrectly, stop Checkouts,
  preserve both event ids, roll back the application revision. Never delete
  either event or the Subscription.
- **Wrong Price:** the only accepted live Price is
  `price_1TsqWWA50jhCdjMEhQLEBpvR`, configured identically in Dashboard and
  Core. A Subscription containing another Price must not grant entitlement —
  stop new Checkouts, capture ids, correct the reviewed deployment or
  Stripe setup. Refund/cancellation is a separate owner-approved action.
- **Past due / cancellation / refunds:** Smart Retries and Stripe's
  failed-payment email own renewal recovery; Core blocks new creation while
  preserving an existing runtime and its data. Portal cancellation is at
  period end (`cancel_at_period_end=true`) — never translate it into
  runtime teardown. Refunds are manual customer-support decisions in
  Stripe; a refund is not a Core mutation and does not authorize retirement
  or purge. Disputes are handled in Stripe with evidence in the approved
  private support system; a dispute never authorizes compute or
  recovery-data deletion.
- **Key or webhook-secret rotation:** keep Checkout dark and name the
  current closure as rollback; create the replacement restricted key with
  the same scope, transfer it directly to root-owned
  `/etc/finite/dashboard.env` (never print either value); restart only
  `podman-finite-saas-dashboard.service`, verify health, run the readiness
  audit; revoke the old key only after the replacement succeeds. For a
  webhook secret, coordinate so one accepted secret covers every delivery,
  otherwise keep Checkout dark and redeliver failed original events after
  the application is ready.

## 4. App-plane host failure

The pattern below is the ADR 0007 emergency, written as the repeatable
procedure. The worked example is
[lat2-replacement-cutover.md](lat2-replacement-cutover.md) (retained until
its Gate E closes; its 2026-08-27/28 record is the executed instance).

1. **Preserve the dead host.** Rescue mode, disks read-only, IPMI for
   diagnosis. Nothing on it may resume writing — a second writer on
   Postgres/chat/sites is unrecoverable split-brain, worse than the outage.
2. **Bank state off-box** (read-only pulls from rescue): the coordinated
   recovery snapshot (verify its manifest sha256 and SQLite integrity
   through `scripts/snapshot-sqlite` on scratch copies), Postgres dumps,
   live chat/brain SQLite, sites/hosted-device/identity/core trees, runner
   state, and the complete secret set per the host's
   `secret-bootstrap-contract.json`. Litestream (seconds RPO) and Borg sit
   behind these as additional lanes.
3. **Wipe and reinstall the replacement from a CI closure artifact** in
   declarative import mode (artifact-driven nixos-anywhere; kexec from the
   artifact; nothing built on the Mac, the target, or rescue). Capture the
   replacement's storage identity before the wipe; the config lands in
   `infra/nixos/hosts/<host>/` (`storage-ids.nix` captured, networking
   filled) and merges to `main` before the artifact is built.
4. **Restore per the authority order** (owner decision at cutover; ADR 0007
   chose: Postgres from the latest coordinated dump; chat + brain from
   litestream; file trees from the banked pulls) — mechanics and offline
   verifies in [recovery.md](recovery.md).
5. **Go live via the closure, not systemd:** the go-live PR flips
   `finite.importMode.enable = false`; deploy the go-live closure through
   the fenced flow (dry activation reviewed first; `--expect-startup` is
   the one import→product exception).
6. **DNS flips last**, owner-performed in the provider consoles, per exact
   record (Namecheap app names; Cloudflare origin for the proxied zone),
   TTLs checked first. ACME re-issues follow the flip — do not verify certs
   against unpropagated DNS (a failed HTTP-01 challenge makes Caddy back
   off; `systemctl restart caddy` retries after propagation).
7. **Single-writer doctrine for the chat move** — the rule that governs
   every chat move, forever: the chat protocol depends on the server being
   **one ordered log**; there must never be two servers able to accept
   writes for the same database, and the server must never "half-accept"
   traffic during a move. **Fail closed: if chat has to go down, it goes
   DOWN** — connection refused is correct; split state is unrecoverable.
   Exact order:
   1. `systemctl disable --now finitechat-server` on the OLD host —
      disable, not just stop, so nothing (reboot, reconcile loop) can
      resurrect a second writer. Verify the port no longer answers.
   2. Checkpoint the WAL
      (`sqlite3 server.sqlite3 "PRAGMA wal_checkpoint(TRUNCATE);"`) and
      copy the database ONLY after step 1.
   3. Start the NEW server; verify via direct IP
      (`curl --resolve chat.finite.computer:443:<new-ip>
      https://chat.finite.computer/health`) — contract version + source
      fingerprint + `source_dirty:false`.
   4. Only then flip DNS. During the TTL window, clients cached on the old
      IP get connection refused — a clean outage, by design.
   5. Rollback inverts the discipline: stop+disable the NEW server BEFORE
      re-enabling the old one, and carry the database back (writes the new
      server accepted must move with it or be consciously discarded).
8. **Post-cutover cleanup:** unregister the replacement's old GitHub
   runners, rotate the legacy credentials that rode the dead host (Core
   runner token, Resend key, tunnel key), keep the dead host powered off,
   and flip the WireGuard peers to the new hub. Record everything in the
   changelog.

## 5. Runner host rebuild

For a dead or new Kata runner host. The worked example is
[lat4-nixos-runner-install.md](lat4-nixos-runner-install.md) (retained until
its Gate F closes).

1. **Preflight:** SMART health on every NVMe (`smartctl -H`), all PASSED.
   Re-run the read-only storage capture and diff against the recorded
   evidence — any drift stops the gate until the repo is corrected. Confirm
   the provider IP is static and rescue mode is available. Prove the box
   holds nothing worth preserving.
2. **Config + artifact:** host directory in `infra/nixos/hosts/<host>/`
   with `storage-ids.nix` (`captured = true`) and networking filled from
   the capture; merge to `main`; dispatch the host's closure workflow at
   the merged rev; the artifact's manifest carries system + disko + kexec
   paths.
3. **Install:** provider rescue mode; drive the install from a Linux driver
   machine (or CI) with the artifact driver — it validates the manifest,
   realizes SYSTEM/DISKO/KEXEC by substitution only, and invokes the
   flake-pinned nixos-anywhere. Reboot into NixOS; verify
   `/run/current-system` is the artifact's SYSTEM path, both RAID1 arrays
   `[UU]`, the storage health unit clean, both ESPs mounted, and the
   filesystem identity file matching `storage-ids.nix`.
4. **Secrets + overlay + drained bring-up:** `/etc/finite/runner.env`
   (0600, **drained**, current promoted `FC_RUNNER_RUNTIME_ARTIFACT_ID`),
   `identity-operator.env` (replaceable operator token),
   `runtime-secrets.env`, `wireguard-private-key`, monitoring write envs.
   Confirm the WireGuard handshake with the app-plane hub and that Core
   answers over the overlay. Register the host's Core keyring entry and
   prove one authenticated, draining runner cycle (zero sandboxes).
   Exercise the storage drills (degraded-array refusal, ESP-guard refusal,
   rollback boundary).
5. **Admission is a separate owner decision:** flip `FC_RUNNER_DRAIN=false`,
   verify capacity and creation beside the existing runner, record the
   decision on the owning ADR.

Hard-won facts baked into the configs (do not "fix" these):

- Disks are addressed by `/dev/disk/by-id` (serial-stable) so installer
  kernel enumeration order can never mismatch them.
- WAN is bound by MAC, not interface name (systemd-networkd
  `matchConfig.MACAddress`) — NIC names change between installer and OS
  kernels; MAC-match is immune.
- Dual ESPs behind a fail-closed ESP-write guard; exact-geometry RAID1 for
  root and `/data`.
- The lat1 single-disk lesson: its mdadm RAID1 superblocks were
  unassemblable on the pinned kernel (array size overran the 129 MiB data
  offset; `md_import_device returned -22` on every member) — root + `/data`
  shipped single-disk and backups were the only redundancy. Lat2/lat3/lat4
  use the storage-qualified geometry instead; never reuse disks carrying
  stale mdadm metadata without a destructive-authorization wipe.

## 6. Escalation & authorization

The following never happen without the owner's fresh, explicit approval:

- wiping or reinstalling any host; booting a dead host back into service;
- DNS record changes;
- undraining a runner host or admitting new capacity;
- refunds, Subscription cancellation, or Checkout re-enablement after a
  billing stop;
- any restore into production, Core database repair, or snapshot unseal;
- crossing a one-way binary boundary backward (Finite Private epochs).

AGENTS.md production-repair rules apply to all of them: gather read-only
evidence, reproduce the failure, prove the change on synthetic state, and
name the backup and rollback boundary before mutating. A selected row, sort
order, identifier order, or other navigation state never confers authority
to choose or rewrite durable user state; ambiguous state fails closed
without mutation.
