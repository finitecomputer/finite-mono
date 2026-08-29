# finite-lat-4 NixOS Runner install (twin of finite-lat-3)

Pattern authority: ADR 0007 as amended by the emergency cutover (PR #715):
lat1 is down, lat2 is the replacement app-plane host and WireGuard hub at
10.254.3.1, and the runner lane moves to this host — finite-lat-4 is the
second storage-qualified Runner host. Lat4-specific evidence, decisions, and
open items:
[`docs/runs/lat4-provisioning-prep.md`](../../docs/runs/lat4-provisioning-prep.md).
This runbook is the execution surface. Every gate that mutates production
requires the owner's fresh approval per the deployment-queue discipline. The
wipe/install steps are `TODO:`-marked until first exercised: this procedure
has not been run yet, and it must be proven before or at the moment of its
first execution, not reconstructed after.

## PRECONDITIONS

- `scripts/finite-status` is green, or the incident owner has accepted why it
  is not. lat4 must not become a deploy or recovery target mid-incident.
- The current lat1 backups and the hosted recovery set are independently
  verified. lat4's own history is never a recovery authority.
- PR #715 has merged AND lat2 is live as the app-plane replacement: lat2
  carries the `wg-finite` hub at `10.254.3.1` with the Core socket proxy and
  Identity Authority proxy. Gates D and E wait on that go-live; lat4's
  overlay path does not exist until the hub is up.
- lat4 holds no user data: it is a fresh provider box with a throwaway
  interim Ubuntu install (verified read-only 2026-08-28; only an empty md0/md1
  resync and OS defaults). Nothing on it needs archiving.
- Operator SSH keys are the three keys already declared in
  `infra/nixos/hosts/finite-lat-4/default.nix`; SSH access works as
  `ubuntu@152.236.34.15` with passwordless sudo.
- A Linux driver machine (or CI job) holds the downloaded CI artifact. The
  closure, disko script, and kexec tarball are consumed from the artifact
  only; nothing is built on the Mac, on a prod box, or in rescue mode.

## STEPS

### Gate A — pre-wipe verification of the interim host

1. SMART health for all four NVMe devices on the still-running interim OS
   (smartmontools is not preinstalled; run from a NixOS kexec image, a rescue
   image, or install it on the interim OS first):

   ```sh
   for d in /dev/nvme0n1 /dev/nvme1n1 /dev/nvme2n1 /dev/nvme3n1; do
     smartctl -H "$d"
   done
   ```

   All four must report PASSED before the wipe is approved.
2. Re-run the read-only capture and diff it against the recorded evidence:

   ```sh
   infra/nixos/scripts/capture-lat4-host-evidence finite-lat-4
   ```

   Disk by-id paths, sizes, NIC MACs, and addressing must match
   `docs/runs/lat4-provisioning-prep.md` §1 and
   `infra/nixos/hosts/finite-lat-4/storage-ids.nix`. Any drift stops the
   gate until the repo is corrected.
3. Confirm in the provider console that 152.236.34.15 is a static assignment
   (not DHCP-by-default) and that rescue mode is available; note the BMC USB
   NIC (`enxbe3af2b6059f`) stays unconfigured.
4. Final sweep that the box holds nothing of value (no user home data, no
   service state beyond the interim OS defaults).

VERIFY: four SMART PASSED; capture diff clean; provider console confirmed;
no data worth preserving.

### Gate B — verify the captured config and build the artifact

1. The `storage-ids.nix` identities and the networking block in
   `infra/nixos/hosts/finite-lat-4/default.nix` are already filled from the
   2026-08-28 capture (see the prep record). Re-verify them against the
   fresh Gate A capture; this PR's host directory must be merged to `main`
   before the artifact is built.
2. Dispatch the `Lat4 NixOS Closure` workflow for the exact merged rev:

   ```sh
   gh workflow run lat4-nixos-closure.yml --ref <REV> \
     -f rev=<REV>
   gh run download --name lat4-nixos-closure-<REV>
   just lat4-runner-rollout-contract
   ```

   The build script refuses to package a host whose `storage-ids.nix` still
   says `captured = false`; the artifact's manifest carries the system,
   disko, and kexec store paths (`finite.lat4.nixos-closure.v2`).
3. Set the WG underlay before install: lat2 — the overlay hub — carries the
   lat4 peer (public key matching the operator-staged lat4 private key) with
   `allowedIPs 10.254.3.4/32` and the peer-scoped firewall rules mirroring
   lat3's; deploy via lat2's closure flow and confirm the lat3 handshake is
   still current. This PR deliberately does NOT carry that lat2 edit: it can
   only land after #715 merges and lat2 is live, as a small follow-up commit
   on this branch before Gate B executes.

VERIFY: artifact manifest matches the merged rev; `nixosConfigurations`
evals green; lat2 deployed with the lat4 peer and its existing handshakes
current.

### Gate C — wipe and install (provider console + artifact install)

1. Provider console: launch Rescue Mode for finite-lat-4; note the one-time
   root password. The rescue environment must show all four NVMe devices
   attached (compare against the Gate B capture).
2. `ssh root@152.236.34.15` (accept the new host key; this is rescue Ubuntu).
   TODO(prove): confirm the rescue kernel exposes all four disks and that
   the disko script's by-id paths resolve.
3. From a Linux driver machine (or CI job) that can realize x86_64-linux
   store paths, drive the whole install with the artifact driver — it
   validates the manifest, proves the rev is on origin/main, realizes the
   SYSTEM/DISKO/KEXEC paths from the artifact's file cache (pure
   substitution, nothing built), and invokes the flake-pinned
   nixos-anywhere:

   ```sh
   # TODO(prove): exact nixos-anywhere flag spellings are proven here on
   # first execution; the invariants are: kexec from the artifact, store
   # paths from the artifact, no build fallback.
   scripts/install-lat4-from-artifact target/lat4-nixos-closure-<REV> \
     root@152.236.34.15
   ```

   The kexec tarball is the same-pin NixOS 26.05 installer built by the CI
   workflow; do not let nixos-anywhere substitute its default.
