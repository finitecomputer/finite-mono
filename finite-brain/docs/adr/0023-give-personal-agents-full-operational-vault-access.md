# Give Personal Agents Full Operational Vault Access

Status: accepted 2026-07-16. Supersedes ADR-0020.

A Personal Agent is fully trusted to operate on behalf of the user across every
current and future Folder in the user's Personal Vault. It may read, write,
organize, share, invite collaborators, and directly delete Vault content and
Folders. Brain automatically maintains the necessary per-Folder cryptographic
grants, but exposes no Agent Workspace or Folder-by-Folder agent delegation
product model.

The User Nostr Identity remains the Personal Vault's sole owner. Only that human
owner may transfer or delete the Vault, change ownership or Recovery
Principals, or add, remove, or replace Personal Agents after bootstrap. An
account-bound agent may establish itself as the initial Personal Agent while
performing agent-first bootstrap under its standing Agent Bootstrap Authority.
Every Personal Agent uses its own Agent Principal Key and never receives or uses
the human's identity secret. User-first and agent-first initialization establish
the same durable, revocable Personal Agent Access relationship; removing it
rotates every current Folder Key required to end future access but cannot recall
plaintext already retained by the agent.

A Personal Vault has exactly one Personal Agent in the current product scope.
Agent-first bootstrap may fill that role only while atomically creating the
Personal Vault; if the Vault already exists, an unpaired agent cannot enroll
itself. Personal Vaults do not expose an add-another-agent flow. Multi-agent
membership and administration belong in Organization Vaults, while Personal
Vault content may also be shared or exported. Personal Agent Access remains
active when the Vault contains no Folders, including after its last Folder is
deleted.

The owner may replace the one Personal Agent. Brain first removes the old
relationship and rotates every current Folder Key needed to end its future
access, then the owner adds the replacement by its Managed Agent Email. The
removed agent cannot re-enroll itself under Agent Bootstrap Authority because
the Personal Vault already exists.

Replacement is one atomic owner-authorized operation: Brain verifies the
replacement's Managed Agent Email and Agent Principal, rotates every current
Folder Key, removes the old agent's grants, creates the replacement's grants,
and swaps the Personal Agent relationship. Any failure leaves the old Personal
Agent and every current Folder unchanged.

User-first setup is one atomic owner-authorized operation that creates the empty
Personal Vault and adds the currently selected, identity-resolved agent as its
one Personal Agent. The client shows the agent's Managed Agent Email before the
owner confirms; dashboard selection only prefills the target and carries no
authority. If agent ownership or identity resolution fails, the operation
creates neither the Vault nor the Personal Agent relationship. Agent-first
bootstrap converges on the same owner, empty Vault shape, and Personal Agent
role.

Personal Agent operations remain signed and minimally audited as the agent's
own Agent Principal, using its Managed Agent Email wherever existing history UI
needs a readable actor. This does not add a separate confirmation, banner, or
product ceremony and never represents the agent action as human-signed.

Both setup paths are atomic and idempotent. An exact retry by the established
Personal Agent returns the existing Personal Vault and relationship; it never
creates a duplicate. An unpaired or different agent fails once the Vault
exists. Any failure before the Vault and Personal Agent relationship are fully
established rolls back both so a clean retry remains possible.

Every new Personal Vault Folder automatically issues current Folder Key Grants
to the owner and current Personal Agent, regardless of which one creates it;
Folder creation exposes no agent-recipient choice. If the Personal Agent role is
vacant, the new Folder grants only the owner. Sharing with other Principals
remains a separate explicit product action.

Permanent deletion of the underlying agent in Core immediately blocks new
Brain actions from that Agent Principal, removes the Personal Agent
relationship, and rotates every current Folder Key. The human-owned Personal
Vault and its content remain intact, and the owner may later add a replacement.
Temporary Runtime stops, restarts, and replacements that preserve the same
Agent Principal do not revoke access. Revocation cannot recall plaintext or old
keys the agent already retained.

Implementation status: owner-initiated removal and replacement provide this
revocation and rotation behavior now. Automatic revocation after permanent
Core-agent deletion remains deferred until Core exposes a permanent-agent
deletion operation or event; Brain must not claim that integration before then.
