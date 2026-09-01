# finite-lat-2 replacement cutover (emergency, lat1 thermal failure)

Decision authority: [ADR 0007](../../docs/adr/0007-finite-lat-2-emergency-app-plane-cutover.md). Every gate that mutates production needs Paul's fresh approval.
Steps marked `TODO(prove)` have not been exercised yet — prove them at
execution, never reconstruct afterward.

## PRECONDITIONS

- The lat1 state pull is complete and verified on the operator Mac
  (snapshot manifest sha256, `scripts/snapshot-sqlite integrity-check` on
  the live DB copies, scratch restore of the coordinated snapshot).
- The banked secret set covers every file in
  `infra/nixos/hosts/finite-lat-1/secret-bootstrap-contract.json`
  EXCEPT `/etc/finite/runner.env` and `/etc/finite/phala-runner.env`
  (runner lane, belongs to the future runner host).
- `scripts/finite-status` state accepted (lat1 down = red; lat3 runner
  unreachable = expected until Gate E).
- The `Lat2 NixOS Closure` workflow exists (this PR) and CI is green.

## STEPS

### Gate A — wipe lat2, capture its storage identity

1. Owner decision to wipe recorded (skip the stale-data archive; lat1's
   data is the authority).
2. BEFORE the wipe: boot Latitude rescue mode and run the read-only
   capture (rescue SSH is `root@64.34.80.19` with the one-time password —
   install the operator key first as in the lat1 evacuation):

   ```sh
   infra/nixos/scripts/capture-lat2-host-evidence root@64.34.80.19
   ```

   Record WAN/LAN NIC MACs, IPv6 address, and gateways from the capture or
   the provider console.
3. Trigger the provider reinstall (wipes all four NVMe devices), then back
   into rescue mode for the install.

VERIFY: capture table shows the expected 2 small + 2 large NVMe shape and
both pairs fit the pinned geometry.

### Gate B — fill the config, build the artifact

1. Review `target/lat2-host-evidence-*/storage-ids.nix.skeleton`: assign
   root-a/root-b/data-a/data-b, confirm geometry, then copy into
   `infra/nixos/hosts/finite-lat-2/storage-ids.nix` (`captured = true`) and
   fill the four `REPLACE-ME` networking placeholders in
   `infra/nixos/hosts/finite-lat-2/default.nix`.
2. PR, merge to `main`, dispatch `Lat2 NixOS Closure` at the merged rev,
   download `lat2-nixos-closure-<REV>`, and `just lat2-runner-rollout-contract`.

VERIFY: artifact manifest carries system + disko + kexec paths for the
merged rev; `.#nixosConfigurations.finite-lat-2` evals green with
`finite.importMode.enable = true`.

### Gate C — install (import-mode closure)

1. From the artifact directory, drive the install with the installer
   helper. It validates the manifest, realizes SYSTEM/DISKO/KEXEC from the
   artifact cache (substitution only — nothing builds), and invokes the
   pinned nixos-anywhere:

   ```sh
   scripts/install-lat2-from-artifact <artifact-dir> root@64.34.80.19
   ```

   Run this from a Linux driver machine or CI job that can realize
   x86_64-linux store paths — `nix copy` from the artifact cache fails
   honestly anywhere else. The kexec tarball is the same-pin NixOS 26.05
   installer built by the CI workflow; do not let nixos-anywhere substitute
   its default. TODO(prove): exact nixos-anywhere behavior is proven at
   first execution; the invariants above are what must hold.

2. Reboot into NixOS. `ssh root@64.34.80.19` (new host key).

VERIFY (import-mode boot): `readlink -f /run/current-system` is the
artifact's SYSTEM path; `/proc/mdstat` shows both arrays `[UU]` idle; the
storage health unit exits clean; `/boot-a` and `/boot-b` mounted;
`systemctl list-units --state=running` shows NO product units (import
mode), while sshd, postgres, node-exporter are up; postgres created a
fresh empty cluster.

### Gate D — import lat1's state

Place secrets (file-to-file from the banked pull; names/modes per
`secret-bootstrap-contract.json`; skip `runner.env` / `phala-runner.env`;
install the staged lat2 WireGuard private key at
`/etc/finite/wireguard-private-key` 0600; the monitoring-write env files
and `litestream-latitude.env` from escrow):