4. Reboot into NixOS. `ssh root@152.236.34.15` (new host key again).

VERIFY: `readlink -f /run/current-system` is the artifact's SYSTEM path;
`cat /proc/mdstat` shows both arrays `[UU]` and idle; the storage health
unit exits clean; both ESPs mount at `/boot-a` and `/boot-b`; the storage
identity file `/data/.finite-filesystem-identity` matches
`storage-ids.nix`.

### Gate D — secrets, overlay, drained bring-up

1. Place host-only secrets (values transferred file-to-file from lat1/lat3
   custody, never via the repo; names and modes only here):
   - `/etc/finite/runner.env` (0600) from
     `infra/nixos/hosts/finite-lat-4/runner.env.example` with
     `FC_RUNNER_DRAIN=true` and the current promoted
     `FC_RUNNER_RUNTIME_ARTIFACT_ID` read from lat3's
     `/etc/finite/runner.env`.
   - `/etc/finite/identity-operator.env` (0600) — the replaceable operator
     token per `infra/runbooks/identity-authority.md`.
   - `/etc/finite/runtime-secrets.env` (0600) — direct copy of lat3's.
   - `/etc/finite/wireguard-private-key` (0600) — the private half of the
     keypair whose public key is already registered on the lat2 hub.
   - `/etc/finite/metrics-remote-write.env` and `/etc/finite/logs-write.env`
     (0600) — monitoring write credentials; the activation preflight
     `check-lat-monitoring-secrets` blocks activation without them.
2. WireGuard: confirm the handshake with the lat2 hub (`wg show` on both
   ends; the endpoint is `64.34.80.19:51820`, local `10.254.3.4`). From
   lat4, the Core URL from `runner.env` must answer over the overlay (an
   HTTP response, not a timeout) — Core and the Identity proxy now live on
   lat2 at `10.254.3.1`. lat4's firewall exposes exactly 22/tcp,
   51820/udp, and the Kata contact range on `wg-finite` only.
3. Core credential registration BEFORE the drained first lease: the
   off-host-escrowed `finite-kata-runner-4` keyring entry lands in
   `/etc/finite/core.env` on lat2 (where Core now runs) and Core restarts
   in the platform-rollout window. Registration is not admission — it only
   lets the Runner authenticate while still drained. Then trigger one
   Runner cycle and confirm an authenticated, draining response — no
   creation is offered while `FC_RUNNER_DRAIN=true`:

   ```sh
   systemctl start finite-saas-runner.service
   journalctl -u finite-saas-runner.service -n 100
   ```

   Expect a successful authentication with drain/capacity-unavailable
   semantics, zero sandboxes created.
4. Storage drills mirroring lat3's qualification: degraded-array and
   rebuild refusal behavior of the health unit, ESP-guard refusal on a
   mismatched mount, and a `nixos-rebuild switch --rollback` boundary
   proof.
5. Run `scripts/finite-status` (before/after ritual) and confirm lat4
   appears from the contract profile with storage green and drain true.

