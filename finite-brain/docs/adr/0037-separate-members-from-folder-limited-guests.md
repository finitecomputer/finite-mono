# Separate Members From Folder-Limited Guests

Status: accepted

FiniteBrain will model Member and Guest as distinct relationships in both
Personal and Organization Brains. A Member belongs to the Brain and is entitled
to current and future all-members Folders; a Guest receives only explicit
Folder Access Readiness and never inherits all-members access. Brain
Invitations create Members, while Folder Invitations and Shared Folder
Connection participation create Guests when the target is not already a
Member. This hard-cuts the ambiguous “limited Member” behavior and prevents a
one-Folder invitation or mount from exposing unrelated all-members Folders.
