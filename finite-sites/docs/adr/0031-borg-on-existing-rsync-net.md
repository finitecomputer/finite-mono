# ADR 0031: Snapshot and Borg Backups

## Decision

Sites uses a stopped-service snapshot archived with native Borg 1.x over SSH
to a dedicated repository on rsync.net. This follows the existing
[hosted backup procedure](../../../infra/nixos/modules/backups.nix).

One job stops only Sites, copies the complete data tree with rsync, creates a
consistent SQLite registry backup, verifies the snapshot, and resumes Sites
before uploading it. Each upload requires a fresh successful capture. This
includes Git history, blobs, authorization state, and the cookie signing key.

On Fly, stock Supervisor and cron own the opt-in job lifecycle. The backup
credential and control socket are inaccessible to the serving process. The
job does not stop Chat, Core, Identity, or runners. The maintenance pause and
daily capture interval must meet the deployment's availability and recovery
requirements before backups are enabled.

## Recovery Contract

Borg provides authenticated encryption and deduplication. Preserve the Sites
repository's exported repokey, passphrase, SSH access, pinned host identity,
runtime configuration, and deploy artifact independently of the serving host.
Another repository's key cannot recover Sites. Local staging is plaintext and
must remain private.

The job does not initialize, prune, or compact the remote repository. No-prune
is not server-enforced append-only protection; verify destination credential
restrictions before claiming isolation from other repositories.

An independent client must restore the complete Recovery Set onto an empty
target and prove content, permissions, and continued publishing. Successful
uploads or local tests alone do not satisfy this requirement. See the
[backup interface](../backups.md) and
[operating runbook](../../../infra/runbooks/sites-borg-recovery.md).