1. **Postgres** (authoritative source: latest coordinated dump, 18:35 UTC):

   ```sh
   # role password must match FC_CORE_DATABASE_URL in /etc/finite/core.env
   sudo -u postgres psql -c "CREATE ROLE finite LOGIN PASSWORD '<from-core.env>'"
   sudo -u postgres createdb -O finite finite_core
   pg_restore --no-owner --role=finite -d finite_core <dump>.dump
   sudo -u postgres psql -d finite_core -tAc \
     "SELECT count(*) FROM finite_private_api_keys;"   # must print 87
   ```

2. **Chat + Brain** (authoritative source: litestream restore from
   `finite-lat-1-litestream` using the banked Latitude object-storage
   credential; the drilled procedure lives in
   [`recovery.md`](recovery.md)):

   ```sh
   # TODO(prove): scratch restore config pointing at the lat1 bucket +
   # endpoint; restore both dbs to /data/staging, then:
   sqlite3 <restored>/server.sqlite3 'PRAGMA integrity_check;'   # ok
   sqlite3 <restored>/finite-brain.sqlite3 'PRAGMA integrity_check;'  # ok
   ```

   Cross-check row counts against the pulled live DB copies, then move both
   into place under `/var/lib/private/finite-chat/data/` and
   `/var/lib/private/finitebrain/` with the ownership the units' DynamicUser
   assigns (start each unit once against an empty dir, stop it, note the
   uid, restore files, chown). TODO(prove): the exact ownership handoff.

3. **File trees** from the banked pull, rsynced to their exact paths:
   `/var/lib/finite-sites` (1.7G), `/var/lib/private/finitechat-hosted-device`
   (1.2G), `/var/lib/private/finite-identity`, and Core's
   `/var/lib/private/finite-saas-core` relay state. Ownership same pattern
   as above.

VERIFY (offline, all loopback): the 87-key invariant; role password
authenticates; chat + brain integrity ok with expected row counts; sites
registry opens; identity store present; `postgres` answers; no product
unit has ever started.

### Gate E — go live

1. Go-live PR: `finite.importMode.enable = false` in
   `infra/nixos/hosts/finite-lat-2/default.nix`; merge; build the go-live
   `lat2-nixos-closure-<REV2>` artifact; `just deploy-lat2-closure
   ARTIFACT_DIR --prepare` then `--activate --expect-startup` (fenced flow,
   dry-activation reviewed first). `--expect-startup` tells the fence this
   is the one import-mode → product startup: the dry activation may name
   only the declared app-plane unit set, and the core product units must be
   active after the switch. Steady-state deploys later omit the flag — any
   product-unit start then is a refusal. The WG peer flip for lat3 (already
   merged on main) rides the next `Lat3 NixOS Closure` deploy — after it,
   lat3's runner reaches Core through lat2.
2. Full verification on lat2 (from the historical cutover checklist):
   Core health on loopback via the private proxy, `finite.computer` vhost
   via `curl --resolve`, chat `/health` + hosted-device, brain health,
   sites vhost, identity `nostr.json`, dashboard login page, 87-key
   invariant again under load.
3. DNS flip (owner, in consoles): Namecheap `finite.computer`,
   `chat.finite.computer`, `brain.finite.computer`, and
   `identity.finite.vip` A records → `64.34.80.19` (check TTLs first);
   Cloudflare `*.finite.chat`/`api.finite.chat` origin → same IP. ACME
   re-issues for the Namecheap names within minutes; `*.finite.chat`
   serves the banked CF Origin pair immediately.
4. `scripts/finite-status` before/after; post-cutover cleanup ticket:
   unregister the 4 old-lat2 GitHub runners, rotate the legacy Core runner
   token/Resend key, keep lat1 powered off.

VERIFY: every product name serves 200 from lat2; runners show current
handshakes to the new hub; finite-status green.

## CHECKPOINTS

| Date | Event | Result |
|---|---|---|
| 2026-08-27 | lat1 thermal failure; rescue mode; state pull started | — |
| 2026-08-28 | ADR 0007 + replacement scaffold PR | this PR |

## ROLLBACK

- Before Gate C's wipe there is no host rollback; the wipe boundary is
  explicit and owner-approved.
- After install: NixOS generations + `nixos-rebuild switch --rollback`;
  the rescue-mode reinstall can be re-run from the same artifact.
- Import mistakes: re-run the affected restore step; nothing product-side
  has started, so there is no live state to corrupt.
- After go-live: `just deploy-lat2-closure ARTIFACT_DIR` with the previous
  artifact, or `nixos-rebuild switch --rollback` on the host; DNS rollback
  is the record flip back to lat1 (dead) — treat DNS rollback as break-glass
  only.
- lat1 stays powered off throughout; resurrecting it is a separate
  owner decision that must first rule out split-brain.
