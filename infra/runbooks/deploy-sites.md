# Deploying finite-sites (finitesitesd) to lat1

Since the 2026-07-09 cutover, finitesitesd runs on finite-lat-1
(64.34.82.77), NOT lat2. Config: `infra/nixos/modules/finitesitesd.nix`
(host `finite-lat-1`). It serves `*.finite.chat` / `*.docs.finite.chat` /
`api.finite.chat` as systemd unit `finite-saas-sites.service`
(finitesitesd on 127.0.0.1:8787), fronted by the one host Caddy with the
Cloudflare Origin CA cert. Data `/var/lib/finite-sites` (16 published sites,
npubs intact, restored from lat2 at cutover). Topology:
`infra/nixos/README.md`. The historical
[2026-07-09 bare-metal transcript](lat1-nixos-reinstall.md) is not current box-
rebuild authority.

> **KATA GAP (flagged follow-up):** this module ships `--app-runner none` —
> sites run WITHOUT microVM isolation, so tier-2 tenant apps do not run until
> Kata (or microvm.nix) is ported. lat2 previously ran `--app-runner kata`.
> Tracked as the KATA ISOLATION TODO in `modules/finitesitesd.nix`.

> History: sites previously deployed to lat2 by rsync-source + `cargo build
> --release` on the box + `sudo install`. That box no longer serves sites.
> Do not resurrect the build-on-box flow.

## Deploy flow — prebuilt immutable mono rev

`fsite/v*` releases still ship the `fsite` CLI + `finitesitesd` linux binary
([release-cli.md](release-cli.md)), but on lat1 the *daemon* is deployed by
nixos-rebuild (the flake builds `finitesitesd` from the pinned mono rev), not
by copying a release tarball onto the box.

### PRECONDITIONS

- The finitesitesd source change is merged to `main` (you deploy a committed
  rev).
- ssh access from the Mac to `ubuntu@finite-lat-2`, with agent forwarding for
  root access from lat2 to `64.34.82.77`. Lat2 is the only production
  build/driver host; do not evaluate or build on the Mac, clawland, or lat1.
- A fresh Postgres/state safety net exists. FLAG: the Hosted Web Chat recovery
  snapshot does **not** cover `/var/lib/finite-sites`; Sites still needs its own
  service-consistent off-host recovery set and restore proof. Do not treat the
  configured chat Borg repository as Sites protection.

### STEPS

1. From the reviewed checkout, prebuild the full pushed commit on lat2 and
   record the two immutable handoff values:

   ```sh
   set -euo pipefail
   git fetch origin --prune
   REV="$(git rev-parse HEAD)"
   [[ "$REV" =~ ^[0-9a-f]{40}$ ]]
   git merge-base --is-ancestor "$REV" origin/main
   SYSTEM="$(just nixos-build-lat1 "$REV")"
   printf 'REV=%s\nSYSTEM=%s\n' "$REV" "$SYSTEM"
   ```

   `REV` must be the exact lowercase 40-hex commit, not a tag, branch, short
   hash, or dirty tree. The printed `/nix/store/...` path is GC-rooted on lat2.

