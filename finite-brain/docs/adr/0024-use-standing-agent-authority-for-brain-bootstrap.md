# Use Standing Agent Authority For Brain Bootstrap

Status: accepted 2026-07-16. Supersedes ADR-0020's one-use user-approved
Personal Vault Bootstrap Authorization.

An authenticated account-bound Agent Principal has standing Agent Bootstrap
Authority to create its user's single user-owned Personal Vault and atomically
establish itself in that Vault's one Personal Agent role. The FiniteBrain skill asks the user once
in ordinary natural language before the agent exercises that authority, but the
reply is behavioral guidance rather than a server-enforced approval, exact
command, button, or setup ticket. Brain therefore does not claim cryptographic
proof that the user confirmed the action.

Agent Bootstrap Authority is available only while the user's Personal Vault
does not exist. If the Vault already exists, an unpaired agent cannot enroll
itself; the calling agent may use the Vault only when it already occupies the
Personal Agent role. The authority cannot create a second Personal Vault,
transfer or delete the Vault, change ownership or Recovery Principals, or add,
remove, or replace another agent.

Core is authoritative for the WorkOS account-to-agent association. Finite
Identity manages the Agent Principal Key inside the agent's protected
environment and is authoritative for Managed Agent Email-to-public-Agent
Principal resolution; its server never receives or returns the private key.
Brain combines those facts and is authoritative for Personal Agent Access.
Finite Chat Hosted Device remains the hosted human-key custodian and signer,
but does not participate in the Personal Agent bootstrap path or grant Brain
access.

An agent-first request never supplies or selects the Personal Vault owner.
Brain derives the owning WorkOS account from Core's authenticated account-agent
association and resolves that account's existing User Nostr Identity through
Finite Identity. Missing, ambiguous, or conflicting facts fail without
creating or changing a Vault, and an existing Personal Vault prevents agent
self-enrollment.

Bootstrap is atomic and idempotent. A retry by the Personal Agent established
by the successful bootstrap returns the existing result; a different or
unpaired agent fails once the Vault exists. Partial failure leaves neither a
new Vault nor a Personal Agent relationship.

The FiniteBrain skill treats bootstrap as a prerequisite within the user's
original task. After successful setup, the agent resumes that task immediately
without asking the user to send a separate continuation message.

The canonical managed FiniteBrain skill owns one concise behavioral
double-check step. With no Personal Vault, it asks once in ordinary language; a
clear affirmative proceeds, while a negative or unclear response leaves Brain
unchanged, acknowledges the skipped setup once, and returns control to the
user. The exact wording is natural rather than scripted. This behavior is not
duplicated across CLI reference or adapter documentation.

When a Personal Vault already exists and the current agent is not its Personal
Agent, the skill does not run the bootstrap double-check or attempt
self-enrollment. It states that the user must replace the current Personal Agent
from Brain settings and leaves the Vault unchanged.
