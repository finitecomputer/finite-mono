# Principal grants and local approval validation

Brain authorizes per-principal grants with provenance. Its authority is local
Brain membership, administration, and Folder access; ordinary authorization
does not call Core or the Identity Directory.

Invitations target npubs, share links, or capability Invite Tokens. Email on an
Invite Token is delivery metadata, not proof of authority. The supported
Approval Card action is `delegation-grant`: validation checks signature,
expiry, nonce replay, and signer standing in `brain_members`/`brain_admins`.

Revocation is an explicit Brain administration action. Organization offboarding
must revoke access in each affected product; a Core account change alone does
not revoke Brain grants. Key rotation cannot recall plaintext already obtained
by a formerly authorized principal.

Historical invitation-plan, departure, and email-bootstrap columns remain in
stored databases. Their presence is not an active authorization path; do not
remove retained schema or readers as part of documentation cleanup.
