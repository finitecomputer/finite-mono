# Reinstalling / recovering finite-lat-1 (NixOS)

How lat1 was cut over to NixOS on 2026-07-09, written as a repeatable
procedure. Use it to rebuild lat1 from bare metal, or to recover if it won't
boot. The NixOS definition is `infra/nixos/` (host: `finite-lat-1`).

**Data safety rule:** every destructive step is preceded by a restore-verified
backup on a DIFFERENT box. The cutover kept all state on lat2 and restore-
drilled the Postgres dump into a scratch container before wiping. Do the same.

## Hard-won facts baked into the config (do not "fix" these)

- **Single-disk, no mdadm** (`infra/nixos/hosts/finite-lat-1/disko.nix`). The
  disko mdadm RAID1 superblocks were unassemblable on the pinned nixpkgs
  (25.11) kernel: recorded array size overran the 129 MiB data offset, so the
  kernel rejected every member with `md: md_import_device returned -22` and
  stage-1 could not mount root. Reproduced on the 6.12 initrd AND a 6.14
  rescue kernel — the arrays themselves were bad, not a config toggle. Root +
  /data are single NVMes; two spare NVMes are free for a future mirror (ZFS,
  or mdadm on a newer nixpkgs). Redundancy today = backups (see below).
- **Disks addressed by `/dev/disk/by-id`** (serial-stable) so the installer
  kernel's enumeration order can't mismatch them.
- **WAN bound by MAC, not interface name** (`default.nix`, systemd-networkd
  `matchConfig.MACAddress = 90:5a:08:2e:63:1b`). The NIC is `enp1s0f1` on the
  NixOS kernel, NOT `eno1` as the Ubuntu capture showed — binding by name
  left the box booted-but-unreachable. MAC-match is immune to renaming.

## Prerequisites

- SSH access to `ubuntu@finite-lat-2`, the repository's only production x86_64
  Nix builder/driver, with temporary agent forwarding for root access from
  lat2 to `64.34.82.77`. Never evaluate or build the production closure on the
  Mac, clawland, or lat1.
- From the reviewed checkout, fetch `origin/main`, set `REV="$(git rev-parse
  HEAD)"`, require that exact lowercase 40-hex commit to be on `origin/main`,
  and run `SYSTEM="$(just nixos-build-lat1 "$REV")"`. Record `REV` and the
  exact output path `SYSTEM`; the helper also roots the matching disko script
  on lat2. This is the go/no-go gate. Do NOT wipe until both build clean.
- `cpio` installed on BOTH lat2 (the driver host) and the target (nixos-anywhere
  needs it to build the kexec initrd; its absence aborts safely pre-wipe).
- Latitude console access (IPMI + Rescue Mode) — the recovery net if boot
  fails. Confirm you can reach it before starting.
- Operator SSH key in `default.nix` `users.users.root.openssh.authorizedKeys`
  (the same key drives the install and reaches the box after reboot).

## Install (wipes the box)

```sh
# On the Mac, record the exact immutable handoff:
set -euo pipefail
git fetch origin --prune
REV="$(git rev-parse HEAD)"
[[ "$REV" =~ ^[0-9a-f]{40}$ ]]
git merge-base --is-ancestor "$REV" origin/main
SYSTEM="$(just nixos-build-lat1 "$REV")"
printf 'REV=%s\nSYSTEM=%s\n' "$REV" "$SYSTEM"
```

Enter lat2 with temporary agent forwarding. Lat2 stores no lat1 deploy key and
cannot resolve the `finite-lat-1` alias:

```sh
ssh -A ubuntu@finite-lat-2
```

On lat2, paste, do not recompute, `REV` and `SYSTEM` from above. This whole
block fails before the wipe if any handoff, store, or authentication check
does not pass:

```sh
set -euo pipefail
REV='<exact-40-hex-rev-from-prebuild>'
SYSTEM='<exact-/nix/store-path-from-prebuild>'
[[ "$REV" =~ ^[0-9a-f]{40}$ ]] || exit 64
[[ "$SYSTEM" =~ ^/nix/store/[0-9a-z]{32}-nixos-system-finite-lat-1-[^/[:space:]]+$ ]] || exit 64
ROOT="$HOME/.local/state/finite-mono/lat1-closures/$REV"
DISKO_ROOT="$HOME/.local/state/finite-mono/lat1-disko-scripts/$REV"
test -L "$ROOT"
test -L "$DISKO_ROOT"
test "$(readlink -f "$ROOT")" = "$SYSTEM"
DISKO="$(readlink -f "$DISKO_ROOT")"
[[ "$DISKO" =~ ^/nix/store/[0-9a-z]{32}-[^/[:space:]]+$ ]] || exit 64
test -x "$DISKO"
nix path-info --option builders '' "$SYSTEM" >/dev/null
nix path-info --option builders '' "$DISKO" >/dev/null
ssh -o BatchMode=yes root@64.34.82.77 true

# NIX_CONFIG also constrains the Nix subprocesses started by nixos-anywhere.
export NIX_CONFIG='builders ='
nix run --option builders '' github:nix-community/nixos-anywhere -- \
  --build-on local \
  --store-paths "$DISKO" "$SYSTEM" \
  --target-host root@64.34.82.77 \
  --phases kexec,disko,install
ssh -o BatchMode=yes root@64.34.82.77 systemctl --no-block reboot
```

