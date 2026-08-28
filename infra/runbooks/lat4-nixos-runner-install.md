# finite-lat-4 NixOS Runner install (twin of finite-lat-3)

Pattern authority: ADR 0007 (second runner twin, PR #715); lat4 is the third
runner host and slots in after the #715 /29 widening merges. Lat4-specific
evidence, decisions, and open items:
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
- PR #715 has merged: `wg-finite` is /29 on lat1 and lat3 with lat2 = .3, so
  lat4 = `10.254.3.4` is a valid overlay address.
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
3. Set the WG underlay before install: lat1 carries the lat4 peer (public key
   matching the operator-staged lat4 private key) with
   `allowedIPs 10.254.3.4/32` and the firewall scope for the new peer; deploy
   via lat1's existing closure flow and confirm the lat1/lat3 handshakes are
   still current. This PR deliberately does NOT carry that lat1 edit: it
   conflicts with PR #715's lat1 /29 change, so it lands as a small follow-up
   commit on this branch immediately after #715 merges, before Gate B
   executes.

VERIFY: artifact manifest matches the merged rev; `nixosConfigurations`
evals green; lat1 deployed with the lat4 peer and its existing handshakes
current.

### Gate C — wipe and install (provider console + artifact install)

1. Provider console: launch Rescue Mode for finite-lat-4; note the one-time
   root password. The rescue environment must show all four NVMe devices
   attached (compare against the Gate B capture).
2. `ssh root@152.236.34.15` (accept the new host key; this is rescue Ubuntu).
   TODO(prove): confirm the rescue kernel exposes all four disks and that
   the disko script's by-id paths resolve.
3. From the artifact directory, realize the manifest's exact store paths from
   the artifact's file binary cache and drive the install consuming only
   those realized paths (the uploaded artifact carries `nix-cache/` plus the
   manifest; the system, disko, and kexec links are named in the manifest,
   not shipped as directories):

   ```sh
   # Realize the exact manifest paths into the local x86_64-linux store.
   nix copy --from "file://$PWD/nix-cache" \
     "$(jq -r .system manifest.json)" \
     "$(jq -r .disko manifest.json)" \
     "$(jq -r .kexec manifest.json)"

   # TODO(prove): exact nixos-anywhere flag spellings are proven here on
   # first execution; the invariants below are what must hold — the disko
   # script comes BEFORE the system closure, the kexec tarball is the
   # same-pin CI artifact (not nixos-anywhere's default), and nothing is
   # built on the target.
   nixos-anywhere \
     --disko-script "$(jq -r .disko manifest.json)" \
     --nixos-system "$(jq -r .system manifest.json)" \
     --kexec "$(jq -r .kexec manifest.json)" \
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
     keypair whose public key is already registered on lat1.
   - `/etc/finite/metrics-remote-write.env` and `/etc/finite/logs-write.env`
     (0600) — monitoring write credentials; the activation preflight
     `check-lat-monitoring-secrets` blocks activation without them.
2. WireGuard: confirm the handshake with lat1 (`wg show` on both ends; the
   endpoint is `64.34.82.77:51820`, local `10.254.3.4`). From lat4, the
   Core URL from `runner.env` must answer over the overlay (an HTTP
   response, not a timeout). lat4's firewall exposes exactly 22/tcp,
   51820/udp, and the Kata contact range on `wg-finite` only.
3. Core credential registration BEFORE the drained first lease: the
   off-host-escrowed `finite-kata-runner-4` keyring entry lands in
   `/etc/finite/core.env` on lat1 and Core restarts in the
   platform-rollout window. Registration is not admission — it only lets
   the Runner authenticate while still drained. Then trigger one Runner
   cycle and confirm an authenticated, draining response — no creation is
   offered while `FC_RUNNER_DRAIN=true`:

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
  and stays dead), or remove the lat4 WG peer from lat1; lat3 and lat1 are
  untouched throughout.
- Gate E: re-set `FC_RUNNER_DRAIN=true` and revoke the new Core keyring
  entry; this is the auditable admission boundary.
