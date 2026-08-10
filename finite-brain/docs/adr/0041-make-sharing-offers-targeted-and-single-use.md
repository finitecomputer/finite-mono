# Make Sharing Offers Targeted And Single-Use

Status: accepted

Amended 2026-08-07 by ADR-0045: an offer remains targeted to one human-facing
email and single-use, but that email may authorize one fixed Invitation
Participant Set rather than exactly one cryptographic principal.

Every Brain Invitation and Folder Invitation will be addressed to exactly one
email or concrete Member Identity and will be consumable exactly once by that
recipient. FiniteBrain will not provide reusable or unscoped public invitation
links; inviting several recipients creates separately inspectable and revocable
invitations. This preserves email-proof and identity-bound acceptance, prevents
a forwarded link from extending access, and gives each recipient an independent
lifecycle and audit record.

Every Mount Offer will likewise name one destination Brain and one of that
Brain's owners or admins. Only that controller may accept it, and it cannot be
redirected into another Brain. Accepted Mount Offers are consumed once while
the resulting Shared Folder Connection remains durable until revoked.
