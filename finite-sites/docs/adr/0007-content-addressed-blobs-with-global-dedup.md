# Content-Addressed Blobs With Global Dedup

Blobs are stored once per sha256, shared across all Sites and Versions. Git
publication reads committed deploy bytes, verifies hashes and records immutable
blobs before activating a Version. Publishing has no client-facing missing-blob
list or direct upload API. `Store::missing_blobs` remains an internal store
helper; it is not a public hash-existence oracle.

Blobs live on the Sites data volume behind a four-operation interface
(`put`/`has`/`get`/path). Off-host recovery uses the complete stopped-Sites
snapshot archived by Borg, as described in
[the recovery runbook](../../../infra/runbooks/deploy-sites.md#backups-and-restore).

Serving authorizes the Site before reading its Version's blobs. Any future
client-visible deduplication protocol must assess whether it exposes another
owner's hash existence; the current internal storage choice does not authorize
such an API.
