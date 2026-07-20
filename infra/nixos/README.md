# infra/nixos — finite-lat-1 as code

The NixOS definition of the single app server (finite-lat-1, 64.34.82.77).
The root flake's `nixosConfigurations.finite-lat-1` composes the modules here;
`packages.nix` builds every server binary from this workspace.

**LIVE since 2026-07-09.** The cutover is done — lat1 was reinstalled as
NixOS and now runs the whole coupled cluster (Core, dashboard, native
Postgres, chat, sites, search, one Caddy edge). This tree IS lat1's current
config; copying and directly switching to the exact closure prebuilt on lat2
is the deploy.
The historical cutover and its hard-won gotchas (single-disk/no-mdadm, disks
by-id, WAN-by-MAC) are in `infra/runbooks/lat1-nixos-reinstall.md`; its
destructive procedure is paused and is not current recovery authority. Brain is
served under the WorkOS-protected dashboard origin. The Hosted Web Chat
offsite-health jobs and first archive now pass; its complete empty-target
restore and the complete Agent/host Recovery Set remain unproved. Its snapshot
is deploy/manual-triggered, not periodic; the former 15-minute stop/start timer
was removed because it broke chat streams. A disk mirror remains deferred and
is defense in depth, not a backup.

## finite-lat-3 storage canary

`nixosConfigurations.finite-lat-3` is the pinned NixOS 26.05 storage-qualified
Runner candidate at `207.188.7.157`. Its host-specific definition is
`hosts/finite-lat-3/`: two exact-size RAID1 arrays, two independently mounted
removable-path ESPs, ext4 project quotas on `/data`, a 64-GiB swapfile with
bounded zswap, stable disk/partition/filesystem identities, and fail-closed
storage health checks. The bootloader wrapper refuses an update unless both
expected FAT ESPs are mounted read-write with their exact PARTUUIDs.

It was installed and storage-qualified on 2026-07-20. The current generation
adds Kata/containerd and a timer-disabled Runner configured for a drained
private-path proof. It is not customer capacity or a Recovery Authority until
the synthetic handoff passes. The authoritative sequence and dated evidence are in
`docs/runs/finite-lat-capacity-and-redundancy.md`. Builds happen on the existing
finite-lat-2 x86_64 Nix builder; lat2's services and storage are unchanged.

## Deploy story

### Bare-metal rebuild (paused; historical transcript follows)

Do not run the commands below as a current procedure. They summarize the
2026-07-09 cutover and remain useful only for artifact handoff and incident
evidence. A new recovery-proved procedure is governed by
`docs/runs/finite-lat-capacity-and-redundancy.md`; until it is exercised, start
an incident with `infra/runbooks/break-glass.md` and preserve state.

```sh
set -euo pipefail
git fetch origin --prune
REV="$(git rev-parse HEAD)" # must be the pushed, reviewed 40-hex commit
[[ "$REV" =~ ^[0-9a-f]{40}$ ]]
git merge-base --is-ancestor "$REV" origin/main
SYSTEM="$(just nixos-build-lat1 "$REV")" # gate before you wipe; record this path
printf 'REV=%s\nSYSTEM=%s\n' "$REV" "$SYSTEM"
```

Enter lat2 with temporary agent forwarding:

```sh
ssh -A ubuntu@finite-lat-2
```

On lat2, paste the recorded values and run this fail-closed block. Do not
recompute either value:

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
# Constrain the Nix subprocesses launched by nixos-anywhere as well.
export NIX_CONFIG='builders ='
nix run --option builders '' github:nix-community/nixos-anywhere -- \
  --build-on local \
  --store-paths "$DISKO" "$SYSTEM" \
  --target-host root@64.34.82.77 --phases kexec,disko,install
