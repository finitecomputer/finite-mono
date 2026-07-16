# Start Personal Vaults Without Seeded Folders

Status: accepted 2026-07-16.

A new Personal Vault contains no default Folders or Folder Objects. This
replaces the personal-bootstrap seed shape in the portability specification,
which created `getting-started` and `restricted`: onboarding scaffolding should
not become permanent user data, and an access mode should not be represented by
an example Folder. When an account-bound agent performs bootstrap under its
standing Agent Bootstrap Authority, Brain atomically establishes full Personal
Agent Access without creating a Folder merely for the relationship.
Organization Vault bootstrap is unchanged.

The Product Client must provide a useful empty state, and unreleased development
fixtures may be reset rather than migrated. Folders and content appear only
through explicit user actions or product workflows the user authorizes.
