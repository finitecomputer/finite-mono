# Borg on the existing rsync.net surface

Accepted September 15, 2026 for FIN-54. This replaces ADR 0030's S3 transport
choice, not its Recovery Set or revision-triggered backup requirements.

Use native Borg 1.x over SSH to a dedicated Sites repository on the existing
rsync.net account. Reuse the established SSH credential, pinned host identity,
passphrase custody, and independent recovery procedure documented in
`infra/nixos/README.md`. Borg provides authenticated encryption, chunk
deduplication, archive completion, and extraction. Do not introduce an AWS
dependency, custom encryption format, or bespoke object-store transport.
Each dedicated Borg repository has its own generated repokey; export the Sites
key independently. Another service's key export cannot recover this repository.

rsync.net is the destination; Borg speaks its own protocol over SSH. A separate
rsync of the live registry/Git tree is neither necessary nor a consistent
backup. If a staging transfer is later needed, transfer only a completed
Recovery Point and its immutable dependencies to a protected filesystem.
Never copy an active Borg repository while it has writers.

Keep application capture separate from archival. Borg must archive completed
Sites Recovery Points, not live SQLite WAL files or changing Git directories.
Revision-triggered work remains asynchronous and durable. Registry and secret
checkpoints also run after metadata-only changes. Deduplication saves transfer
and storage; it does not make the current full-catalog capture incremental.
Do not enable a recurring service-stop job to bypass checkpoint coordination.

The first implementation slice is the native Borg operator procedure and a
synthetic encrypted-archive application restore test. There is no new daemon
transport API. Existing local capture/restore commands remain authoritative for
the Sites format. Scheduling, bounded incremental capture, off-host freshness
reporting, and production remote qualification remain FIN-54 gates.

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
