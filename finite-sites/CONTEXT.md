# Context

Glossary for Finite Sites. Code, docs, and prompts should use these words
with exactly these meanings.

- **Finite Site**: one published website living at `{name}.{base domain}`,
  owned by one Publishing Principal, with an immutable version history.
- **Principal**: the authorization subject permissions attach to. A Principal
  may be represented by an email address during bootstrap and by verified key
  identities once available.
- **Publishing Principal**: a Principal allowed to create Project
  Repositories and Project Outputs. A Publishing Principal may be established
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
  Outputs up to the Publishing Limit without an operator allowlist round trip.
- **Publishing Limit**: the product meaning of "unlimited" publishing in v0:
  a Publishing Principal may create up to 100 Project Outputs before an
  operator or later billing policy changes the limit.
- **Publishing Revocation**: an operator action that removes a Principal's
  ability to create new Project Repositories or Project Outputs without
  necessarily removing existing collaboration or viewing access.
- **Publishing Ownership Recovery**: an audited transfer that restores control
  of a Project Repository and its Outputs to a verified replacement Principal
  without changing or deleting their data.
- **Output Disable**: an operator action that stops one or more Project
  Outputs from serving while preserving source history and audit context.
- **Emergency Delete**: a manual operator action reserved for extreme abuse,
  where preserving source history or names is less important than removing
  harmful product data.
- **Native Principal**: a Principal known by npub inside Finite surfaces, such
  as a chat participant. Native shares can target this Principal directly.
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
- **Email Access Delegation**: a revocable Finite Sites authorization allowing
  one Agent Principal to exercise one verified email Principal's Sites grants
  without making them the same Principal.
- **Project Repository**: the editable git history for a project. It may begin
  with data, grow logic around that data, and later produce one or more Project
  Outputs. A Project Repository may exist before any public-facing UI exists.
- **Bare Project Repository**: a Project Repository with zero Project Outputs.
  It has a Project Slug, collaborators, Git Remote, Project Status, Project
  List entry, and git history, but no viewer URL, active Version, or served
  artifact. It is source-first state, not a failed publish.
- **Project Status**: a control-plane query for one Project Repository. It
  reports repository existence, Git Remote, Project Outputs, deploy branches
  and paths, current deploy/version status, and the actor's project permission
  when known.
- **Project List**: a control-plane query listing Project Repositories the
  actor owns or may edit. It is scoped to Project Repositories, not only sites
  or other served outputs.
- **Project Slug**: the stable URL-safe identifier for a Project Repository.
  It is separate from Site Name. Simple projects may use the same string for
  Project Slug and Site Name, but the CLI must make that choice explicit
  rather than inferring one from the other.
- **Project Output**: a user-facing artifact produced from a Project
  Repository, such as a Finite Site, Document Output, or PDF Output. Project
  Outputs own serving visibility, sharing, active version pointers, and version
  history.
- **Output Routing Name**: the globally unique DNS label for a Project Output
  within that output kind's serving namespace. Output Routing Names are not
  global across all kinds; a site, document, and PDF may share the same label
  because they live on different serving domains.
- **Document Output**: a read-only Project Output whose source is authored as
  Markdown in a Project Repository and viewed as a rendered document. It is for
  collaborative writing and review, not only software documentation.
- **Stateful App Output**: a Project Output whose source is an app runtime
  directory in a Project Repository. It is declared with `kind = "app"` in
  Project Config, served under the Site Base Domain by Site Name, and deployed
  as one immutable app bundle plus an explicit App Start Command. It shares the
  Site Name namespace with static Finite Sites.
- **App Source Path**: the project-relative directory declared for a Stateful
  App Output. Agents commit source, migrations, seed data, and explicit runtime
  payload under this path; Finite Sites versions that directory as the app
  bundle.
- **App Start Command**: the explicit `start` command in Project Config for a
  Stateful App Output. Finite Sites sets `PORT` and `DATA_DIR` before running
  it; the app must listen on `0.0.0.0:$PORT`.
- **App Data Directory**: the runtime `$DATA_DIR` provided to a Stateful App
  Output. It is the only live mutable state location and survives deploys,
  restarts, and wake/sleep. Deploys must not overwrite it.
- **PDF Output**: a read-only Project Output whose served artifact is a PDF
  committed to a Project Repository. An agent or user generates the PDF before
  pushing; Finite Sites stores and serves the committed PDF bytes as immutable
  output versions.
- **PDF Name**: the globally unique, stable URL-safe identifier for a PDF
  Output, served under the PDF base domain. It is separate from Site Name and
  Document Name.
- **PDF Base Domain**: the serving-plane wildcard domain under which PDF
  Outputs live. It does not host Project Repository control-plane APIs.
- **Document Visibility**: the Visibility of a Document Output. It uses the
  same private, shared, and public meanings as other Project Outputs and is
  independent from Project Visibility.
