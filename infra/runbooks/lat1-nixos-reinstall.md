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

- A machine with Nix + the built closure (the cutover used lat2, the runner
  box). `nix build .#nixosConfigurations.finite-lat-1.config.system.build.toplevel`
  first — this is the go/no-go gate; do NOT wipe until it builds clean.
- `cpio` installed on BOTH the driver host and the target (nixos-anywhere
  needs it to build the kexec initrd; its absence aborts safely pre-wipe).
- Latitude console access (IPMI + Rescue Mode) — the recovery net if boot
  fails. Confirm you can reach it before starting.
- Operator SSH key in `default.nix` `users.users.root.openssh.authorizedKeys`
  (the same key drives the install and reaches the box after reboot).

## Install (wipes the box)

```sh
# from the driver host, in a finite-mono checkout, target reachable as root:
nix run github:nix-community/nixos-anywhere -- \
  --flake .#finite-lat-1 \
  --target-host root@64.34.82.77 \
  --phases kexec,disko,install
# then reboot into the installed system:
ssh root@64.34.82.77 systemctl reboot     # or power-cycle via the console
```

nixos-anywhere kexecs its own installer (which CAN partition these disks),
runs disko (single-disk), installs the closure, and stops before reboot.

## If it won't boot: rescue-mode recovery

1. Latitude panel → **Rescue Mode** (boots Ubuntu-in-RAM with SSH; gives a
   fresh password each launch). Add your key to `ubuntu`'s authorized_keys
   and `sudo` to add the driver host's key to `/root/.ssh`.
2. Diagnose from the IPMI console FIRST (it shows the real screen even with no
   network / no root password): stage-1 mount error vs `login:` prompt (=
   booted, network issue) vs BIOS/no-boot.
3. To re-deploy a fixed config, re-run nixos-anywhere from the driver host
   against the rescue environment (it re-kexecs + reinstalls). Single-disk
   installs are fast (~10 min, closure copy dominates).
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
<any>.finite.chat                        -> 200
```
Plus the Tinfoil limiter `/health` → `usageApi authenticated: true, 200`
(Finite Private end-to-end).

## Post-install follow-ups

Enable offsite borg backups first (single-disk = backups are the safety net),
then a disk mirror. The Nix configuration enables the Kata Runner timer; verify
its live credential, capacity, Runtime artifact, and readiness path before the
internal production canary. Phala remains a fast-follow adapter and does not
gate Kata. Brain + oauth2-proxy are deferred (still on smoke). See
`finite-fable/notes/lat1-cutover-complete-2026-07-09.md`.
