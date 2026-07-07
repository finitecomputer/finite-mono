# Domain Notes For Agents

- Read `CONTEXT.md` first.
- Keep FiniteBrain Vault, Folder, sharing, and OKF policy out of this repo.
- Treat NIP-05 as a mutable identification binding. Never store it as the
  primary account key.
- Treat a Frostr group public key as a possible user primary key. Individual
  shares are not identities.
- Treat the native secure-storage share as the user's Cold Backup Share. Normal
  signing uses server plus active user-client share; the cold backup share is
  for recovery or rotation unless a later ADR says otherwise.
- Treat the default FiniteBrain agent model as a shared user-agent signer:
  agents request signatures as the user's primary key through policy-bound
  signing sessions. Record agent accountability in Finite audit/session data,
  not as a separate default Nostr public key.
- Store bearer session tokens only as hashes.
- Challenge consumption and session creation must be transaction-backed when
  implemented in storage.
