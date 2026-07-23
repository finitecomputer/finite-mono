# Recovery And Durability

Use this branch for backup, restore, migration, disaster recovery, or any claim
that hosted Brain data is durable.

The Recovery Set has five live parts:

1. Core account and authority data.
2. Finite Chat state, transcript history, and decryptable attachments.
3. The Hosted Device identity paired with its Chat client database.
4. Brain state and knowledge.
5. Agent state sufficient to complete a fresh turn after restore.

Brain sync, a server export, a Provider Durable Volume, a TEE, or a disposable
`.finitebrain/` search index is not independently a Recovery Set. Never copy a
derived search index as backup material.

Recovery execution is platform-runbook work, not an `fbrain` action. In a
Finite Mono checkout, follow the authoritative
`infra/runbooks/hosted-web-chat-recovery.md` for evidence gathering, snapshot
boundaries, synthetic restore, and rollback. In a managed runtime without that
runbook, stop and escalate to the platform operator. Production repair remains
read-only until the user explicitly authorizes the named mutation.

Completion requires restoration of the same five-part set onto an empty target,
with all of the following proven:

- Account and authority records reopen under the restored services.
- Chat counts and history match, and a restored attachment decrypts.
- The Hosted Device secret is a valid secp256k1 scalar; its derived public key
  matches both `identity.json` and the Chat database's sole account binding.
- Brain identity and knowledge reopen under that same restored identity.
- A fresh Agent turn succeeds after restore.

Coordinated tampering, an invalid scalar, a missing identity/database pair, an
ambiguous account binding, or a partial restore fails closed. Name the backup
and rollback boundary before any authorized production mutation.

If every completion condition has not been observed, report exactly which
evidence is missing and describe recovery as unproven rather than durable.
