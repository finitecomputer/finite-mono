# Bound Pending Sharing Offers To Thirty Days

Status: accepted

Brain Invitations, Folder Invitations, and Mount Offers will expire after seven
days by default. A creator may select a duration from one hour through thirty
days with `--expires-in`; FiniteBrain will not support non-expiring pending
offers. Acceptance creates the durable relationship governed by its own
revocation workflow, so expiration applies only to the unused offer. This
replaces effectively permanent delivery handles with one consistent bounded
lifetime.
