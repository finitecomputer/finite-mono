# Context

Glossary for Finite Sites. Code, docs, and prompts should use these words
with exactly these meanings.

- **Sites Platform Service**: the Finite Sites product boundary that owns
  publishing APIs, Git Remotes, Site serving, Sites authorization,
  and Sites durability as one service. It may live in `finite-mono`, but other
  Finite products depend on its service contracts rather than its internals.
- **Dedicated Sites Host**: a Finite-operated VM or VPS whose primary role is
  running the Sites Platform Service and its durable Sites state. It separates
  Sites serving from the shared Finite control/app host and from Agent Runtime
  hosts.
- **Sites Service Boundary**: the versioned contracts by which other Finite
  products and clients use the Sites Platform Service. The boundary includes
  public control-plane APIs, Git Remotes, serving URLs, and narrowly scoped
  internal APIs, but not registry tables or daemon implementation details.
- **Static Sites API**: the Sites Service Boundary for the static-only target
  model. It presents each Project Repository as having zero or one Project
  Site, not a list of typed outputs, and uses Site vocabulary in public
  request and response fields.
- **Sites Component Release**: one versioned release of the Sites Platform
  Service and its agent-facing CLI. Until the Sites Service Boundary is stable,
  server and CLI behavior are shipped and reasoned about as one component.
- **Validation Build**: an exact unreleased or pinned Sites Component build
  used during the V2 Validation Phase. It is not promoted as the public
  rolling CLI release until the default production endpoint can satisfy it.
- **Sites Capability Check**: the client/server compatibility handshake that
  tells an agent whether its CLI understands the Sites Service Boundary it is
  contacting. It exists to fail clearly at the boundary, not to preserve every
  older client behavior.
- **Sites Hostname Boundary**: the public hostnames owned by the Sites Platform
  Service: the Sites API origin, Git Remote origin, and one-label Site hosts
  under the Site Base Domain. Finite application/dashboard/chat origins are not
  Sites hostnames.
- **Sites V2 Endpoint**: the minimal temporary or versioned Sites endpoint
  needed for new `fsite` clients and selected agent runtimes to target the
  Static Sites API before canonical Sites hostnames move. It may carry both
  control API and Git Remote traffic on one origin; it does not imply
  duplicating every production hostname plane.
- **V2 Validation Phase**: the opt-in period where new `fsite` clients or
  selected agent runtimes publish to the Sites V2 Endpoint while legacy Sites
  production remains authoritative for canonical Sites hostnames.
- **Validation Site Base Domain**: the temporary wildcard domain used to test
  served Sites during the V2 Validation Phase. It is validation plumbing, not
  the long-term public Site URL contract.
- **Validation Site URL**: a served Site URL under the Validation Site Base
  Domain. It may prove behavior during validation, but it is not a durable
  public URL promise.
- **Sites Dependency Fact**: a narrow non-authorizing fact that Sites obtains
  from another Finite service, such as Identity Directory NIP-05 resolution or
  a bounded SaaS account assertion for an already shared Site. It never
  includes mailbox proof, publish-grant satisfaction, Share state, or other
  Sites permission state; Sites answers those questions from its own tables.
- **Sites Recovery Set**: the durable Sites-owned state required to restore
  Project Repositories, Sites, versions, sharing, and audit history
  without relying on another product's database as the source of truth.
- **Sites Behavior Compatibility**: the migration promise that existing
  `fsite` static-site workflows, Git Remotes, serving URLs, visibility, and
  sharing keep their user-visible meaning while the hosting architecture
  changes.
- **Sites Cutover**: the operator-led move from the legacy Sites deployment to
  the Sites Platform Service after the Sites V2 Endpoint has been proven.
  Already-published Sites keep serving while publishing writes may be briefly
  frozen, Sites state may be reconciled once, and canonical Sites hostnames
  move.
- **Publishing Write Freeze**: a bounded Sites Cutover period during which Git
  pushes and publishing mutations are rejected or paused while already-published
  Sites continue serving.
- **Static Launch Check**: a pre-cutover check that protects the static-only
  target model. Retired output kinds, multiple Project Sites for one Project
  Repository, or retired Core grant state are not converted into target Sites
  state; this is not a reusable migration framework.
- **Static-Only Sites Service**: the target shape of the Sites Platform
  Service: Project Repositories publish static Finite Sites only. There is no
  output kind dimension, tenant app process, warm runtime path, or runtime
  compatibility flag for retired output kinds.
- **Finite Site**: one published website living at `{name}.{base domain}`,
  owned by one Publishing Principal, with an immutable version history.
- **Principal**: the authorization subject permissions attach to. A Principal
  may be represented by an email address during bootstrap and by verified key
  identities once available.
