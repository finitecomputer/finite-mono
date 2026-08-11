# Start Mounts With An Explicit Destination Participant

Status: accepted

Amended 2026-08-07 by ADR-0045: a Personal Brain destination includes its owner
and ready Personal Brain Agent Set rather than one optional Personal Agent.

Accepting a Mount Offer will initially include only the accepting destination
owner or admin as a Mount Participant rather than exposing the source Folder to
every destination Brain Member. Destination governance may explicitly add or
remove its own Members afterward; source governance may not micromanage the
destination roster but may revoke the entire Shared Folder Connection. The
destination may also revoke the connection. This chooses a disclosure-safe
default and Brain-local roster control over automatic organization-wide access.

When the destination is a Personal Brain, acceptance explicitly includes the
owner and every ready Personal Brain Agent as initial Mount Participants because
Personal Brain Agent Access covers all content in that Brain. The acceptance
preview and receipt must identify the human and included agent count; a Personal
Brain whose agents are not yet ready initially includes only its owner.

Only the destination Brain's owner, Personal Brain Agents, admins, and Members are
eligible Mount Participants. Destination Guests cannot participate, preventing
a Brain from transitively resharing mounted content through identities it does
not govern.