2. SSH to lat2 with temporary agent forwarding and paste the recorded values
   exactly. Preflight forwarded root authentication, then copy, switch, and
   prove lat1 runs the exact prebuilt closure:

   ```sh
   ssh -A ubuntu@finite-lat-2
   ```

   On lat2, run:

   ```sh
   set -euo pipefail
   REV='<exact-40-hex-rev-from-prebuild>'
   SYSTEM='<exact-/nix/store-path-from-prebuild>'
   [[ "$REV" =~ ^[0-9a-f]{40}$ ]] || exit 64
   [[ "$SYSTEM" =~ ^/nix/store/[0-9a-z]{32}-nixos-system-finite-lat-1-[^/[:space:]]+$ ]] || exit 64
   ROOT="$HOME/.local/state/finite-mono/lat1-closures/$REV"
   test -L "$ROOT"
   test "$(readlink -f "$ROOT")" = "$SYSTEM"
   nix path-info --option builders '' "$SYSTEM" >/dev/null
   ssh -o BatchMode=yes root@64.34.82.77 true
   # The exact lat2-built closure is unsigned; authenticated root SSH is the
   # trust boundary for this reviewed handoff.
   nix copy --no-check-sigs --option builders '' \
     --to ssh-ng://root@64.34.82.77 "$SYSTEM"

   UNIT="finite-nixos-activate-${REV}.service"
   ssh -o BatchMode=yes root@64.34.82.77 \
     bash -s -- "$REV" "$SYSTEM" "$UNIT" <<'LAT1'
   set -euo pipefail
   rev="$1"
   system="$2"
   unit="$3"
   [[ "$rev" =~ ^[0-9a-f]{40}$ ]] || exit 64
   [[ "$system" =~ ^/nix/store/[0-9a-z]{32}-nixos-system-finite-lat-1-[^/[:space:]]+$ ]] || exit 64
   [[ "$unit" == "finite-nixos-activate-${rev}.service" ]] || exit 64
   test "$(readlink -f "$system")" = "$system"
   test -x "$system/bin/switch-to-configuration"
   nix-store --check-validity "$system" >/dev/null
   load_state="$(systemctl show --property=LoadState --value "$unit" 2>/dev/null || true)"
   [[ "$load_state" == not-found ]] || {
     echo "refusing to replace existing transient unit $unit ($load_state)" >&2
     exit 73
   }
   nix-env --option builders '' --profile /nix/var/nix/profiles/system \
     --set "$system"
   test "$(readlink -f /nix/var/nix/profiles/system)" = "$system"
   systemd-run --quiet --unit="$unit" --property=Type=oneshot \
     --property=RemainAfterExit=yes --no-block \
     "$system/bin/switch-to-configuration" switch
   LAT1

   deadline=$((SECONDS + 600))
   while true; do
     if ! state="$(ssh -o BatchMode=yes -o ConnectTimeout=5 root@64.34.82.77 \
       systemctl show --property=ActiveState --value "$UNIT" 2>/dev/null)"; then
       state=unreachable
     fi
     case "$state" in
       active) break ;;
       activating|inactive|unreachable) ;;
       failed)
         ssh -o BatchMode=yes root@64.34.82.77 \
           journalctl --no-pager -n 100 -u "$UNIT" >&2 || true
         exit 1
         ;;
       *) echo "unexpected activation state: $state" >&2; exit 1 ;;
     esac
     (( SECONDS < deadline )) || { echo "activation timed out" >&2; exit 1; }
     sleep 2
   done
   PROFILE="$(ssh -o BatchMode=yes root@64.34.82.77 \
     readlink -f /nix/var/nix/profiles/system)"
   ACTUAL="$(ssh -o BatchMode=yes root@64.34.82.77 \
     readlink -f /run/current-system)"
   test "$PROFILE" = "$SYSTEM"
   test "$ACTUAL" = "$SYSTEM"
   ssh -o BatchMode=yes root@64.34.82.77 systemctl stop "$UNIT"
   ```

   Empty builders keep all evaluation/building local to lat2. Installing the
   system profile first preserves boot/generation rollback; transient-unit
   activation survives SSH loss and cannot build on lat1.

3. Config-only changes (listen flags, `--app-runner`, sites.env references,
   Caddy vhosts) all live in `infra/nixos/modules/` — never edit units on the
   box. Cert is the Cloudflare Origin CA pair at
   `/etc/finite-saas/certs/finite-chat-origin.{pem,key}` (no ACME; the zone is
   Cloudflare-proxied Full-strict — do not "fix" cert errors by switching to
   ACME).

### VERIFY

1. `ssh root@64.34.82.77 'systemctl status finite-saas-sites'` — active.
2. `curl -fsS https://api.finite.chat/api/v1/healthz`.
3. Load a published site (`https://<something>.finite.chat`) and a
   `*.docs.finite.chat` vhost. (sitesd serves by Host header; there is no
   root `/healthz` on the wildcard vhosts — a 404 at `/` is normal.)
4. TODO: once finitesitesd exposes a `source_commit` health payload
   (finitechat-style contract gate), gate on it here.

### ROLLBACK

```sh
ssh root@64.34.82.77 nixos-rebuild switch --rollback
```

reverts to the previous generation (finitesitesd binary + config together);
or prebuild and deploy the previous known-good rev's exact closure from lat2.
Verify `/run/current-system` against the selected rollback path, then re-run
VERIFY and reconcile git within a day (break-glass rule).
