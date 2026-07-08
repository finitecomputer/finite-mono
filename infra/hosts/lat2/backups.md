# Backups on finite-lat-2

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

First action item regardless of automation: move the Jul 2 tarball out of
`/tmp` (e.g. `sudo mv /tmp/finite-sites-20260702T145453Z.tar.gz
/var/backups/finite-sites/`), then take a fresh one.

## Proposed fix (not yet deployed)

`systemd/finite-sites-backup.service` + `systemd/finite-sites-backup.timer`
— both headed **PROPOSED — NOT YET DEPLOYED**, disabled by default. Daily at
03:15 UTC: tar `/var/lib/finite-sites` + `/etc/finite-saas` to
`/data/backups/finite-sites-<stamp>.tar.gz` (root 0600), keep the newest 14.

Install (explicit operator step; nothing auto-enables):

```sh
sudo install -m 0644 infra/hosts/lat2/systemd/finite-sites-backup.service /etc/systemd/system/
sudo install -m 0644 infra/hosts/lat2/systemd/finite-sites-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now finite-sites-backup.timer   # the explicit opt-in
```

Restore drill (a backup is only real once restored — infra/README.md
principle 4): extract to a scratch dir and run
`sqlite3 .../registry.db 'PRAGMA integrity_check;'` expecting `ok`, per
`finite-sites/docs/deploy-finite-lat-2.md` §6. registry.db is WAL-mode
SQLite, so a live tar can be mid-write; the stop-the-world backup in that
doc remains the gold path before destructive operations.

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
