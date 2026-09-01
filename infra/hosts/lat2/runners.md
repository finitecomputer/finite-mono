# GitHub Actions runner inventory on finite-lat-2

> **DECOMMISSION TARGET.** Docker/image CI moved to Depot, production Nix
> closure builds moved to the Depot-backed `Lat1 NixOS Closure` workflow, and
> no current workflow should select a lat2 runner. Keep this file only as
> inventory for removing old registrations during the Gate E cleanup of
> [`lat2-replacement-cutover.md`](../../runbooks/lat2-replacement-cutover.md)
> (the former decommission runbook was retired 2026-08-29).

Captured 2026-07-08. The authoritative registration state is GitHub Settings
-> Actions -> Runners for each repository, not this dated file. Before
repurposing or wiping the host, every runner whose name starts with
`finite-lat-2` must be offline, unregistered from GitHub, and removed from
`/srv/github-runner/`.

| Install dir | Runner name | Registered repo | Custom labels |
|---|---|---|---|
| `/srv/github-runner/finite-mono` | `finite-lat-2-mono` | `finitecomputer/finite-mono` | `finite-lat-2,docker,nix` |
| `/srv/github-runner/finitechat-hermes-runtime` | `finite-lat-2-finitechat-hermes-runtime` | `finitecomputer/finitechat` | `finite-lat-2,docker,hermes-runtime` |
| `/srv/github-runner/finitecomputer-tinfoil-runtime` | `finite-lat-2-tinfoil-runtime` | `finitecomputer/finitecomputer` | `finite-lat-2,nix,docker,tinfoil-runtime` |
| `/srv/github-runner/finitecomputer-v2-runtime` | `finite-lat-2-finitecomputer-v2-runtime` | `finitecomputer/finitecomputer-v2` | `finite-lat-2` |

Defaults `self-hosted,Linux,X64` applied when these runners were registered.
All captured services ran as `User=ubuntu` from host-generated systemd units
named `actions.runner.<owner>-<repo>.<runner-name>.service`.

## Removal notes

Do not archive runner credentials. The files below are bearer credential
material and should be revoked through GitHub runner removal, then deleted with
the runner directory:

- `/srv/github-runner/*/.credentials`
- `/srv/github-runner/*/.credentials_rsaparams`
- `/srv/github-runner/*/.runner`

For each runner, use a short-lived removal token from the registered
repository's GitHub Actions runner settings or API. Token values must never be
written to this repo, logs, shell history, or a shared terminal transcript.

```sh
cd /srv/github-runner/<runner-dir>
sudo ./svc.sh stop || true
sudo ./svc.sh uninstall || true
./config.sh remove --token <REMOVAL_TOKEN>
```

After GitHub shows the runner removed, delete the local directory as part of
the lat2 decommission runbook. There is intentionally no restart, maintenance,
stale-lease, or fallback procedure here; a stuck lat2 runner is removed, not
repaired.