- **Publishing Principal**: a Principal allowed to create Project
  Repositories and Sites. A Publishing Principal may be established
  through email bootstrap or through a native npub path; email is never
  required for the long-term identity model.
- **Publishing Bootstrap Invite**: an email-delivered invitation that lets a
  recipient prove control of an email address and establish a Publishing
  Principal. Early Finite Sites treats email invites as a growth path for
  agent publishing; later policy may restrict who can complete the bootstrap
  without changing the Principal model.
- **Self-Registered Publish Grant**: the default v0 Publishing Principal
  bootstrap. A local Publishing Key signs `fsite auth register`, receives a
  self-sourced publish grant, and can create Project Repositories and Project
  Sites up to the Publishing Limit without an operator allowlist round trip.
- **Publishing Limit**: the product meaning of "unlimited" publishing in v0:
  a Publishing Principal may create up to 100 Sites before an operator policy
  changes the limit.
- **Publishing Revocation**: an operator action that removes a Principal's
  ability to create new Project Repositories or Sites without
  necessarily removing existing collaboration or viewing access.
- **Publishing Ownership Recovery**: an audited transfer that restores control
  of a Project Repository and its Site to a verified replacement Principal
  without changing or deleting their data.
- **Site Disable**: an operator action that stops one or more Sites from
  serving while preserving source history and audit context.
- **Emergency Delete**: a manual operator action reserved for extreme abuse,
  where preserving source history or names is less important than removing
  harmful product data.
- **Native Principal**: a Principal known by npub inside Finite surfaces, such
  as a chat participant. Native shares can target this Principal directly.
- **Requesting User Share**: an explicit, revocable Share created atomically by
  an Agent Principal's signed Project Init for the authenticated human sender's
  Native Principal. It grants view access to the Project Site; it does not
  change Project ownership, collaboration, or Git access.
- **External Principal**: a Principal identified by email because they are not
  yet a Finite user. External shares use email verification.
- **Principal Link**: an explicit, approved relationship between Principals
  that represent the same user or agent across identity paths. Finite Sites
  does not infer a Principal Link merely because an External Principal and a
  Native Principal appear related.
- **Email Link**: a verified Principal Link from one email address to one
  Native Principal. It is created only by an explicit email verification flow,
  lets future email-based collaborator grants resolve to the native npub, and
  keeps email optional for npub-primary users.
- **Sites Email Principal**: a durable Sites owner keyed by a verified,
  deliverable mailbox. It is product authorization state, not an Identity
  Principal Link.
- **Authorized Sites Key**: a revocable native npub allowed to exercise one
  Sites Email Principal's grants without making the key and mailbox the same
  Principal. Older documents call this an Email Access Delegation.
- **Project Repository**: the editable git history for a project. It may begin
  with data, grow logic around that data, and later produce zero or one Project
  Site. A Project Repository may exist before any public-facing UI exists.
- **Project Init**: the replay-safe control-plane mutation that creates or
  verifies a Project Repository from Project Config and, when the config
  declares a Project Site, creates or verifies that Project Site. It is the
  canonical agent-facing setup flow for both source-only and served Projects,
  including adding a Project Site to an existing source-only Project later.
- **Bare Project Repository**: a Project Repository with no Project Site.
  It has a Project Slug, collaborators, Git Remote, Project Status, Project
  List entry, and git history, but no viewer URL, active Version, or served
  artifact. It is source-first state, not a failed publish.
- **Project Status**: a control-plane query for one Project Repository. It
  reports repository existence, Git Remote, Project Site, deploy branch
  and paths, current deploy/version status, and the actor's project permission
  when known.
- **Project List**: a control-plane query listing Project Repositories the
  actor owns or may edit. It is scoped to Project Repositories, not only served
  Sites.
- **Project Slug**: the stable URL-safe identifier for a Project Repository.
  It is separate from Site Name. When Project Config omits Site Name, the Site
  Name defaults to the Project Slug.
- **Project Site**: the optional served Finite Site produced by one Project
  Repository. A Project Repository has zero or one Project Site. The Project
  Site owns Site Name, Deploy Branch, Deploy Path, Visibility, Shares, active
  Version pointer, and version history.
- **Project Site Identity**: the immutable public identity of a Project Site in
  v2: Site Name, Deploy Branch, and Deploy Path.
- **Retired Output Kind**: a former served-artifact variant such as app,
  document, or PDF. Remaining code or documents that depend on output kinds are
  legacy removal work rather than target Sites contracts.
