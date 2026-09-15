# Borg on the existing rsync.net surface

Accepted September 15, 2026 for FIN-54. Adapt the existing legacy Sites backup
coverage to Fly using stopped-Sites snapshots and native Borg archives.

Use native Borg 1.x over SSH to a dedicated Sites repository on the existing
rsync.net account. Reuse the established SSH credential, pinned host identity,
passphrase custody, and independent recovery procedure documented in
`infra/nixos/README.md`. Borg provides authenticated encryption, chunk
deduplication, archive completion, and extraction.
Each dedicated Borg repository has its own generated repokey; export the Sites
key independently. Another service's key export cannot recover this repository.

rsync.net is the destination; Borg speaks its own protocol over SSH. A separate
rsync of the live registry/Git tree is neither necessary nor a consistent
backup. If a staging transfer is later needed, transfer only a completed
Recovery Point and its immutable dependencies to a protected filesystem.
Never copy an active Borg repository while it has writers.

Reuse the Sites portion of `infra/nixos/modules/backups.nix`: stop Sites,
copy its data directory and create a consistent SQLite registry backup, verify
the snapshot, and resume Sites. Archive that completed snapshot with Borg.
Adapt the existing job's lifecycle controls to Fly; do not carry its Chat,
Core, Identity, or runner stops into an independent Sites backup. Resume Sites
before the remote upload and ensure capture failures also trigger recovery of
the previously running service. Measure the pause and agree its operating
window before enabling the job; this is not a zero-downtime promise.

One scheduled job creates a fresh snapshot before each upload. If capture
fails, report failure rather than uploading an older snapshot as fresh. Keep
the existing daily archival cadence as the starting proposal; confirm the
acceptable data-loss window before enabling it. Borg supplies deduplication
and archive consistency. Full snapshots include metadata-only changes and all
source history without per-project queues, revision hooks, a dependency graph,
separate metadata schedules, or a custom reconciliation service.

Implementation lives in the Sites operator script and opt-in stock
Supervisor/cron image configuration. There is no new daemon transport API.
The custom content-addressed capture/restore code has been removed; tests use
the actual stopped-Sites snapshot and native Borg flow. Remaining operational
work is credential provisioning, image promotion, external freshness alerts,
and an independent remote restore drill.

The archival job does not prune or compact. Retention needs separately
authorized administrative access and restore proof. No-prune is not
server-enforced append-only protection: the existing SSH credential has been
documented as overprivileged. Do not claim isolation from other repositories
until destination restrictions have been verified.

Option B / Fly is the active hosting path; Option A / Latitude remains available
without changing the backup format. Both need independent access to the Borg
endpoint, SSH key, pinned host key, passphrase, exported Sites repokey, runtime
configuration, and deploy artifact. A clean client restoring to an empty target
must prove serving, permissions, and continued publishing. A local encrypted
Borg test is not evidence of rsync.net access or a completed Fly restore drill.