```

Then the secrets checklist below + the data restore in the runbook.

Do not run Nix evaluation, `nix build`, `nixos-rebuild`, or `nixos-anywhere`
for the production closure on macOS. Nix would inherit `/etc/nix/machines` or
the operator's personal builder settings. The root recipe runs only SSH on the
Mac, is fixed to `ubuntu@finite-lat-2`, checks the remote hostname and system,
and evaluates/builds the exact pushed GitHub commit on lat2 with `builders =`
explicitly empty. There is no builder override: neither clawland nor lat1 is a
permitted production build host.

### Every deploy after that

`finite-lat-2` is the required x86_64 build host for finite-mono production
closures. **Do not use clawland and do not build on finite-lat-1.** A deploy
has two immutable handoff values: a full lowercase 40-hex Git commit `REV` and
the exact `/nix/store/...-nixos-system-finite-lat-1-...` path `SYSTEM` printed
by the prebuild. A tag, branch, 12-character abbreviation, or dirty working
tree is not an acceptable handoff.

From the reviewed checkout on the Mac, confirm the commit is pushed to
`origin/main`, then prebuild it:

```sh
set -euo pipefail
git fetch origin --prune
REV="$(git rev-parse HEAD)"
[[ "$REV" =~ ^[0-9a-f]{40}$ ]]
git merge-base --is-ancestor "$REV" origin/main
SYSTEM="$(just nixos-build-lat1 "$REV")"
case "$SYSTEM" in /nix/store/*) ;; *) exit 1 ;; esac
printf 'REV=%s\nSYSTEM=%s\n' "$REV" "$SYSTEM"
```

The helper's stdout is only `SYSTEM`. Evaluation and building occurred over
SSH on lat2 with remote builders disabled, and the result is GC-rooted at
`~ubuntu/.local/state/finite-mono/lat1-closures/$REV` on lat2. The matching
bare-metal disko script is rooted at
`~ubuntu/.local/state/finite-mono/lat1-disko-scripts/$REV`; it is used only by
the reinstall flow. Record both printed values in the deploy log. Then SSH to
lat2 with temporary agent forwarding and paste those exact two values; do not
recompute either value
there. Lat2 intentionally stores no lat1 deploy key, and it cannot resolve the
`finite-lat-1` alias:

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
# The exact lat2-built closure is unsigned. The authenticated root SSH
# transport is the trust boundary for this reviewed handoff.
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
printf 'expected=%s\nprofile=%s\nactual=%s\n' "$SYSTEM" "$PROFILE" "$ACTUAL"
test "$PROFILE" = "$SYSTEM"
test "$ACTUAL" = "$SYSTEM"
ssh -o BatchMode=yes root@64.34.82.77 systemctl stop "$UNIT"
```

This first advances `/nix/var/nix/profiles/system` to the exact copied closure,
so boot and generation rollback agree with the activation. The direct
`switch-to-configuration` call runs in a transient systemd unit and survives a
lost SSH connection; it does not evaluate or build on lat1. The final
exact-path assertion is the deployment identity check and must pass before
service-specific verification.
If the client SSH connection drops after `systemd-run`, reconnect through
lat2 and inspect `systemctl status` plus `journalctl -u` for
`finite-nixos-activate-$REV.service`; activation continues independently. Do
not start a second activation until that unit and both exact profile paths are
accounted for.
Rollback remains `ssh root@64.34.82.77 nixos-rebuild switch --rollback`,
followed by recording and verifying the newly active `/run/current-system`.

## Secrets bootstrap checklist (values NEVER in this repo)

All root-owned, 0600 unless noted. Names only; sources are the old hosts.

| File | Variable names | Value source |
|---|---|---|
| `/etc/finite/core.env` | `FC_CORE_DATABASE_URL` (embeds `POSTGRES_PASSWORD`), `FC_CORE_API_TOKEN`, `FC_CORE_RUNNER_CREDENTIALS_JSON`, one `FC_CORE_RUNNER_CREDENTIAL_TOKEN_*` variable per active Runner credential, `FC_FINITE_PRIVATE_USAGE_API_TOKEN`, `WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `FC_WORKOS_OPERATOR_ORG_ID` | Existing names come from the k8s Secret on old lat1. The checked-in production Kata generation may temporarily retain legacy `FC_CORE_RUNNER_API_TOKEN`; before any second worker starts, replace it with the metadata keyring and separately named Kata/Phala bearer variables documented in `finitecomputer-v2/docs/finite-stack-deployment.md`. Route and worker credentials must be distinct. The usage token pairs with the Tinfoil-sealed `FINITE_USAGE_API_SERVICE_KEY` — **do not rotate at cutover**. Core uses the WorkOS API key only to resolve the verified user record for a validated JWT `sub`. |
| `/etc/finite/runner.env` | the generic Core, artifact, Kata, endpoint, and secret-reference names in `infra/hosts/lat1/systemd/runner.env.example`, including `FC_RUNNER_FINITE_PRIVATE_SPECIALIZATION_WORKER_API_KEY` | provision the route-scoped Runner credential; select `kata`, the promoted mono runtime artifact, loopback Core, and the production Sites/Brain endpoints; copy the dedicated specialization worker client token from its owning host secret without reusing the GLM key |
| `/etc/finite/phala-runner.env` | `FC_CORE_RUNNER_API_TOKEN`, `FC_RUNNER_PHALA_API_KEY`, `FC_RUNNER_RUNTIME_ARTIFACT_ID` (+ the bounded runtime env/secret-reference names in `infra/hosts/lat1/systemd/phala-runner.env.example`) | **dark/not installed by this definition**; separately authorize a Core keyring credential named `finite-phala-runner-1`, bound to class `phala` and source host `finite-lat-1-phala-control-1`, plus a host-only Phala HTTPS API key; never reuse the Kata token or put either credential in Runtime environment |
| `/etc/finite/runtime-secrets.env` | the shared tool-provider names selected by Core's names-only `FC_CORE_RUNTIME_SECRET_REFERENCES_JSON` and listed in `infra/hosts/lat1/systemd/runtime-secrets.env.example` | legacy `../finitecomputer/secrets/shared-provider-keys.env`; values remain host-only, OpenRouter is not selected for the new platform, and specialization credentials stay in their owning service |
| `/etc/finite/dashboard.env` | `FC_CORE_API_TOKEN`, `WORKOS_API_KEY`, `WORKOS_CLIENT_ID`, `WORKOS_COOKIE_PASSWORD`, `FC_WORKOS_OPERATOR_ORG_ID`, `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `GOOGLE_WORKSPACE_CLIENT_ID`, `GOOGLE_WORKSPACE_CLIENT_SECRET` (+ optional `FC_RELAY_ADMIN_TOKEN`, `FC_RELAY_HOST_ENDPOINTS_JSON`) | Existing names come from the k8s Secret on old lat1; provision the same missing operator-org predicate used by Core before rollout |
| `/etc/finite/hosted-web-device.env` | `FINITECHAT_HOSTED_API_TOKEN` | generate for the Hosted Web Device internal service boundary; the service and dashboard read this same server-only value; store it in the team password manager |
| `/etc/finite/sites-viewer-session.env` | `FINITE_SITES_VIEWER_SESSION_TOKEN` | generate exactly 32 random bytes as 64 lowercase hex characters (`openssl rand -hex 32`) for the Sites verified-email viewer-session boundary; systemd/Podman read this root:root 0600 file before dropping service privileges; Sites and the dashboard receive the same server-only value; store it in the team password manager |
| `/var/lib/finitecomputer/backups/rsync-net/{id_ed25519,known_hosts,borg-passphrase}` | existing finitecomputer Borg SSH private key, pinned rsync.net host key, and repository passphrase | copy the established root-only credential bundle from an existing finitecomputer host; the off-host passphrase copy already lives in the ignored `../finitecomputer/workspaces/trf/secrets/` tree. Do not generate a parallel credential set or put values in this repo. Verify the destination restriction before claiming append-only protection. |
| `/etc/finite-saas/sites.env` (0640) | `RESEND_API_KEY` (+ optional `FINITE_IDENTITY_AUTHORITY`) | migrated from lat2 `/etc/finite-saas/sites.env` |
| `/etc/finite-saas/certs/finite-chat-origin.pem` (0644) / `.key` (0640 root:caddy) | — | copied from lat2 at cutover (Cloudflare Origin CA pair; host-agnostic, covers the zone) |
| `/etc/finite/searxng.env` | `SEARXNG_SECRET` (+ optional `SEARXNG_BASE_URL`, `SEARXNG_LIMITER`) | lat2 `finite-search/searxng/.env` |
| `/etc/finite/firecrawl.env` | `BULL_AUTH_KEY`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB`, `MAX_CPU`, `MAX_RAM` | lat2 `finite-search/firecrawl-upstream/.env` |
| Postgres role password | — | `ALTER ROLE finite WITH PASSWORD '<POSTGRES_PASSWORD>';` before the restore (`modules/postgres.nix` header) |

finite-brain has **no** secret env (plain-config `Environment=` lines only,
per the smoke capture).

## Google Workspace OAuth production setup

The dashboard connection flow uses one operator-managed Google OAuth client;
users connect it from their machine's **Connections** page. The live credential
must be an OAuth 2.0 Client ID with application type **Web application**. In
Google Cloud Console, its Authorized redirect URI must be exactly:

```text
https://finite.computer/google-workspace/callback
```

That is a separate callback from WorkOS' `/callback`; do not substitute one
for the other or add a trailing slash. The server performs the code exchange,
so this flow does not require a browser-side Google secret.

Before enabling the connection:

1. Configure the OAuth consent screen for the intended canary accounts. Use
   **Internal** when the project and every user belong to the same Google
   Workspace organization. Otherwise keep the app in **Testing** and add each
   participating account as a test user until the app's publication and
   verification work is deliberately taken on.
2. Enable the Gmail, Google Calendar, Google Drive, Google Sheets, Google Docs,
   People, and Google Apps Script APIs in that project.
3. Configure the consent screen with the exact checked-in scope contract in
   `finite-skills/skills/productivity/google-workspace-finite/references/google-workspace-scopes.json`.
   This includes the OpenID identity scopes used to bind the connected email;
   omitting an API or requested scope makes the dashboard reject the grant.
4. Put only the corresponding values in `/etc/finite/dashboard.env`, under
   the names `GOOGLE_WORKSPACE_CLIENT_ID` and
   `GOOGLE_WORKSPACE_CLIENT_SECRET`. `WORKOS_COOKIE_PASSWORD` is also required
   there to seal the short-lived, user-bound OAuth state. Never copy those
   values into this repository, a command transcript, or logs.
5. Keep the checked-in `FC_DASHBOARD_BASE_URL` and
   `NEXT_PUBLIC_WORKOS_REDIRECT_URI` origins (or an explicit
   `FC_DASHBOARD_PUBLIC_URL` override) pointed at the production dashboard.
   Browser-facing OAuth redirects must use that configured origin rather than
   the dashboard container's loopback request URL.

Acceptance is not a configuration inspection or a callback-only probe. From
one real, authorized production account, click **Connect**, complete Google's
consent, return to Connections with the connected Google email visible, and
then perform one real operation through the agent whose API and permission are
inside the granted scope (for example, a Drive search or Calendar list). Keep
that final operation read-only unless the tester explicitly intends a write.

## Port map (consolidated box)

| Port | Bind | What | Was |
|---|---|---|---|
| 22 | public | sshd (root key-only) | lat1 |
| 80/443 | public | Caddy — ALL vhosts | lat1 + lat2 + clawland + smoke edges |
| 3000 | 127.0.0.1 | dashboard (podman, host-net) | was lat1 k3s NodePort 30080 |
| 3002 | 127.0.0.1 | firecrawl api (podman) | lat2 |
| 3015 | 127.0.0.1 | finite-brain | smoke (previously public-bound there) |
| 4200 | 127.0.0.1 | finite-saas-core (nix-built binary) | was lat1 k3s ClusterIP |
| 5432 | 127.0.0.1 | postgres 16 native (`finite_core`) | was lat1 k3s StatefulSet |
| 8080 | 127.0.0.1 | searxng (podman) | lat2 |
| 8787 | 127.0.0.1 | finitesitesd | lat2 |
| **8788** | 127.0.0.1 | **finitechat-server (moved off 8787** — sitesd owns it here; public URL unchanged) | clawland 8787 |
| 38918 | 127.0.0.1 | Finite Chat Hosted Web Device (dashboard-internal) | new |
| 9100 | 127.0.0.1 | node-exporter | new |
| 2019 | 127.0.0.1 | caddy admin API | lat1/lat2 |
| 14200 | 10.254.3.1 (WireGuard) | private proxy to Core :4200 | lat3 Runner only |
| dynamic 32768-60999 | 10.254.3.2 (WireGuard) | lat3 Kata Runtime contact/health | lat1 peer only |

Caddy vhost → backend: `finite.computer` → 4200 (`/internal/finite-private/*`)
else 3000; `chat.finite.computer` → 8788; `api./*.finite.chat` +
`*.docs.finite.chat` → 8787 (Cloudflare Origin CA). Brain has no independent
edge: authenticated `/client` and `/_admin/*` requests go through the dashboard
to loopback :3015, then Brain applies its Nostr authorization.

## Open follow-ups (post-cutover; grep for TODO)

Resolved during the 2026-07-09 cutover: disko device layout (single-disk,
by-id), gateways/resolvers, root ssh key, dashboard image digest. Still open:

- **Non-disruptive recovery cadence + restore proof** (`modules/backups.nix`) —
  the service-consistent Hosted Web Chat snapshot service, rsync.net
  repository, Borg 1.2 selection, established credential paths, and
  stale-health units are defined. Snapshot creation is deploy/manual-triggered;
  no 15-minute timer exists. The 2026-07-18 live inventory observed the
  offsite jobs healthy and a verified first archive. Add a stream-safe cadence
  and complete an empty-target drill before claiming the accepted RPO. A
  destination-enforced append-only upload credential remains recommended
  hardening.
- **Disk mirror** — root + `/data` are single NVMe. The matching Micron and
  Samsung disks contain stale MD metadata from the failed 2026-07-09 install;
  they are not free/untouched spares. The accepted `finite-lat-3` rehearsal
  must prove exact member sizing, release-matched assembly, dual-ESP boot, and
  degraded rebuild before a separately authorized lat1 reprovision.
- **Runner fast-follow** — Kata is the production adapter; Phala must pass the
  same provider-neutral contract before it is enabled.
- **KATA ISOLATION** (`modules/finitesitesd.nix`): sites run
  `--app-runner none` — tier-2 `app` sites lack microVM isolation until Kata
  (or microvm.nix) is ported.
- **firecrawl API** (:3002) down — searxng works; crawl/scrape degraded.
- Dead-man's-switch ping (`modules/monitoring.nix`); finite-search image
  digest pins (`modules/finite-search.nix`).