- **Legacy Static Output Config**: the old `[outputs.<id>]` static-site Project
  Config shape. Sites may accept it with a deprecation warning only when it
  describes exactly one static Site; it does not preserve public output IDs or
  output kinds in the target model.
- **Deploy Tree**: committed files selected from a Project Repository and
  materialized as a Version. Agents produce Deploy Trees; Finite Sites
  validates and serves them.
- **One-Off Publishing**: a simple use of the Project Repository model where a
  user or agent creates a Project first, writes `finite.toml`, commits only the
  files they want future editors to start from, and pushes the Deploy Branch.
  It is not a separate upload surface; the Project Repository remains the
  source of truth even when the committed tree is only built/static bytes.
- **Deploy Branch**: the Project Repository branch whose pushed commits create
  new Versions automatically. Pushing to a Deploy Branch updates content but
  does not change visibility or permissions.
- **Deploy Path**: the project-relative path within the Deploy Branch selected
  as the Site's Deploy Tree.
- **Project Visibility**: who may read, clone, or fetch a Project Repository.
  It is private by default and independent from Site Visibility. Public-read
  Project Visibility means read-only Git access; it never
  grants push access.
- **Managed Skills Repository**: a Project Repository whose `skills/` tree is
  consumed by finitecomputer runtimes. Finite-owned baseline skills may use
  public read-only Project Visibility. Customer, user, and team skills remain
  private by default and use normal Project Repository auth.
- **Site Name**: the lowercase DNS label (3–63 chars) for a Finite Site,
  globally unique within the Site Base Domain, first-come, and allocated before
  any Version is deployed. Reserved names are rejected.
- **Reserved Site Name**: a Site Name unavailable for new allocation because
  it is owned by legacy Sites, reserved for a service label such as `v2`, set
  aside by operator policy, or owned by another current Sites authority.
- **Pre-User Reset**: a destructive operator action that wipes Finite Sites
  product state during pre-user development so examples can be redeployed
  through the current model without legacy adapters.
- **Publishing Key / Owner**: the Nostr keypair (npub) of the human or agent
  Publishing Principal. It owns Project Repositories, lists Sites, and may
  change Site sharing. The publish grant cache is keyed on it. It is the
  shared Finite identity within that principal's Finite Home: stored at
  `~/.finite/identity/identity.json` (`$FINITE_HOME/identity/identity.json` in
  hosted runtimes), minted by whichever Finite tool runs first in that home,
  and never copied into fsite's own config store. A human Finite Home and an
  agent Finite Home do not share this secret.
- A private Project Repository must have either an independent collaborator or
  a tested **Publishing Ownership Recovery** path before it is treated as
  durable user data.
- **Project Collaborator**: an email address or key identity granted edit
  rights to a Project Repository. Project collaboration is the default edit
  permission; Site sharing controls served read access.
- **Project Grant**: a control-plane mutation that gives a Principal edit
  access to a Project Repository, usually with role `editor`, and may send an
  invitation with agent-facing instructions.
- **Project Revoke**: a control-plane mutation that removes a Principal's edit
  access to a Project Repository and revokes active Git Credentials scoped to
  that Principal and Project.
- **Agent Principal Key**: the distinct npub controlled by an agent and stored
  in that agent's Finite Home. It authenticates the agent as its own Native
  Principal across Finite Sites, Finite Chat, and Finite Brain. It is never
  presumed to be the human user's key or automatically linked to that human.
- **Email Bootstrap**: the act of proving control of an email address from a
  Publishing Bootstrap Invite. A successful Email Bootstrap establishes or
  resolves an External Principal and enables publishing for that Principal
  within the Publishing Limit. It does not by itself make an Agent Principal
  the same Principal as the human who controls the email.
- **Agent Delegation**: a Principal-approved authorization that lets one Agent
  Principal Key act for that Principal on one Project Repository, with bounded
  capabilities.
- An **Authorized Sites Key** is product-scoped across one Sites Email
  Principal's grants; an **Agent Delegation** is bounded to one Project
  Repository.
- An agent using either delegation signs as its **Agent Principal Key**, and
  Sites audit records the delegation separately from actor identity.
- **Git Remote**: the standard git clone/push endpoint for a Project
  Repository, canonically `https://git.finite.chat/{project}.git` in
  production. The server-returned Git Remote is authoritative, so validation
  deployments may use the Sites V2 Endpoint before canonical hostnames move.
  Agents use normal git commands against it; Finite Sites maps authenticated
  pushes to Project Repository permissions.
- **Git Credential**: a revocable, scoped HTTPS credential minted after an
  email verification or Key Challenge. It lets standard git clients clone or
  push one Project Repository according to the Principal's permissions.
