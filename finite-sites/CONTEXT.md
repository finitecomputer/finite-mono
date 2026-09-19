# Finite Sites

Sites serves committed static bytes at `*.finite.site`; API and Git use
`https://finite.site`. Sites owns its registry, repositories and permissions.

- **Publishing Principal / Key**: the native identity signing Sites operations.
  An Agent signs as itself, never with its human's key.
- **Project Repository**: the editable Git source and collaboration boundary.
  It may have zero or one **Project Site**. A source-only repository has no
  viewer URL or active Version.
- **Project Slug / Site Name**: the repository URL identifier and the Site's
  DNS label. They are separate identities; use server-returned URLs.
- **Project Init**: replay-safe setup from `finite.toml`. It may add a Site to
  an existing source-only repository; it does not infer output paths.
- **Deploy Branch / Deploy Path**: the committed branch and directory published
  by a Git push. Sites validates bytes and creates a **Version**, then atomically
  advances the **Active Version**. Sites does not run builds.
- **Collaborator / Share**: collaboration permits repository editing; a Share
  permits Site viewing. Public-read Git never grants push access.
- **Visibility**: private, shared or public Site viewing policy. Public requires
  explicit human confirmation; publishing does not change sharing.
- **Sites Email Principal / Authorized Sites Key**: a mailbox-verified Sites
  owner and a revocable native key allowed to exercise that owner's permissions.
  This authorization does not link identities or grant another product access.
- **Viewer Cookie**: a bounded host-scoped session; each request still checks
  current Site permission. **Magic Links** are reusable until their 15-minute
  expiry; account-session handoffs are single-use and expire after 60 seconds.
- **Sites Recovery Set**: registry, blobs, Git repositories, permissions and
  cookie key, restored together using independently held recovery credentials.

The parser retains deprecated `[outputs.*]` input for exactly one static Site.
Previous content URLs use reviewed redirects; cookies and Git credentials do
not transfer across hosts. See [service contract](docs/adr/0028-static-only-sites-platform-service.md)
and [operations](../infra/runbooks/deploy-sites.md).

The dashboard Sites list is the selected agent's Project Sites inventory through
a Sites-owned Hermes plugin and the supported CLI. It is not an account-wide
human inventory; see the [inventory contract](../finitecomputer-v2/docs/agent-product-inventory.md).
