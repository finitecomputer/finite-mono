# Principal Grants with Provenance, Chat Approval Cards, and Optional Core Enrichment

Status: accepted

Supersedes the stored-cohort account-access design proposed in the account
agent cohort issues (#441–459) and its two implementation PRs (#465, #467),
which are closed unmerged.

## Decision

FiniteBrain access is modeled as **signed, per-principal Grants with
Provenance**. Every human, agent, browser signer, and chat device is a
Principal with its own key; every access relationship is a signed Grant record
stamped with its origin (who delegated, via which invitation or approval, at
what roster state). There is no stored "cohort" entity, no stored delegation
or authority record, and no server-side intent evaluation. Brains and Sites
work fully without Finite accounts; account-shaped conveniences (invite by
email, agent roster) are an optional enrichment layer, and when the enrichment
is unreachable the convenience degrades, never the cryptography.

## Model

- **Principal** — any keypair that can hold Grants: a human's chat device, an
  agent, a Brain UI browser signer, a CLI identity. Alice never needs to know
  her npub exists.
- **Grant** — a signed record giving a Principal a role in a Brain or Folder.
  Grants are immutable facts with provenance; they are the only access record.
- **Signer tiers** — a policy table over the same primitives: content
  read/write by any content-granted Principal; member administration by any
  admin-granted Principal (including the Brain UI's browser signer); authority
  delegation and ownership-class actions require a signature from the human's
  chat client.
- **Approval Card** — the single human signing surface, rendered in chat
  (eventually multiple-choice, matching the agent question format). An agent
  or service relays the signed approval; the Brain server validates the
  signature plus the account binding via Finite Identity. Brain never watches
  rooms and never judges intent.
- **Finite Identity** — the binding library: npub ↔ account ↔ npub. It
  resolves principals and attests bindings; it grants no Brain authority.
- **SaaS Core** — optional enrichment: the authoritative account agent roster
  and durable, replayable, monotonic Permanent Departure Facts. Routine Brain
  authorization never calls Core. When Core is unreachable, email-shaped UX
  fails closed; everything else continues.
- **Revocation** — Brain-local demotions revoke cohort-derived Grants in the
  same transaction. Core departures are consumed from a last-applied-revision
  cursor, applied exactly once, with Folder Key rotation on replay. Already
  distributed plaintext is un-recallable by physics, not by choice.
- **Recovery Set** — designated in advance, dormant, activates only on
  provable total admin loss (departure facts covering every admin). Recovery
  is provable, never claimed; "lost phone" is not a departure. Originators
  are not required around forever.
- **Sites** — verifies the same signed-approval artifact as Brain and remains
  server-authoritative; the asymmetry is one of rendering, not of trust.

## Consequences

- Alice's intent is honored without npub literacy: tell the bot, tap the card,
  done. Every action stays signed and attributable to the actual Principal.
- The implementation surface shrinks dramatically versus the stored-cohort
  design: no cohort tables, no reconciliation machinery, no mixed-version
  gates, no shared HMAC assertion minter (and therefore no cross-service
  credential reuse).
- Deploys and rollbacks stay ordinary: there is no cohort schema state whose
  existence would make rollback unsafe.
- The Brain UI renders membership from Grant provenance and hosts tier-1/2
  actions; tier-3/4 actions deep-link to the Approval Card in chat.
