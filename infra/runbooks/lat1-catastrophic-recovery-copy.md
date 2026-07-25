# finite-lat-1 catastrophic recovery copy

This runbook covers loss of the complete finite-lat-1 host, not an ordinary
service rollback. It complements the component Recovery Sets and does not make
RAID a backup.

## Recommended custody model

Keep three independent layers:

1. Rebuildable NixOS configuration and public recovery tooling in
   `finite-mono`.
2. The encrypted rsync.net Borg repositories for current component Recovery
   Sets, Agent retirement archives, and the secret bootstrap roots.
3. A separately encrypted, operator-custodied Borg repository on removable
   media or another operator-controlled destination. Populate it during the
   final pre-RAID writer fence.

Do not use a raw image of the 480-GB root device plus the 1.92-TB data device as
the primary catastrophic copy. Raw images copy unused blocks and stale RAID
metadata, are awkward to verify, and do not establish application consistency.
The current irreplaceable host state plus recovery snapshots is tens of
gigabytes; a file-level Borg copy is smaller, inspectable, deduplicated, and
can be restored into an empty target.

The operator copy should contain:

- `/var/lib/finite-saas-runner` in full, including active, retired, rollback,
  and upgrade-snapshot state;
- `/var/lib/finite-sites`;
- `/var/lib/private/finitechat-hosted-device`;
- `/var/lib/private/finite-chat`;
- `/var/lib/private/finitebrain`;
- `/var/lib/finite-identity`;
- `/var/lib/finite-saas-core`;
- `/data/recovery-snapshots` and `/data/backups`;
- `/etc/finite` and `/etc/finite-saas`;
- `/var/lib/finitecomputer/backups/rsync-net`.

The component snapshot/restore tools remain the preferred recovery source for
SQLite and Postgres. Raw live database files in the catastrophic copy are only
defense in depth and must not replace the coordinated snapshot or `pg_dump`.

## Bootstrap root of trust

Accessing the remote Borg repository requires all four of:

- the exact rsync.net repository endpoint;
- an independently held SSH authentication credential and pinned `known_hosts`;
- the Borg passphrase;
- the exported Borg repokey.

The production private SSH key should be escrowed outside finite-lat-1 before
the host is destroyed. A separately held account password is also a valid
break-glass transport credential. Archiving either credential inside the
repository is useful redundancy but is circular by itself. Store the transport
credential, pinned host identity, and Borg recovery material in an encrypted
operator-controlled location with an independent unlock path. Never commit
values, fingerprints, or password-derived hashes.

As of the 2026-07-25 read-only audit, the operator Mac has mode-0600 copies of
the lat1 Borg passphrase and exported repokey. The operator then proved an
interactive, password-authenticated SSH login from the Mac to the rsync.net
account and listed the account root containing the `finitecomputer` directory.
That closes independent transport access. The Mac's ordinary SSH identities
still cannot authenticate and no independent local copy of the noninteractive
repository transport key has been proved. A Borg repository listing using the
independently held passphrase and repokey, followed by an empty-target restore,
remain separate recovery proofs.

## Pre-RAID creation gate

1. Select a destination with enough free space. Prefer an encrypted removable
   disk so the only copy is not on the operator's system volume.
2. Initialize a new local Borg repository with encryption independent of the
   rsync.net repository. Export its key immediately and retain its passphrase
   separately.
3. Enter the separately approved maintenance fence: drain creation, stop all
   Agent compute and product writers, and create the final coordinated v3
   snapshot.
4. Run the remote Borg job and prove the new archive contains the bootstrap
   roots without printing their contents.
5. Stream the named catastrophic paths over authenticated SSH into the local
   encrypted Borg repository. Do not create an intermediate plaintext tar.
6. Restart writers if this is only a backup rehearsal. Keep them stopped when
   the approved RAID rebuild immediately follows.
7. On an empty offline scratch target, restore the bootstrap files and a
   representative active, retired, Sites, Chat, Brain, Identity, and Core
   artifact. Run the normal component verifiers. A successful archive listing
   alone is insufficient.
8. Record archive names, sizes, checksums of public manifests, destination
   identity, and pass/fail. Do not record secret values or live user content.

The local catastrophic repository is a second recovery destination, not a
license to delete remote archives or the five retired-Agent Recovery Sets.

## Production authorization boundaries

The following require separate explicit approval:

- reading and transferring the rsync.net SSH private key;
- choosing and writing the operator-custodied destination;
- stopping writers and Agent compute;
- running the coordinated production snapshot and remote archive;
- downloading the catastrophic copy;
- deleting any container metadata, rollback directory, durable state, or
  source archive.