- **Document Name**: the globally unique, stable URL-safe identifier for a
  Document Output, served under the document base domain. It is separate from
  Site Name.
- **Document Base Domain**: the serving-plane wildcard domain under which
  Document Outputs live, canonically `docs.finite.chat` in production. It does
  not host Project Repository control-plane APIs.
- **Document Source Path**: the project-relative path declared for a Document
  Output. It points either to one Document Markdown file or to a Document Root
  directory.
- **Document Root**: the directory in a Project Repository that contains the
  Markdown files for a directory-shaped Document Output.
- **Document Entry**: the Markdown file inside a Document Root that opens when
  a viewer visits the Document Output root URL. Directory Documents default to
  `index.md`; Single-File Documents use the source file as the entry.
- **Single-File Document**: a Document Output whose source is one Document
  Markdown file. The file is the Document Entry and renders at the Document
  Output root.
- **Document Project Output Config**: the `finite.toml` output entry that
  declares a Document Output. It uses the same Project Repository, Deploy
  Branch, and collaborator model as other Project Outputs.
- **Document Directory Index**: an optional `_index.md` file inside a
  Document Root directory. It is ordinary Document Markdown and may provide
  navigation or ordering hints for that directory, matching common llm-wiki
  folder conventions; it is not required and is not a generated cache.
- **Document Markdown**: the Markdown source for a Document Output. Finite
  Sites stores the authored text and promises a strict Document Renderer
  Subset; content outside that subset may render plainly or be ignored.
- **Document Frontmatter**: optional YAML metadata at the top of a Document
  Markdown file. Recognized fields may shape document presentation and
  navigation; unknown fields remain source metadata and are ignored by the
  renderer.
- **Document Renderer Subset**: the bounded Markdown features the Rust
  renderer must handle predictably for v0: headings, paragraphs, emphasis,
  lists, blockquotes, code spans and fences, links, images, tables,
  frontmatter, directory indexes, and Document Wikilinks. Raw HTML and richer
  blocks are outside the subset until they become explicit Document
  Components.
- **Document Component**: an explicit, allowlisted rich block or inline element
  in a Document Output. Document Components are product features, not arbitrary
  raw HTML or JavaScript.
- **Document Route**: the viewer-facing path for a Markdown file in a
  Document Output. Document Routes are clean URLs derived from Markdown paths
  inside the Document Root.
- **Document Navigation**: the viewer navigation for a Document Output,
  derived from the Document Snapshot unless a later document feature gives
  authors explicit navigation control.
- **Document Wikilink**: an Obsidian-style link inside Document Markdown,
  such as `[[Page]]` or `[[Page|label]]`. Document Wikilinks are a
  compatibility feature resolved within one Document Root; standard Markdown
  links remain the canonical link format.
- **Document Snapshot**: the exact authored Document Markdown selected for one
  deployed Document Output version. Finite Sites renders from that Markdown;
  rendered HTML is not the source of truth.
- **Document Warning**: a non-blocking issue found in a Document Snapshot,
  such as a broken internal link or unresolved Document Wikilink. Document
  Warnings do not prevent a Document Output from being served.
- **Deploy Output**: committed files selected from a Project Repository and
  materialized as a Version. Agents produce Deploy Outputs; Finite Sites
  validates and serves them.
- **One-Off Publishing**: a simple use of the Project Repository model where a
  user or agent creates a Project first, writes `finite.toml`, commits only the
  files they want future editors to start from, and pushes the Deploy Branch.
  It is not a separate upload surface; the Project Repository remains the
  source of truth even when the committed tree is only built/static bytes.
- **Deploy Branch**: the Project Repository branch whose pushed commits create
  new Versions automatically. Pushing to a Deploy Branch updates content but
  does not change visibility or permissions.
- **Project Visibility**: who may read, clone, or fetch a Project Repository.
  It is private by default and independent from the Visibility of any Project
  Output. Public-read Project Visibility means read-only Git access; it never
  grants push access.
- **Managed Skills Repository**: a Project Repository whose `skills/` tree is
  consumed by finitecomputer runtimes. Finite-owned baseline skills may use
  public read-only Project Visibility. Customer, user, and team skills remain
  private by default and use normal Project Repository auth.
- **Site Name**: the Output Routing Name for a Finite Site. It is a lowercase
  DNS label (3–63 chars), globally unique within the Site Base Domain,
  first-come, allocated by a Project Output before any Version is deployed.
  Reserved names are rejected.
- **Pre-User Reset**: a destructive operator action that wipes Finite Sites
  product state during pre-user development so examples can be redeployed
  through the current model without legacy adapters.
- **Publishing Key / Owner**: the Nostr keypair (npub) of the human or agent
  Publishing Principal. It owns Project Repositories, lists outputs, and may
  change output sharing. The publish grant cache is keyed on it. It is the
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
  permission; individual Project Outputs may add narrower rules later.
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
- An **Email Access Delegation** is product-scoped across Finite Sites email
  grants; an **Agent Delegation** is bounded to one Project Repository.
