# Content-Addressed Blobs With Global Dedup

Blobs are stored once per sha256, shared across all sites and versions.
A publish session reports which manifest hashes the server lacks, and the
client uploads only those. Uploads are verified byte-for-byte against the
hash they claim before the blob row is recorded.

Blobs live on the Sites data volume behind a four-operation interface
(`put`/`has`/`get`/path). Off-host recovery uses the complete stopped-Sites
snapshot archived by Borg, as described in
[ADR 0031](0031-borg-on-existing-rsync-net.md).

Known tradeoff: the missing-blob list reveals whether a given hash exists
anywhere on the platform (here.now and Workers static assets accept the
same leak). Logged in the technical debt ledger; per-owner dedup scoping is
the fallback if it ever matters.

**Considered Options**

- Per-site blob namespaces: no cross-tenant hash oracle, but no dedup of
  framework assets shared by every generated site.
- Global content-addressed store: maximal dedup, simplest serving path;
  chosen with the leak documented.
