# Revision backups with shared-state checkpoints

Sites retains live SQLite, Git, and static blobs on the serving host and backs
up changed project revisions independently. Registry and service-secret
checkpoints preserve ownership, permissions, and revision mappings without
recopying every site's content. A completed Recovery Point must name and verify
all of its immutable dependencies; a successful publish does not imply that
asynchronous backup has finished.

This replaces the proposed recurring whole-service archive approach in FIN-54.
No global serving stop or new publishing workflow is required. Source-only
projects, non-deploy Git refs, metadata-only changes, and historical revisions
are part of the Recovery Set. Upload retries and reconciliation must be durable;
partial uploads cannot become selectable Recovery Points.

Backup retention must preserve shared objects referenced by any retained
Recovery Point. Object-age expiration alone is not a valid garbage collector.
Backup access and decryption-key recovery remain independent of the serving host.
Production qualification requires an empty-target application restore, not
merely successful upload or SQLite integrity. Neither this decision nor local
tests authorize production provisioning or cutover.

The original S3 transport choice was replaced on September 15, 2026 by
[Borg over SSH to rsync.net](0031-borg-on-existing-rsync-net.md). This does not
relax the checkpoint, durable scheduling, or application restore requirements.
