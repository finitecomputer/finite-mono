# Backups on finite-lat-2

> **HISTORICAL 2026-07-08 SITES CAPTURE — DO NOT RUN.** ADR 0007 moved the
> live app-plane stack back onto `finite-lat-2` on 2026-08-29, but this file is
> still only evidence of the old pre-NixOS Sites backup gap. Current backup and
> restore procedures live under [`infra/runbooks/`](../../runbooks/) and the
> NixOS host modules. Do not move old Sites files, install or enable these
> timers, or use `/data` as a recovery target from this document.

## Current reality (captured 2026-07-08) — GAP

There is **no automated backup on this host**:

- No cron: the crontab binary is not even installed; `/etc/cron.d` has only
  `e2scrub_all`.
- No backup systemd timers (only OS defaults: dpkg-db-backup, apt-daily,
  fstrim, e2scrub, mdcheck, motd-news, tmpfiles-clean, mdmonitor-oneshot).
- No backup scripts in `/usr/local/bin`, `/usr/local/sbin`,
  `/opt/finite/finitecomputer/tools`, `/root`, or `/home/ubuntu`.

Manual tarballs only:

| File | Size | Date | Durable? |
|---|---|---|---|
| `/var/backups/finite-sites/finite-sites-20260617T215714Z.tar.gz` | 46.0 MB | 2026-06-17 | yes — **newest durable backup** |
| `/var/backups/finite-cleanup/finite_sites_pre_msb_cleanup_20260617T213015Z.tar.gz` | 46.0 MB | 2026-06-17 | yes (pre-MicroSandbox-cleanup snapshot) |
| `/tmp/finite-sites-20260702T145453Z.tar.gz` | 18.0 MB | 2026-07-02 | **NO — /tmp is a 94G tmpfs; lost on reboot** |

The data at stake: `/var/lib/finite-sites` — apps 110M, blobs 24M, git 26M,
`registry.db` SQLite (~4.4M with WAL), plus the cookie secret. `/data`
(1.8T `/dev/md1`) is empty (28K used) and unused.

The former first action was to move the July 2 tarball out of `/tmp`. It is no
longer an authorized action on the CI/build host.

## Proposed fix (not yet deployed)

`systemd/finite-sites-backup.service` + `systemd/finite-sites-backup.timer`
— both headed **PROPOSED — NOT YET DEPLOYED**, disabled by default. Daily at
03:15 UTC: tar `/var/lib/finite-sites` + `/etc/finite-saas` to
`/data/backups/finite-sites-<stamp>.tar.gz` (root 0600), keep the newest 14.

Historical proposed install (never run):

```sh
sudo install -m 0644 infra/hosts/lat2/systemd/finite-sites-backup.service /etc/systemd/system/
sudo install -m 0644 infra/hosts/lat2/systemd/finite-sites-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now finite-sites-backup.timer   # the explicit opt-in
```

Restore drill (a backup is only real once restored — infra/README.md
principle 4): extract to a scratch snapshot directory, record its files in a
relative-path `manifest.sha256`, then use the snapshot helper:

```sh
(
  cd EXTRACTED_SNAPSHOT
  find var etc -type f -print0 \
    | LC_ALL=C sort -z \
    | xargs -0 sha256sum > manifest.sha256
)
scripts/snapshot-sqlite integrity-check \
  EXTRACTED_SNAPSHOT/var/lib/finite-sites/registry.db
```

Expected output is `ok`. The helper refuses an unmanifested directory and
never opens the extracted evidence in place. registry.db is WAL-mode SQLite,
so a live tar can be mid-write; perform a stop-the-world backup before
destructive operations.

Known limits of this proposal, accepted to keep it dead simple:

- `/data` is the same chassis — this closes the tmpfs/staleness gap, not the
  off-box gap. Off-box (and Litestream for registry.db, debt-ledger item 4)
  is the follow-up.
- The tarball contains secrets (`sites.env`, the Origin CA key). Never copy
  it off-box unencrypted.
- Tier-2 app state under `/var/lib/finite-app/*` (systemd runner
  StateDirectory) is NOT in scope; no `finite-app@` instances are running
  today, and the Kata runner's app data lives inside
  `/var/lib/finite-sites/apps/`, which is covered.
