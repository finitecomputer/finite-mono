# Treat The Device As The V1 Identity Boundary

Status: accepted

Portable v1 treats the Trusted Device Boundary as the protection for the
Member Identity's persistent Finite secret rather than requiring an additional
application password, interactive unlock, or hardware-backed signer. The
identity keeps the current Finite Home's strict storage contract, but
compromise of it is explicitly total Member Identity compromise for every
Finite product using that home; Session Folder Keys do not claim to protect
against that attacker, and a user-visible lock must prevent automatic grant
reopening until the controller explicitly unlocks. A human client and an Agent
Runtime may be provisioned with separate Finite Homes and keypairs without
changing FiniteBrain's Member Identity authorization semantics.

## Consequences

- Member devices and headless runtimes must protect identity storage with their
  OS account boundary and encrypted disk or volume.
- V1 does not provide per-device identity or revocation.
- Unattended Agent Runtimes may continue using the local signer after restart.
- Product and security claims must distinguish a plaintext-blind server from a
  compromised trusted endpoint.