- An agent using either delegation signs as its **Agent Principal Key**, and
  Sites audit records the delegation separately from actor identity.
- **Git Remote**: the standard git clone/push endpoint for a Project
  Repository, canonically `https://git.finite.chat/{project}.git` in
  production. Agents use normal git commands against it; Finite Sites maps
  authenticated pushes to Project Repository permissions.
- **Git Credential**: a revocable, scoped HTTPS credential minted after an
  email verification or Key Challenge. It lets standard git clients clone or
  push one Project Repository according to the Principal's permissions.
- **Agent-Safe CLI**: a command surface that agents can inspect and operate
  without out-of-band documentation. It provides structured input/output,
  dry-run validation, and machine-readable descriptions of available commands
  and workflows.
- **CLI Product Verb**: one of the primary agent-facing actions:
  `project`, `auth`, or `view`. Product verbs name real product
  primitives rather than aliases or wrappers around a second surface. If a
  Product Verb is confusing, the primitive itself must be improved instead of
  hidden behind a friendlier command.
- **Auth Guidance Failure**: a command failure that tells an agent which auth
  step is missing and how to complete it before retrying the original Product
  Verb.
- **Project Config**: a project-level configuration file, conventionally
  `finite.toml`, describing Project Outputs such as sites or documents.
- **Key Challenge**: proof of control for a nostr key. The private key never
  leaves the user's machine; the actor signs a bounded challenge instead.
- **Email Key**: a local nostr keypair verified for one email address by a
  single-use email token. It signs email-keyed project git credential requests
  without exposing npubs.
- **Publish Grant Cache**: the local registry table deciding whether a
  Publishing Key may create Projects, allocate Project Outputs, and deploy new
  Versions.
  Self-registered grants are the v0 default, operator grants remain the
  manual override/revocation path, and Core grants become the paid-entitlement
  path. If no active, unexpired grant exists, creating Projects or allocating
  Project Outputs fails closed.
- **Allowlist**: the deployed operator command surface for adding/removing
  `operator` publish grants. De-allowlisting an owner only removes the
  operator grant; a separate active Core grant can still allow publishing.
- **Publish Session**: a pending upload: a validated manifest plus the set
  of blobs the server still needs. Finalizing it creates a Version.
- **Manifest**: the list of `(path, sha256, size)` entries describing one
  complete site version. Paths are absolute and conservatively validated.
- **Blob**: immutable bytes stored by sha256, deduplicated across all sites
  and versions. Uploads are verified against the hash they claim.
- **Version**: an immutable Project Output snapshot created from a Deploy
  Branch push. The Project Output serves its **Active Version**; the pointer
  flip is atomic.
- **Agent Handoff File**: `/llms.txt` on a Project Output. A user-authored
  file is ordinary output content. If absent, the platform may synthesize one
  for editable outputs so agents can discover the supported edit flow.
- **Agent Full Context File**: `/llms-full.txt` on a Document Output. It is a
  bounded Markdown concatenation of the Document Snapshot for agents that want
  one fetch; oversized documents fall back to the Agent Handoff File index.
- **Document Agent Links**: machine-discoverable links on a rendered Document
  Route that point agents to the Agent Handoff File and to the page's
  Markdown companion URL. Document Agent Links obey the Document Output's
  Visibility.
- **Markdown Companion URL**: the raw Document Markdown representation of a
  Document Route, exposed by appending `.md` to the human-facing route shape
  instead of using a separate platform namespace. It returns the exact authored
  Document Markdown for that page. Directory index pages use the route-shaped
  companion URL; the Document Output root uses `/index.md`.
- **Visibility**: `private` (nobody), `shared` (emails on the Share list),
  or `public`. New Project Outputs are private by default. Changing
  Visibility is a Project Output sharing mutation. Making a Project Output
  public requires an explicit confirmation from the human, relayed as
  `confirm_public`.
- **Share**: one `(Project Output, Principal)` row granting view access to a
  served output. Removing it revokes access on the next request, even for live
  cookies.
- **Magic Link**: a single-use, 15-minute login token mailed to a shared
  email. Redeeming it sets a Viewer Cookie on the site's own host.
- **Viewer Cookie**: an HMAC-signed `(site, email, expiry)` proof, scoped to
  one site host. It proves login; the Share table decides access.
- **Control Plane**: the NIP-98-authenticated API (Project Init, git auth,
  sharing, status). **Serving Plane**: anonymous-or-cookie HTTP on site
  subdomains. One process serves both in v1, split by Host header.
- **Base Domain**: the wildcard domain under which sites live —
  `sites.localhost` in development, `finite.chat` in production.
- **Outbox**: the dev mailer's output directory; each would-be email is a
  text file containing the magic link.
