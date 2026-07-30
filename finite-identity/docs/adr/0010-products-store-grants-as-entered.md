# Products preserve resolution input without confusing it with authority

Products preserve the typed Mailbox Address, NIP-05 Name, or native Nostr
identifier a user entered as audit and invitation context. That input is not
itself durable authorization authority.

Each product materializes its own stable authorization form after the required
proof. Sites authorizes a revocable native key set owned by a verified Sites
Email Principal; Brain encrypts Folder Keys to native `npub`s; Chat resolves a
NIP-05 Name to the canonical participant `npub`. Finite Identity supplies
resolution and proof facts but does not own those product grants.
