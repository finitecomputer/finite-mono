# Make Organization Brain Collaboration A Desired-State Operation

Status: accepted

Organization Brain collaboration is one idempotent desired-state operation,
not a caller-authored sequence of member, admin, and Folder-grant mutations.
The default admin collaboration scope includes every existing Organization
Brain Folder. The trusted client resolves the target, opens every locally
available current Folder Key, and prepares recipient grants; one Brain command
then atomically applies the requested Brain Role and the supplied current
grants while returning complete or explicit per-Folder partial state. Retrying
the same intent repairs remaining gaps. Low-level permission operations remain
available and role-only.

This preserves the distinction between Brain Role and Folder Access Readiness:
the server may validate policy, key versions, signed evidence, and encrypted
grant envelopes, but it never opens, derives, or manufactures Folder Keys.
Allowing an honest partial result was chosen over either false success or
discarding useful role/grant progress when the acting Member Identity cannot
open every Folder. Complete is reported only after authoritative state proves
the requested role and a current Folder Key Grant for every Folder in scope.
