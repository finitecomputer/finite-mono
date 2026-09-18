# Account and Principal boundaries

Account Auth gates SaaS account access. A Nostr Principal proves control of a
key. These are separate authorities; a selected dashboard Agent, matching
email string or Core relationship does not grant product access by itself.

| Identity | Authority |
| --- | --- |
| WorkOS user/account | Dashboard and Core SaaS authorization |
| Human Chat Principal / Device | Human signing identity and per-Device MLS state |
| Agent Principal | Separate key under the Runtime's durable Finite Home |
| Finite VIP NIP-05 name | Directory binding to a public key; managed agent names are not mailboxes |
| Product grant | The owning product's current permission and key-distribution rules |

Tools in one Finite Home reuse its local identity key. Human and Agent Runtime
homes never share a secret. Local key storage, locking and signing are owned
by [Finite Identity](../../finite-identity/SPEC.md). Identity Directory owns
NIP-05 names and their proof; it does not own Brain or Sites authorization.

Brain verifies its own membership and Folder grants. Sites owns its Email
Principals, native key sets, shares and viewer sessions. Each product verifies
the evidence required for its operation locally. Brain revocation is explicit;
a Core account change alone does not remove Brain access or rotate Folder Keys.

Agent operations use the Agent Principal and explicit product grants, never an
implicit copy of the human's secret. Dashboard navigation or an Agent display
name cannot authorize choosing or rewriting durable Chat/Brain state.

Runtime stop, restart or image replacement preserves the Agent Principal.
WorkOS logout blocks dashboard access without revoking an Agent's independent
product keys. Revocation cannot recall plaintext previously obtained.

Public Directory state is not backup of the private signing key. Recovery must
restore the required keys together with their product data from independently
held recovery material. Same-volume preservation and TEE isolation do not prove
that Recovery Set. See [recovery authority](../../docs/adr/0001-recoverability-precedes-operator-blindness.md).
