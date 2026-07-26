# Make Folder Access Revocation An Atomic Client-Owned Workflow

Status: accepted

Removing a Mount Participant, revoking a Shared Folder Connection, and directly
revoking Folder access will use one client-owned Folder Access Revocation
workflow. The trusted client opens the current Folder Key, generates the next
key, prepares grants for every remaining authorized identity, and submits the
access removal, new key version, and replacement grants atomically. Users will
not construct or pass raw rotation payload files. If the complete rotation
cannot be prepared, no access state changes and the operation returns the exact
blocker; success is reported only after authoritative state verifies that the
removed identities lack current Folder Access Readiness. The server continues
to validate policy and encrypted grants without opening or manufacturing Folder
Keys.
