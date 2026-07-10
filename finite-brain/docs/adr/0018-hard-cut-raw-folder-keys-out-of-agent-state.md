# Hard-Cut Raw Folder Keys Out Of Agent State

Status: accepted

The next Agent State version removes raw Folder Keys and performs a hard
migration from legacy state: an upgraded `fbrain` scrubs legacy key material,
clears stale unlocked status, and reopens encrypted Folder Key Grants through
the acting Member Identity's signer. There is no compatibility fallback that
uses serialized legacy keys; a missing signer or grant leaves the Folder
locked, and FiniteBrain does not claim to erase prior copies retained by
backups, snapshots, or filesystem history. The migration preserves encrypted
Folder Key Grants and any Recovery Principal or Recovery Set material; it
removes redundant opened plaintext keys, not recovery authority.
