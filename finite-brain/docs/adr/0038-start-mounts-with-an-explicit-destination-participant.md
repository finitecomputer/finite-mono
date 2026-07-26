# Start Mounts With An Explicit Destination Participant

Status: accepted

Accepting a Mount Offer will initially include only the accepting destination
owner or admin as a Mount Participant rather than exposing the source Folder to
every destination Brain Member. Destination governance may explicitly add or
remove its own Members afterward; source governance may not micromanage the
destination roster but may revoke the entire Shared Folder Connection. The
destination may also revoke the connection. This chooses a disclosure-safe
default and Brain-local roster control over automatic organization-wide access.

When the destination is a Personal Brain with a current Personal Agent,
acceptance explicitly includes both the owner and Personal Agent as initial
Mount Participants because Personal Agent Access covers all content in that
Brain. The acceptance preview and receipt must name both identities; a Personal
Brain without a Personal Agent includes only its owner.

Only the destination Brain's owner, Personal Agent, admins, and Members are
eligible Mount Participants. Destination Guests cannot participate, preventing
a Brain from transitively resharing mounted content through identities it does
not govern.