- **Agent-Safe CLI**: a command surface that agents can inspect and operate
  without out-of-band documentation. It provides structured input/output,
  dry-run validation, and machine-readable descriptions of available commands
  and workflows.
- **Project Workflow Description**: an Agent-Safe CLI description of a current
  Project Repository workflow. A workflow remains valid when its underlying
  Project Init, Git Remote, and sharing steps remain valid; only references to
  retired outputs or output kinds are legacy removal work.
- **CLI Product Verb**: one of the primary agent-facing actions:
  `project`, `auth`, or `view`. Product verbs name real product
  primitives rather than aliases or wrappers around a second surface. If a
  Product Verb is confusing, the primitive itself must be improved instead of
  hidden behind a friendlier command.
- **Auth Guidance Failure**: a command failure that tells an agent which auth
  step is missing and how to complete it before retrying the original Product
  Verb.
- **Project Config**: a project-level configuration file, conventionally
  `finite.toml`, describing static Site publishing choices for a Project
  Repository. Its target shape declares the Project and optionally one Project
  Site.
- **Key Challenge**: proof of control for a nostr key. The private key never
  leaves the user's machine; the actor signs a bounded challenge instead.
- **Email Key**: a local nostr keypair verified for one email address by a
  single-use email token. It signs email-keyed project git credential requests
  without exposing npubs.
- **Publish Grant Cache**: the local registry table deciding whether a
  Publishing Key may create Projects, allocate Sites, and deploy new
  Versions.
  Self-registered grants are the v0 default, and operator grants remain the
  manual override/revocation path. If no active, unexpired grant exists,
  creating Projects or allocating Sites fails closed.
- **Allowlist**: the deployed operator command surface for adding/removing
  `operator` publish grants.
- **Publish Session**: a pending upload: a validated manifest plus the set
  of blobs the server still needs. Finalizing it creates a Version.
- **Manifest**: the list of `(path, sha256, size)` entries describing one
  complete site version. Paths are absolute and conservatively validated.
- **Blob**: immutable bytes stored by sha256, deduplicated across all sites
  and versions. Uploads are verified against the hash they claim.
- **Version**: an immutable Site snapshot created from a Deploy Branch push.
  The Site serves its **Active Version**; the pointer flip is atomic.
- **Agent Handoff File**: `/llms.txt` on a Site. A user-authored
  file is ordinary Site content. If absent, the platform may synthesize one
  for editable Sites so agents can discover the supported edit flow.
- **Visibility**: `private` (only explicitly shared Native Principals),
  `shared` (explicit Native Principal or email Shares), or `public`. New Sites
  are private by default. Changing Visibility is a Site sharing mutation.
  Making a Site public requires an explicit confirmation from the human,
  relayed as `confirm_public`.
- **Share**: one `(Site, Principal)` row granting view access to a served Site.
  Removing it revokes access on the next request, even for live
  cookies.
- **Site Share Mutation**: the Project-scoped command or API mutation that
  changes viewer access for a Project Site. Because a Project Repository has
  at most one Project Site, it is not scoped by an output identifier.
- **Magic Link**: a reusable, 15-minute login token mailed to a shared email.
  Each redemption sets a Viewer Cookie on the site's own host.
- **Viewer Cookie**: an HMAC-signed `(Site, Principal, expiry)` proof, scoped
  to one Site host. Legacy email cookies retain their existing wire shape. A
  cookie proves a bounded session; the Share table still decides access on
  every request.
- **Native Viewer Session**: a bounded NIP-98 proof for one exact Site-host
  session endpoint, POST body, nonce, client, and same-origin return path. The
  signer must already have a Native Principal Share. Direct native clients
  receive Viewer Cookies immediately; Hosted Web redeems a single-use link for
  the same cookies. Proof never creates a Share.
- **Verified Email Viewer Session**: a server-to-server exchange that accepts
  an email already verified by the SaaS account boundary and, only when that
  email is already on a shared Site's Share list, mints the existing
  reusable Magic Link. It never creates a Share. The browser redeems the link
  on the Site host and ordinary per-request Share checks preserve
  immediate revocation. Issuance and durable outstanding links are bounded per
  Site/email. The ordinary cookie is top-level `SameSite=Lax`; a distinct
  `Partitioned` cookie carries iframe access.
- **Control Plane**: the NIP-98-authenticated API (Project Init, git auth,
  sharing, status). **Serving Plane**: anonymous-or-cookie HTTP on site
  subdomains. One process serves both in v1, split by Host header.
- **Base Domain**: the wildcard domain under which sites live —
  `sites.localhost` in development, `finite.chat` in production.
- **Outbox**: the dev mailer's output directory; each would-be email is a
  text file containing the magic link.