VERIFY: finite-status shows finite-lat-4 green and drained; Grafana shows
`finite-lat-4` metrics/logs with `role="runner"`; the deployment changelog
records Gate D completion.

### Gate E — admission decision (separate, later, owner-approved)

Not part of the install. Core already carries the `finite-kata-runner-4`
credential from Gate D. If taken: flip `FC_RUNNER_DRAIN=false` in lat4's
`runner.env`, verify lease capacity reports 42 and creation works beside
lat3, and record the admission decision as a status note on ADR 0007. Until
this gate, exactly one Runner accepts new creation (ADR 0005, retained).

### Gate F — fleet adoption: import lat1's agent runner state

Ownership: Paul produces and ships the archives; the lat4 operator verifies,
imports, and relocates each Runtime through the exact relocation contract.
Transport: scp directly to lat4 after Gate C (no dumbpipe staging hop). Every
mutation below needs fresh owner approval; the archives are the only copy of
this state, so verification precedes every step. lat1's source state stays
stopped-and-intact until each observation window passes.

Inbound archives (provenance in `docs/runs/lat4-provisioning-prep.md` §7):

| Archive | Exact size | sha256 (archive-level) |
|---|---:|---|
| `finite-saas-runner.tar.zst` | 20,089,881,363 B | `a839a582eb5b48fee08a8971c4d9d4cb585adedc3e715b8cd94ef9699d7fe4ba` |
| `data-finite-saas-runner.tar.zst` (newer lat1 `/data/finite-saas-runner/kata` state) | 8,189,970,726 B | `df451a5ce610099ad137c9489e12150025f9cfc5fda610c166fd4b6ef140a17d` |
| `runtime-recovery-archive.tar.zst` (Recovery Set, Aug 26 op) | 8,193,943,598 B | `6c3c1e5312833765e4ec049bd8680e8cd33bd07df5008b939496851a0db5176b` |
| `manifests-bundle.tar` | 50,765,312 B | `bf9a111956b66db99b675b5bcb38b0da8f5e99d9f40010900ed143740303972e` |

1. Copy the archives plus `runner-files.sha256` / `runner-files.sizes`
   (154,299 lines, paths relative to `finite-saas-runner/`) and
   `data-kata-files.sha256` / `data-kata-files.sizes` (7,065 lines including
   the zero-byte dumbpipe `./.done` transport marker) to `/data/staging/` on
   lat4. `finite-prepare-data-root` creates that directory as `root:root
   0700`. An archive-mode rsync from macOS can replace the destination
   directory metadata with the Mac UID and mode; restore the root-only
   boundary after transport and before inspection:

   ```sh
   sudo chown root:root /data/staging
   sudo chmod 0700 /data/staging
   sudo find /data/staging -maxdepth 1 -type f \
     -exec chown root:root {} + -exec chmod 0600 {} +
   ```
2. Verify before extracting:

   ```sh
   sudo sha256sum -c <<'HASHES'
   a839a582eb5b48fee08a8971c4d9d4cb585adedc3e715b8cd94ef9699d7fe4ba  /data/staging/finite-saas-runner.tar.zst
   df451a5ce610099ad137c9489e12150025f9cfc5fda610c166fd4b6ef140a17d  /data/staging/data-finite-saas-runner.tar.zst
   6c3c1e5312833765e4ec049bd8680e8cd33bd07df5008b939496851a0db5176b  /data/staging/runtime-recovery-archive.tar.zst
   bf9a111956b66db99b675b5bcb38b0da8f5e99d9f40010900ed143740303972e  /data/staging/manifests-bundle.tar
   HASHES
   ```

