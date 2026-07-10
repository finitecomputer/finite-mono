# Authorize Member Identities, Not Controller Kinds

Status: accepted

FiniteBrain's authorization and attribution principal is the signing Nostr
`npub`, called a Member Identity; the server does not classify its controller
as human, agent, shared client, or any other kind. Controllers may share one
keypair or use several, but each keypair is a separate identity from
FiniteBrain's perspective, and any desired separation of access, revocation,
or attribution is created by using separate Member Identities and Folder Key
Grants rather than agent-specific authorization. Client and runtime policy may
provision or label distinct human and agent keypairs, and product-scoped
delegation may describe that provenance, but neither grants FiniteBrain access
until the signing npub receives the required membership, access, and Folder Key
Grants.
