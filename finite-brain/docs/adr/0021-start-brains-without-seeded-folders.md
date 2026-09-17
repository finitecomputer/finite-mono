# Start Brains Without Seeded Folders

Status: accepted 2026-07-16; expanded to Organization Brains 2026-07-18.

A new Personal or Organization Brain contains no default Folders or Folder
Objects. This replaces the old seed shape that created `getting-started` and
`restricted`: onboarding scaffolding should not become permanent user data, and
an access mode should not be represented by an example Folder.

Personal Brain bootstrap still establishes the human owner and one Personal
Agent atomically. Organization Brain bootstrap still establishes its initial
member-admin set atomically, including both the creating agent and authenticated
requester when an agent creates the Brain on the requester's behalf. Neither
relationship requires a Folder or Folder Key Grant at Brain creation time.