3. Extract into the declared work root. lat4's `workRoot` is
   `/data/finite-saas-runner` (lat3-style, on the 1.8T data array — NOT
   lat1's `/var/lib` path); the archive root `finite-saas-runner/` lands
   there directly, confirmed acceptable for the Core re-point. The delivered
   tarballs contain non-authoritative macOS AppleDouble `._*` records and the
   `futurepaul/staff` owner. Neither appears in the
   authoritative manifests. Exclude the metadata and use lat3's proven
   `root:root` ownership instead of importing a foreign UID/GID:

   ```sh
   sudo tar --warning=no-unknown-keyword --no-same-owner \
     --exclude='._*' --exclude='*/._*' \
     -I zstd -xf /data/staging/finite-saas-runner.tar.zst -C /data
   sudo chown root:root /data/finite-saas-runner
   sudo chmod 0700 /data/finite-saas-runner
   ```

4. Per-file integrity over the whole imported tree (all 154,299 lines must
   pass):

   ```sh
   sudo sh -c 'cd /data/finite-saas-runner && sha256sum -c --quiet /data/staging/runner-files.sha256'
   ```

5. Overlay the newer kata capture second. It contains the same 7,064 payload
   paths for `runtime_95aa8aa8937bf5c32922` as the main archive: 7,040 hashes
   agree and 24 are newer/different. Extracting in the reverse order silently
   restores the older state. The data manifest's extra `./.done` line is a
   dumbpipe completion marker, not an archive member; do not fabricate it.

   ```sh
   sudo tar --warning=no-unknown-keyword --no-same-owner \
     --exclude='._*' --exclude='*/._*' \
     -I zstd -xf /data/staging/data-finite-saas-runner.tar.zst -C /data
   sudo chown root:root /data/finite-saas-runner
   sudo chmod 0700 /data/finite-saas-runner

   sudo sh -c '
     cd /data
     awk "substr(\$0,67) != \"./.done\"" \
       /data/staging/data-kata-files.sha256 |
       sha256sum -c --quiet -
   '
   sudo sh -c '
     cd /data/finite-saas-runner
     awk "substr(\$0,67) !~ /^\\.\\/kata\\/runtime_95aa8aa8937bf5c32922\\//" \
       /data/staging/runner-files.sha256 |
       sha256sum -c --quiet -
   '
   ```

   This proves the final 154,299-file tree as 147,235 unchanged main-capture
   files plus 7,064 newer overlay files. Keep
   `runtime-recovery-archive.tar.zst` hash-verified and root-only in
   `/data/staging/`; it is an independent Recovery Set, not a third live-tree
   overlay. Extract it only after Paul names a separate non-canonical target.
6. Restore gate (per
   `docs/runs/finite-lat-capacity-and-redundancy.md`): writer stopped (the
   Runner is drained and `ConditionPathExists`-gated), no containerd task
   for these Runtimes, manifests stable, and the target was empty (fresh
   Gate C install). For any SQLite-backed state use the scratch-copy rule —
   never open a snapshot SQLite file directly: verify through
   `scripts/snapshot-sqlite` or a copy, requiring `PRAGMA integrity_check`
   clean and identity-hash equality against the archive's recorded values.
7. Enumerate the exact migrated Runtime set BEFORE any binding change. Two
   independent lists must agree exactly — any discrepancy fails closed:

   - **From the imported tree:** the durable directories under
     `/data/finite-saas-runner/kata/` (each named by a `DURABLE_STATE_ID`),
     plus Paul's per-runtime export records carrying the
     `runtime_relocation.v1` field set: `PROJECT_ID`, `RUNTIME_ID`,
     `SOURCE_HOST_ID` (`finite-lat-1`), `SOURCE_MACHINE_ID`,
     `DURABLE_STATE_ID`, `RUNTIME_ARTIFACT_ID`, `STATE_SCHEMA_VERSION`,
     `EXPECTED_AGENT_NPUB`, and the export-time `state-manifest` sha256.
   - **From Core's records:** a read-only export of every Runtime currently
     bound to `finite-lat-1` (Runtime ID, machine ID, artifact, schema,
     Principal).

   Generate `migrated-runtimes.manifest` (one record per Runtime) from
   their intersection. A broad `source_host_id` selection alone is NOT the
   migration set: every Runtime in the set must carry a full record, and
   Runtimes absent from it stay untouched on their existing binding.
8. Per-Runtime exact relocation — no bulk binding change. For each record,
   one Runtime (or small batch) at a time, follow
   `infra/runbooks/runtime-cold-relocation.md` STEPS with the
   absent-compute variant:

   - Run the bounded absence probe on lat1 (rescue mode: container and
     task absent) and record the attestation — required before
     `--source-compute-absent` may be used.
   - Compute the target manifest on lat4 with the deployed Runner binary
     over `/data/finite-saas-runner/kata/<durable-state-id>`; require it to
     equal the export-recorded source manifest exactly.
   - Enqueue via the system-installed Core CLI:
     `finite-saas-core runtime-cold-relocate-exact --source-compute-absent`
     with `--project-id`, `--expected-agent-runtime-id`,
     `--expected-source-host-id finite-lat-1`,
     `--expected-source-machine-id`, `--target-source-host-id
     finite-lat-4`, `--expected-agent-npub`, and
     `--durable-state-manifest-sha256`.
   - Core replaces the binding only after the lat4 Runner proves the staged
     tree at lease time (manifest, identity file, absent target compute)
     and the launched `/contact` endpoint exposes the expected npub. A
     failed pre-binding request removes target compute and leaves Core
     untouched — rollback stays the reviewed relocation transaction, not
     manual repair.
   - Give each relocated Runtime its own observation window (chat round
     trip, workspace present) before the next batch; keep lat4 otherwise
     drained.

9. `scripts/finite-status` before/after; migrated Runtimes appear under
   `finite-lat-4` with drain still true.

First-execution evidence (2026-08-29):

- The operator Mac copied all eight files directly over lat4's public SSH
  endpoint. Source and destination full sha256 values and exact byte sizes
  agreed for all four archives; `/data/staging` is restored to `root:root
  0700` with files `0600`.
- The target work root was empty, the Runner condition was gated off because
  `/etc/finite/runner.env` was absent, and containerd had zero containers and
  zero tasks before import. Both RAID1 arrays were `[UU]`.
- The main capture followed by the newer kata overlay produced exactly
  154,299 authoritative regular files. The final combined checksum and size
  proofs passed as 147,235 non-overlay files plus 7,064 overlay files; no
  AppleDouble file was imported.
- All 217 selected SQLite databases outside `workspace`, `.venv`, and
  `node_modules` trees returned `ok` from `scripts/snapshot-sqlite
  integrity-check` through manifested scratch copies. No imported database
  was opened directly.
- The directory inventory and the checksum-manifest-derived inventory agreed
  on 35 payload durable-state directories; `.recovery-rollbacks` is a control
  directory, not a migration candidate. All 35 identity files are regular,
  non-symlink files. The deployed Runner produced a target state manifest for
  every candidate in root-only
  `/data/staging/lat4-target-state-manifests.txt` (record sha256
  `4de33d7aaf5a7ec777901cac22a38246dc3d4bc11e96be544c7c95c809e6ca1a`).
- WireGuard handshook with lat2 and `10.254.3.1:14200` was reachable by the end
  of the import. This is import evidence only: Core enumeration, the exact
  35-directory/Core-record intersection, Runner credential installation,
  relocation, chat observation, and Gate F completion remain pending.
- **Later 2026-08-29:** Paul installed Gate D credentials, undrained lat4
  (`FC_RUNNER_DRAIN=false`), relocated the active lat1 set (21 tasks in the
  `finite` containerd namespace; Core records on `finite-lat-4` /
  `2026-08-29.1`), and rolled lat3's active set to the same artifact. Chat
  observation is not closed: `finitechat hermes serve` still wedges on
  unbounded HTTP (fix in PR #765, not yet in the runtime image). Finite
  Private is in `FINITE_ADMISSION_MODE=allowlist` with one allowlist entry;
  `https://finite.computer` is the Vercel outage page (usage-API health 307s
  to HTML), while lat2 at `64.34.80.19` still serves Core
  `/internal/finite-private/v1/health` as 401 JSON when resolved directly,
  and `chat.finite.computer` remains on lat2 and healthy. Do not revert the
  limiter to `usage-api` until `FINITE_USAGE_API_URL` reaches Core, not the
  outage origin. Lat4's new-launch pin is still
  `finite-agent-runtime-2026-08-27.2`.

VERIFY: all four received archives hash-clean, 154,299 per-file checks pass,
integrity checks clean, the two enumerations agree exactly, every migrated
Runtime relocated through `runtime_relocation.v1` and answering chat from
lat4, and the deployment changelog records Gate F.

ROLLBACK: the import is additive onto an empty target; remove the imported
tree and Core bindings revert per the relocation contract's rollback section
(reverse exact transaction; the old source canonical path is preserved
non-canonically). No bulk edit of Core bindings occurs at any point. The
source archive remains stopped-and-intact throughout.

## VERIFY (whole runbook)

- Every gate above states its own verify; none may be skipped forward.
- `scripts/finite-status` before and after every rollout step.
- The repo matches the box: any manual change lands in `infra/` within a
  day (break-glass rule).

## ROLLBACK

- Gate A: stop and hold lat4 untouched; nothing has happened to it yet.
- Gate B: the PR does not merge, or reverts; the artifact is never built
  with `captured = false` (the build script refuses).
- Gate C: before reinstall there is no host rollback (the wipe is the
  boundary); after install, NixOS generations and
  `nixos-rebuild switch --rollback`; the rescue-mode reinstall can be
  re-run from the same artifact.
- Gate D: remove secret files (the Runner is `ConditionPathExists`-gated
  and stays dead), or remove the lat4 WG peer from the lat2 hub; lat3 and
  lat2 are untouched throughout.
- Gate E: re-set `FC_RUNNER_DRAIN=true` and revoke the new Core keyring
  entry; this is the auditable admission boundary.