After the console shows the reboot completed and SSH returns, paste the same
handoff into a fresh lat2 shell and verify the booted closure exactly:

```sh
set -euo pipefail
REV='<exact-40-hex-rev-from-prebuild>'
SYSTEM='<exact-/nix/store-path-from-prebuild>'
[[ "$REV" =~ ^[0-9a-f]{40}$ ]] || exit 64
[[ "$SYSTEM" =~ ^/nix/store/[0-9a-z]{32}-nixos-system-finite-lat-1-[^/[:space:]]+$ ]] || exit 64
ACTUAL="$(ssh -o BatchMode=yes root@64.34.82.77 \
  readlink -f /run/current-system)"
test "$ACTUAL" = "$SYSTEM"
```

nixos-anywhere kexecs its own installer (which CAN partition these disks),
runs disko (single-disk), installs the closure, and stops before reboot. Its
`--store-paths` inputs are the already-built system and disko outputs rooted by
the exact `REV`; `--build-on local` additionally forbids the tool's automatic
target-build fallback. Empty builders keep production evaluation/building on
lat2 and prevent either clawland or lat1 from becoming a build machine.

## If it won't boot: rescue-mode recovery

1. Latitude panel → **Rescue Mode** (boots Ubuntu-in-RAM with SSH; gives a
   fresh password each launch). Add your key to `ubuntu`'s authorized_keys
   and `sudo` to add the driver host's key to `/root/.ssh`.
2. Diagnose from the IPMI console FIRST (it shows the real screen even with no
   network / no root password): stage-1 mount error vs `login:` prompt (=
   booted, network issue) vs BIOS/no-boot.
3. To re-deploy a fixed config, prebuild and record a new exact `REV` and
   `SYSTEM`, then re-run nixos-anywhere from lat2 with `NIX_CONFIG='builders
   ='` and `--build-on local` against the rescue environment (it re-kexecs +
   reinstalls).
   Single-disk installs are fast (~10 min, closure copy dominates).
4. Read a member/array/NIC with `mdadm --examine`, `ip link`, `lsblk` in
   rescue to confirm the fix before redeploying.

## Secrets bootstrap

Place the `/etc/finite/*.env`, `/etc/finite-saas/sites.env`, and
`/etc/finite-saas/certs/finite-chat-origin.{pem,key}` files per the checklist
in `infra/nixos/README.md`. Cert perms matter: `.pem` 644, `.key` 640
root:caddy, dir 755 — else Caddy fails to load TLS. Never put secret values in
git; extract them from the old k8s Secret / hosts and scp them in.

## Data restore

- **Postgres** (`postgres-backup-restore.md`): the db is `finite_core`, role
  `finite`. `ALTER ROLE finite WITH PASSWORD '<POSTGRES_PASSWORD>'` + `ALTER
  DATABASE finite_core OWNER TO finite`, then `pg_restore -d finite_core
  --no-owner --role=finite --clean --if-exists <dump>` (as the postgres user,
  from a path it can read — NOT /root). **Verify the invariant**: 87 Finite
  Private keys (`select count(*) from finite_private_api_keys`).
- **Sites**: restore the `/var/lib/finite-sites` tar; sitesd serves by Host
  header (no /healthz — 404 there is normal). Cloudflare origin cert covers
  `*.finite.chat`.
- **Chat**: single-writer doctrine (`deploy-finitechat-server.md`) — disable
  the old writer, WAL-checkpoint, copy the SQLite, `chown --reference` the
  state dir (DynamicUser), start, verify `/health`. Never two writers.

## DNS + TLS ordering (matters)

Flip DNS to lat1 BEFORE expecting a Let's Encrypt cert: Caddy's ACME HTTP-01
challenge validates against whatever the name resolves to. If you verify
before propagation, the challenge hits the old box (404) and Caddy backs off —
`systemctl restart caddy` after propagation to retry. Cloudflare-proxied
`*.finite.chat` uses the origin cert instead (no ACME).

## Verify (all over real public DNS)

```
finite.computer                          -> 200   (dashboard)
finite.computer/internal/finite-private/ -> 401   (alive+gated)
chat.finite.computer/health              -> 200
brain.finite.computer/health             -> 200
<any>.finite.chat                        -> 200
```
Plus the Tinfoil limiter `/health` → `usageApi authenticated: true, 200`
(Finite Private end-to-end).

## Post-install follow-ups

Enable offsite borg backups first (single-disk = backups are the safety net),
then a disk mirror. The Nix configuration enables the Kata Runner timer; verify
its live credential, capacity, Runtime artifact, and readiness path before the
internal production canary. Phala remains a fast-follow adapter and does not
gate Kata. Brain now runs on lat1 at its canonical
`brain.finite.computer` origin; the smoke host remains a rollback source.
