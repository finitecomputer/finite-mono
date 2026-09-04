"""Finite architecture contracts registry — the canonical source of truth.

This file is pure data. Edit it in the same PR that adds, changes, or deletes
a seam between components. `scripts/render-architecture` regenerates
docs/architecture/ARCHITECTURE.md, docs/architecture/site/index.html, and
docs/architecture/contracts.json from it; `scripts/render-architecture
--check` lints and fails on drift. Never edit the generated files by hand.

A CONTRACT is any coupling where "if I change X, I must also change Y":
a runtime call, a shared protocol, a data format another party reads, a
deploy-time coupling, or two parties reimplementing the same protocol-ish
thing. A contract entry exists to be argued about and, where possible,
deleted.

Statuses:
  current  — live today
  dying    — deletion in flight or fully designed and queued
  proposed — an audit proposal, not started
  kept     — kept by design; `death` says why it stays
  dead     — deleted; the entry stays in the ledger so the downward trend is
             visible. Requires `closed` (date) and at least one campaign.

Verified against origin/main @ 68236fb8 (2026-08-31), post auth-kernel (#784).
"""

MAIN_REV = "31bf31d6"
UPDATED = "2026-09-02"

# ---------------------------------------------------------------------------
# The kernel — the rules every contract is reviewed against (auth-kernel era).
# A new contract that violates one of these needs an explicit exception.
# ---------------------------------------------------------------------------

PRINCIPLES = [
    ("Local authority",
     "A product answers every authorization question against its own tables. "
     "If the answer isn't local, the answer is no. No cross-service auth "
     "calls at request time."),
    ("Grants name keys or tokens, never emails",
     "An email-shaped grant is a promise to resolve a third party's identity "
     "later — that is the complecting vector the auth kernel removed."),
    ("Email is delivery, not identity",
     "Mail carries a capability link. The link is the auth; the email proved "
     "nothing. Wrong-person risk is handled by single-use tokens and "
     "revocation."),
    ("One public router per service",
     "Each service exposes exactly one public router on a dedicated listener; "
     "the edge proxies it verbatim and never filters per route."),
    ("Chat availability is the primary promise",
     "Durable history and availability outrank every other property; anything "
     "that couples chat availability to another service's uptime is a bug."),
    ("Recoverability precedes operator blindness",
     "ADR 0001: no Recovery Authority is removed until the same Recovery Set "
     "has restored onto an empty target. A TEE is not a backup."),
]

# ---------------------------------------------------------------------------
# Hosts
# ---------------------------------------------------------------------------

HOSTS = {
    "lat2": {
        "name": "finite-lat-2",
        "role": "App-plane god-box (live): core, postgres, identity, "
                "chat-server, hosted-device, sites, brain, dashboard, caddy, "
                "litestream, borg, monitoring. ADR 0007 cutover target.",
        "status": "live",
    },
    "lat3": {
        "name": "finite-lat-3",
        "role": "Kata Runner host (live) — agent runtime capacity, RAID1 + "
                "storage contract.",
        "status": "live",
    },
    "lat4": {
        "name": "finite-lat-4",
        "role": "Third storage-qualified Runner (drained admission, "
                "2026-08-28; takes 10.254.3.4 after the /29 widening).",
        "status": "drained",
    },
    "lat1": {
        "name": "finite-lat-1",
        "role": "RETIRED — thermal disk failure 2026-08-27 (ADR 0007). The "
                "production CD conductor is still hard-wired to it (hazard, "
                "see cd-conductor-lat1).",
        "status": "retired",
    },
    "monitoring": {
        "name": "finite-monitoring",
        "role": "Metrics VPS. Has never alerted anyone (audit 09).",
        "status": "live",
    },
    "clawland": {
        "name": "clawland-ovh",
        "role": "Legacy finite.vip fleet box + AEON specialization worker + "
                "finite.vip NIP-05 route proxy. Decommission proposed (audit 03).",
        "status": "legacy",
    },
    "smoke": {
        "name": "ovh-vps-smoke",
        "role": "Legacy-managed box (old brain host). Brain now runs on lat2.",
        "status": "legacy",
    },
    "sites-vps": {
        "name": "finite-sites-1 (planned)",
        "role": "Dedicated sites VPS — first move of the service-isolation "
                "wave (Paul, 2026-08-31). Needs KVM for tier-2 Kata apps.",
        "status": "planned",
    },
    "brain-vps": {
        "name": "finite-brain-1 (planned)",
        "role": "Dedicated brain VPS — second move. Audit 11 actually "
                "recommended brain first (Litestream restore is the drilled "
                "path); resolve the ordering discrepancy before cutting.",
        "status": "planned",
    },
    "chat-vps": {
        "name": "finite-chat-1 (planned)",
        "role": "Dedicated chat VPS — chat-server + hosted-device move last, "
                "as an inseparable pair.",
        "status": "planned",
    },
}

# ---------------------------------------------------------------------------
# Components
# ---------------------------------------------------------------------------

COMPONENTS = {
    "chat-server": {
        "name": "finitechat-server",
        "kind": "service",
        "lane": "chat",
        "host": "lat2",
        "store": "SQLite (single writer, blobs inline)",
        "summary": "Ordered ciphertext log: MLS commits, Welcomes, KeyPackage "
                   "leases, membership intervals, SSE hints. Zero same-host "
                   "dependencies.",
    },
    "hosted-device": {
        "name": "finitechat-hosted-device",
        "kind": "service",
        "lane": "chat",
        "host": "lat2",
        "store": "Per-user durable tree /var/lib/finitechat-hosted-device",
        "summary": "A real chat Device operated per WorkOS user + a narrow "
                   "dashboard HTTP adapter. Inseparable pair with chat-server.",
    },
    "chat-clients": {
        "name": "chat clients + CLI + Hermes plugin",
        "kind": "client",
        "lane": "chat",
        "host": "agent-runtime / user devices",
        "store": "Local device stores",
        "summary": "Native clients, finitechat-cli, the Hermes finitechat "
                   "plugin, and the agent sidecar — all speak the device "
                   "protocol to chat-server.",
    },
    "core": {
        "name": "finite-saas-core",
        "kind": "service",
        "lane": "control",
        "host": "lat2",
        "store": "Postgres 16 (loopback)",
        "summary": "Accounts, projects, agent-launch leases, capacity fences, "
                   "Finite Private grants, skills revision pins.",
    },
    "dashboard": {
        "name": "dashboard",
        "kind": "service",
        "lane": "control",
        "host": "lat2",
        "store": "None (stateless UI over Core + hosted-device)",
        "summary": "WorkOS-session web UI. Authenticates users and vouches "
                   "for them to hosted-device and sites via shared tokens.",
    },
    "identity": {
        "name": "identity → Directory",
        "kind": "service",
        "lane": "control",
        "host": "lat2",
        "store": "SQLite",
        "summary": "Post auth-kernel: the NIP-05 name directory. Answers "
                   "'what npub is name@finite.vip' (public nostr.json) and "
                   "name claiming. Two listeners: 8790 full router "
                   "(loopback operator routes), 8791 public_router only.",
    },
    "brain": {
        "name": "finite-brain",
        "kind": "service",
        "lane": "brain",
        "host": "lat2",
        "store": "SQLite /var/lib/finitebrain",
        "summary": "Encrypted Folder-scoped knowledge spaces. brain_members "
                   "is the access record; invites are capability tokens.",
    },
    "sites": {
        "name": "finitesitesd",
        "kind": "service",
        "lane": "sites",
        "host": "lat2",
        "store": "registry.db + blobs/ + git/projects/ (Kata tier-2 apps)",
        "summary": "Site publishing/hosting: daemon-local email proofs, "
                   "shares, viewer cookies, git smart HTTP, stateful app "
                   "outputs as Kata microVMs (needs /dev/kvm).",
    },
    "runner": {
        "name": "finite-saas-runner",
        "kind": "service",
        "lane": "runtime-fleet",
        "host": "lat3, lat4",
        "store": "Agent /data trees (UNBACKED — RPO ≈ ∞, audit 14)",
        "summary": "Launches and supervises agent runtimes. Kata is the only "
                   "prod launcher now; Apple containers are the local-dev "
                   "lane. Takes leases from Core; registers agent names at "
                   "the Directory.",
    },
    "agent-runtime": {
        "name": "agent runtime",
        "kind": "runtime",
        "lane": "runtime-fleet",
        "host": "lat3, lat4",
        "store": "Per-agent home, Hermes state, workspace, skills cache",
        "summary": "Hermes + finitechat sidecar + finite-agentd + fbrain/fsite "
                   "CLIs + managed skills revision, packaged in lockstep in "
                   "one runtime image.",
    },
    "skills": {
        "name": "finite-skills pipeline",
        "kind": "pipeline",
        "lane": "control",
        "host": "CI → runtime images",
        "store": "Immutable Finite Skills Revisions",
        "summary": "finite-skills is the only editable source; CI publishes "
                   "immutable revisions; runtime images embed one offline "
                   "revision.",
    },
    "edge": {
        "name": "edge (Caddy → Cloudflare tunnels planned)",
        "kind": "infra",
        "lane": "control",
        "host": "lat2",
        "store": "—",
        "summary": "One Caddy terminates finite.computer, brain/chat.finite."
                   "computer, identity.finite.vip, api/*./*.docs.finite.chat. "
                   "Proxies public listeners verbatim, never filters.",
    },
    "fbrain-cli": {
        "name": "fbrain CLI",
        "kind": "client",
        "lane": "brain",
        "host": "agent-runtime / laptops",
        "store": "Local working tree",
        "summary": "Agent's surface into brain; NIP-98; also the org-brain "
                   "control plane (this very workflow).",
    },
    "fsite-cli": {
        "name": "fsite CLI",
        "kind": "client",
        "lane": "sites",
        "host": "agent-runtime / laptops",
        "store": "Local site source",
        "summary": "Agent's surface into sites; NIP-98 + local email token "
                   "redeem.",
    },
    "legacy": {
        "name": "legacy finitecomputer fleet",
        "kind": "legacy",
        "lane": "legacy",
        "host": "clawland, smoke",
        "store": "Legacy statefulsets, matrix-synapse, finited planes",
        "summary": "Whiteglove product for unmigrated box1/TRF users + the "
                   "AEON worker + finite.vip fleet. Deliberately outside "
                   "mono's authority; shrinking.",
    },
    "workos": {
        "name": "WorkOS",
        "kind": "external",
        "lane": "control",
        "host": "external",
        "store": "Accounts and orgs (source of truth, never access authority)",
        "summary": "Account system for the dashboard. Product servers never "
                   "consult it at request time.",
    },
    "resend": {
        "name": "Resend",
        "kind": "external",
        "lane": "—",
        "host": "external",
        "store": "—",
        "summary": "Transactional mail — notification delivery only, never "
                   "identity.",
    },
    "cloudflare": {
        "name": "Cloudflare",
        "kind": "external",
        "lane": "control",
        "host": "external",
        "store": "R2 (planned blob substrate), DNS zones, tunnels (planned)",
        "summary": "DNS for 4 zones / 11 hostnames today; audit 07 moves blobs "
                   "to R2 and audit 08 moves the edge to CF tunnels.",
    },
    "ci": {
        "name": "GitHub Actions + Depot",
        "kind": "external",
        "lane": "control",
        "host": "external",
        "store": "CI-built digest-pinned images, NixOS closures via Cachix",
        "summary": "Nothing is built on a prod box; releases are "
                   "component-scoped tags with rolling aliases in "
                   "finite-releases.",
    },
}

# ---------------------------------------------------------------------------
# Contracts
# ---------------------------------------------------------------------------

CONTRACTS = [
    # ---- runtime edges -----------------------------------------------------
    dict(
        id="hd-chat-pair",
        name="hosted-device ↔ chat-server pairing",
        kind="edge",
        parties=["hosted-device", "chat-server"],
        status="kept",
        owner="paul",
        via="loopback HTTP FINITECHAT_SERVER_URL; systemd Requires=/PartOf=",
        summary="Hosted-device is a chat Device driven over loopback HTTP to "
                "the server. They are one unit for availability and moves.",
        death="Kept by design: the pair is the hosted chat product. It moves "
              "to finite-chat-1 together, last, in the isolation wave.",
        evidence=["infra/nixos/modules/finitechat-hosted-device.nix:14-30"],
        campaigns=["service-isolation"],
    ),
    dict(
        id="dash-hd-vouch",
        name="dashboard vouches users into hosted-device",
        kind="edge",
        parties=["dashboard", "hosted-device"],
        status="kept",
        owner="paul",
        via="x-finite-workos-user-id header + shared FINITECHAT_HOSTED_API_TOKEN",
        summary="The dashboard authenticates the human via WorkOS and vouches "
                "for them to hosted-device with a header plus a shared "
                "bearer. Hosted-device never calls WorkOS itself.",
        death="Kept by design (auth kernel rule: agents are principals, "
              "humans act through hosted surfaces). Survives the host split "
              "as-is — token over HTTPS/WireGuard.",
        evidence=["infra/nixos/modules/finitechat-hosted-device.nix:41-49",
                  "infra/nixos/modules/dashboard.nix:32,55"],
        campaigns=[],
    ),
    dict(
        id="hd-brain-idp",
        name="hosted-device mints brain credentials for browsers",
        kind="edge",
        parties=["hosted-device", "brain"],
        status="current",
        owner="paul",
        via="HTTP /v1/brain/… key ops from per-user identity.json",
        summary="The browser reaches brain through hosted-device, which mints "
                "proofs from the user's per-user identity.json secret. Real "
                "cross-product coupling: a chat-lane outage degrades browser "
                "auth for brain.",
        death="Candidate future trim: browser auth via capability links "
              "instead of the hosted key surface. Not designed yet.",
        evidence=["finitechat/crates/finitechat-hosted-device/src/lib.rs:2579-2660",
                  "essentials-audit 11 §2.2"],
        campaigns=[],
    ),
    dict(
        id="hd-sites-idp",
        name="hosted-device mints sites viewer sessions",
        kind="edge",
        parties=["hosted-device", "sites"],
        status="current",
        owner="paul",
        via="HTTP /v1/sites/identity-provider authorizeViewerSession",
        summary="Same pattern as hd-brain-idp: browser viewer sessions for "
                "sites minted through the chat lane's hosted-device.",
        death="Same candidate trim as hd-brain-idp.",
        evidence=["finitechat/crates/finitechat-hosted-device/src/lib.rs:2579-2660",
                  "essentials-audit 11 §2.2"],
        campaigns=[],
    ),
    dict(
        id="runner-core-lease",
        name="runner ↔ core launch leases",
        kind="edge",
        parties=["runner", "core"],
        status="kept",
        owner="team",
        via="HTTP over WireGuard; route-scoped runner credential bound by core",
        summary="Agent creation leases, capacity fences, drain state. This is "
                "the control-plane contract of the runtime fleet.",
        death="Kept by design: core is the launch authority. The kata-runner "
              "role itself is CI-pinned by just runner-host-contract.",
        evidence=["infra/nixos/modules/kata-runner-host.nix:100",
                  "finitecomputer-v2/crates/finite-saas-core/src/store.rs lease_agent_creation"],
        campaigns=[],
    ),
    dict(
        id="runner-identity-names",
        name="runner registers agent names at the directory",
        kind="edge",
        parties=["runner", "identity"],
        status="current",
        owner="paul",
        via="POST /api/v1/operator/agent-email-bindings over WireGuard, "
            "operator token, 250ms retry loop",
        summary="The one identity hop the auth kernel kept: launch binds "
                "agent@finite.vip synchronously, so agent creation on a "
                "runner host still depends on the directory answering.",
        death="Designed: mint a single-use claim token at the directory; the "
              "runtime claims its own name with its own key over the public "
              "URL, off the launch critical path. Deletes the WireGuard "
              "reader and the operator token on runner hosts.",
        evidence=["finitecomputer-v2/crates/finite-saas-runner/src/lib.rs:56,766",
                  "auth-kernel explainer, Agent launch section"],
        campaigns=["directory-claim-token"],
    ),
    dict(
        id="dash-core-api",
        name="dashboard reads core",
        kind="edge",
        parties=["dashboard", "core"],
        status="kept",
        owner="team",
        via="HTTP (FC_CORE_API_TOKEN)",
        summary="Projects, launches, entitlements — the dashboard is a "
                "WorkOS-authenticated view over core state.",
        death="Kept by design.",
        evidence=["infra/nixos/modules/dashboard.nix:44"],
        campaigns=[],
    ),
    dict(
        id="core-workos",
        name="core resolves WorkOS users",
        kind="edge",
        parties=["core", "workos"],
        status="kept",
        owner="team",
        via="WorkOS API",
        summary="Accounts and orgs source of truth. Explicitly never an "
                "access authority inside any product server (kernel rule).",
        death="Kept by design — the WorkOS boundary sentence.",
        evidence=["docs/architecture narrative; architecture-overview.md"],
        campaigns=[],
    ),
    dict(
        id="dash-workos",
        name="dashboard WorkOS sessions",
        kind="edge",
        parties=["dashboard", "workos"],
        status="kept",
        owner="team",
        via="OAuth session cookies",
        summary="Human login for the only human-facing surface.",
        death="Kept by design. WorkOS is a request-time dependency of the "
              "dashboard only — never of brain/sites/chat.",
        evidence=["infra/nixos/modules/dashboard.nix"],
        campaigns=[],
    ),
    dict(
        id="mail-notification",
        name="brain + sites send notification mail",
        kind="edge",
        parties=["brain", "sites", "resend"],
        status="kept",
        owner="team",
        via="Resend API (RESEND_API_KEY per service)",
        summary="Invite links, publication and token mail. Delivery only — "
                "kernel rule: email is never identity.",
        death="Kept by design.",
        evidence=["finite-brain/crates/finite-brain-app/src/main.rs:105-112",
                  "/etc/finite-saas/sites.env (name only)"],
        campaigns=[],
    ),
    dict(
        id="dash-sites-viewer-token",
        name="dashboard ↔ sites shared viewer-session token",
        kind="edge",
        parties=["dashboard", "sites"],
        status="current",
        owner="team",
        via="Shared FINITE_SITES_VIEWER_SESSION_TOKEN (64-hex) in both env files",
        summary="The dashboard mints viewer cookies for sites browser "
                "sessions. One of the two secrets that span hosts after the "
                "VPS split (the other is the hosted-device vouch token).",
        death="Survives the split mechanically (bearer over HTTPS) but is a "
              "candidate for the capability-token consolidation (05c).",
        evidence=["infra/nixos/modules/dashboard.nix:35,56",
                  "infra/nixos/modules/finitesitesd.nix:106-117"],
        campaigns=["capability-tokens"],
    ),
    dict(
        id="sites-core-reconcile",
        name="sites reconcile-identity operator CLI → core",
        kind="edge",
        parties=["sites", "core"],
        status="current",
        owner="alex",
        via="Optional FC_CORE_API_BASE_URL/TOKEN; unset on the serve path",
        summary="Operator-run reconciliation only — the sites daemon's serve "
                "path never calls core.",
        death="Candidate deletion with the capability-token consolidation; "
              "operator tooling, not a runtime seam.",
        evidence=["finite-sites/crates/finitesitesd/src/lib.rs:45-46,525-544"],
        campaigns=["capability-tokens"],
    ),
    dict(
        id="agentd-chat-sync",
        name="agent sidecar ↔ chat-server device sync",
        kind="edge",
        parties=["chat-clients", "chat-server", "agent-runtime"],
        status="kept",
        owner="paul",
        via="chat device protocol (MLS, SSE hints) + store-backed admission",
        summary="The agent's chat Device syncs every joined Room; admission "
                "is seeded from FINITECHAT_OWNER_NPUBS through finite-agentd "
                "(owner-npubs authz, #712) with the allowed-users mirror as "
                "the launcher's only source.",
        death="Kept by design — this is the chat protocol itself.",
        evidence=["finitechat sidecar; dd3b7f88, f6e3a389 (#712)"],
        campaigns=[],
    ),
    dict(
        id="nostrjson-read",
        name="public nostr.json resolution",
        kind="edge",
        parties=["identity", "fbrain-cli", "fsite-cli", "chat-clients"],
        status="kept",
        owner="team",
        via="Unauthenticated public GET on the 8791 public_router",
        summary="Post auth-kernel name resolution: clients read nostr.json "
                "like any nostr client; product servers resolve nothing.",
        death="Kept by design — this is the directory's whole job.",
        evidence=["infra/nixos/modules/finite-identity.nix:8-14"],
        campaigns=[],
    ),

    # ---- dead runtime edges (the ledger's wins) ----------------------------
    dict(
        id="brain-identity-resolution",
        name="brain → identity resolution",
        kind="edge",
        parties=["brain", "identity"],
        status="dead",
        closed="2026-08-31",
        owner="paul",
        via="NIP-05 fetch + satisfies-grant",
        summary="Brain asked identity to resolve emails/npubs at request "
                "time. Killed by the auth kernel.",
        death="Dead: #784. Grants now name npubs or tokens; the server keeps "
              "no FINITE_IDENTITY_* wiring.",
        evidence=["infra/nixos/modules/finite-brain.nix:41-42 (remnant comment)"],
        campaigns=["auth-kernel"],
    ),
    dict(
        id="brain-core-roster",
        name="brain → core roster + departure facts",
        kind="edge",
        parties=["brain", "core"],
        status="dead",
        closed="2026-08-31",
        owner="paul",
        via="30s departure-facts poll; roster lookups; invitation plans",
        summary="The two-phase invitation plans, roster revisions, and the "
                "departure poller — the distributed-authority knot.",
        death="Dead: #784 (−17.5k lines with the rest of the cut). Offboarding "
              "is now an explicit revoke per product.",
        evidence=["essentials-audit 11 §2.3 (pre-cut state)"],
        campaigns=["auth-kernel"],
    ),
    dict(
        id="sites-identity-proofs",
        name="sites → identity proofs + mail relay",
        kind="edge",
        parties=["sites", "identity"],
        status="dead",
        closed="2026-08-31",
        owner="paul",
        via="satisfies-grant, mailbox-proofs, nip05-resolution, notification relay",
        summary="Sites' mailbox proofs lived in identity and its first-"
                "publication mail relayed through identity's mailer.",
        death="Dead: #784. Daemon-local email_login_tokens + local mailer are "
              "the only proof sites consults.",
        evidence=["essentials-audit 11 §2.4 (pre-cut state)"],
        campaigns=["auth-kernel"],
    ),
    dict(
        id="hd-identity-bindings",
        name="hosted-device → identity account-principal bindings",
        kind="edge",
        parties=["hosted-device", "identity"],
        status="dead",
        closed="2026-08-31",
        owner="paul",
        via="POST account-principal-bindings at runtime creation",
        summary="Runtime creation used to fail if identity was down — chat "
                "availability coupled to a service with no other reason to "
                "be in that path.",
        death="Dead: #784. The identity-operator.env load is gone from the "
              "hosted-device unit.",
        evidence=["infra/nixos/modules/finitechat-hosted-device.nix:44-46"],
        campaigns=["auth-kernel"],
    ),
    dict(
        id="search-stack",
        name="finite-search (SearXNG + Firecrawl)",
        kind="edge",
        parties=[],
        status="dead",
        closed="2026-08-30",
        owner="alex",
        via="NixOS stack on the app plane",
        summary="Retired with zero live dependencies; two-commit revert "
                "resurrection documented.",
        death="Dead: #774.",
        evidence=["08d3042d, becd877e"],
        campaigns=["search-retirement"],
    ),

    # ---- shared conventions --------------------------------------------------
    dict(
        id="nip98",
        name="NIP-98 signed requests",
        kind="convention",
        parties=["brain", "sites", "chat-server", "fbrain-cli", "fsite-cli",
                 "chat-clients", "runner", "agent-runtime"],
        status="kept",
        owner="team",
        via="schnorr-signed url + method + payload hash",
        summary="The universal operator/product auth: principals are keys, "
                "requests are signed. The one auth protocol everything "
                "speaks.",
        death="Kept by design — but the implementations are consolidating "
              "onto finite-nostr (see nip98-impls-4).",
        evidence=["finite-brain protected_routes.rs; finite-nostr"],
        campaigns=["single-nip98"],
    ),
    dict(
        id="capability-tokens-v1",
        name="Finite Capability Token v1 (fbit-)",
        kind="convention",
        parties=["brain", "sites", "chat-server", "identity"],
        status="proposed",
        owner="paul",
        via="unguessable token: hash-stored, single-use, revocable, expiry optional",
        summary="05c: adopt brain's fbit- token verbatim as the one grant "
                "mechanism across products (see grant-mechanisms-11 for the "
                "eleven it replaces).",
        death="Proposed: this entry dies into 'kept' once adopted — it is the "
              "consolidation target, not a victim.",
        evidence=["essentials-audit 05c"],
        campaigns=["capability-tokens"],
    ),
    dict(
        id="mls-chat-protocol",
        name="chat protocol (MLS over ordered ciphertext log)",
        kind="convention",
        parties=["chat-server", "chat-clients", "hosted-device", "agent-runtime"],
        status="kept",
        owner="paul",
        via="MLS commits/Welcomes; server never sees plaintext",
        summary="Server owns ordering and delivery; Devices own keys, groups, "
                "and applied cursors. Product layers must not acquire "
                "protocol authority by convenience.",
        death="Kept by design.",
        evidence=["docs/architecture-overview.md chat layering table"],
        campaigns=[],
    ),
    dict(
        id="release-assets",
        name="component releases + rolling aliases",
        kind="convention",
        parties=["ci", "chat-clients", "fbrain-cli", "fsite-cli"],
        status="kept",
        owner="team",
        via="finitechat/vX.Y.Z tags; finitechat-latest rolling aliases in finite-releases",
        summary="Release asset names are product contracts — never rename.",
        death="Kept by design.",
        evidence=["AGENTS.md; infra/images/README.md"],
        campaigns=[],
    ),
    dict(
        id="runtime-image-lockstep",
        name="runtime image packages the toolchain in lockstep",
        kind="convention",
        parties=["agent-runtime", "chat-clients", "fbrain-cli", "fsite-cli",
                 "skills", "ci"],
        status="kept",
        owner="team",
        via="One image: finitechat plugin + fsite + fbrain + one skills revision",
        summary="The runtime image is the compatibility unit: everything the "
                "agent runs ships together.",
        death="Kept by design, but per-service closures (12) make *build and "
              "deploy* per-service while the image stays lockstep.",
        evidence=["infra/images/README.md"],
        campaigns=["per-service-closures"],
    ),
    dict(
        id="owner-npubs-admission",
        name="chat admission birth seed (owner npubs)",
        kind="convention",
        parties=["agent-runtime", "chat-server", "core"],
        status="current",
        owner="paul",
        via="FINITECHAT_OWNER_NPUBS → agentd seeds sidecar admission; store-backed mirror",
        summary="#712: which agents may talk to a runtime is decided by owner "
                "npubs seeded before any child process starts — product-owned "
                "authorization, no identity-service call.",
        death="Kept by design (kernel-conformant). Watch that the mirror "
              "stays the launcher's only source.",
        evidence=["dd3b7f88, f6e3a389, 466ac211 (#712)"],
        campaigns=[],
    ),

    # ---- data contracts ------------------------------------------------------
    dict(
        id="chat-sqlite",
        name="chat single-writer SQLite (blobs inline)",
        kind="data",
        parties=["chat-server"],
        status="current",
        owner="paul",
        via="/var/lib/finite-chat/data/server.sqlite3, WAL enforced",
        summary="The entire durable chat state including attachments. "
                "Single-writer doctrine: a restored copy must never run "
                "alongside the live server. RPO target ≤10s (audit 14).",
        death="Blobs move to R2 (07) — the database shrinks before the "
              "process moves hosts. The SQLite file itself is kept.",
        evidence=["finitechat/crates/finitechat-server/src/store/mod.rs:109-111",
                  "essentials-audit 07, 14"],
        campaigns=["r2-blobs", "service-isolation"],
    ),
    dict(
        id="brain-sqlite",
        name="brain SQLite",
        kind="data",
        parties=["brain"],
        status="kept",
        owner="paul",
        via="/var/lib/finitebrain/finite-brain.sqlite3",
        summary="Members, folders, grants, tokens. Litestream lane moves with "
                "the service; restore is the drilled path (already done once "
                "in the ADR 0007 cutover).",
        death="Kept by design for the store; the host moves "
              "(finite-brain-1).",
        evidence=["infra/nixos/modules/finite-brain.nix:27,53-54"],
        campaigns=["service-isolation"],
    ),
    dict(
        id="sites-store",
        name="sites registry + blobs + git repos",
        kind="data",
        parties=["sites"],
        status="current",
        owner="alex",
        via="/var/lib/finite-sites: registry.db, blobs/, git/projects/",
        summary="Content-addressed blobs and bare git repos on disk today; "
                "tier-2 Kata apps add microVM state.",
        death="blobs/ → R2 (07) before the host move, so data shrinks first; "
              "git/projects/ stays on disk.",
        evidence=["finite-sites/crates/finitesitesd/src/git.rs:128",
                  "essentials-audit 07, 11 §2.4"],
        campaigns=["r2-blobs", "service-isolation"],
    ),
    dict(
        id="core-postgres",
        name="core Postgres 16 (loopback)",
        kind="data",
        parties=["core"],
        status="current",
        owner="team",
        via="FC_CORE_DATABASE_URL at 127.0.0.1:5432",
        summary="The only Postgres in the tree — no product server touches "
                "it. Control-plane component; stays on the app-plane host.",
        death="Proposed (15): PlanetScale Postgres is GA; dump-restore + "
              "one-secret swap, dialect risk LOW.",
        evidence=["infra/nixos/modules/postgres.nix:1-3",
                  "essentials-audit 15"],
        campaigns=["core-planetscale"],
    ),
    dict(
        id="skills-revisions",
        name="immutable Finite Skills Revisions",
        kind="data",
        parties=["skills", "agent-runtime", "core"],
        status="kept",
        owner="team",
        via="CI-published revisions; core pins desired revision ids",
        summary="Skills are release artifacts, not git state; old revisions "
              "must keep resolving forever.",
        death="Kept by design.",
        evidence=["ADR 0002"],
        campaigns=[],
    ),
    dict(
        id="runner-data-unbacked",
        name="runner /data has no backup (RPO ≈ ∞)",
        kind="data",
        parties=["runner", "agent-runtime"],
        status="current",
        owner="alex",
        via="Agent identities, Hermes homes, workspaces on lat3/lat4 /data",
        summary="Audit 14: the fleet's most durable user state has zero "
                "backup. The un-argued-about hole in the recovery story.",
        death="14's target matrix: ≤24h everything, ≤10s chat+brain; pairing "
              "with store consolidation to shrink what needs backing up.",
        evidence=["essentials-audit 14"],
        campaigns=["backup-truth"],
    ),
    dict(
        id="litestream-per-db",
        name="per-db Litestream replication",
        kind="data",
        parties=["chat-server", "brain", "sites", "identity"],
        status="kept",
        owner="team",
        via="One replicator unit per SQLite db, PartOf= the owning service",
        summary="The product backup lane; moves with its service when hosts "
                "split.",
        death="Kept by design; destination moves to R2 (07, start-fresh).",
        evidence=["infra/nixos/modules/finite-litestream.nix:281-312"],
        campaigns=["r2-blobs"],
    ),
    dict(
        id="borg-rsyncnet",
        name="borg host backups → rsync.net",
        kind="data",
        parties=["edge", "dashboard"],
        status="kept",
        owner="team",
        via="Per-host repositories; lat2 got a dedicated repo at cutover",
        summary="Host-level backup lane for the app plane.",
        death="Kept; per-service decomposition of the recovery fence is part "
              "of the isolation wave.",
        evidence=["infra/nixos/hosts/finite-lat-2/default.nix (bottom)"],
        campaigns=["service-isolation"],
    ),

    # ---- deploy / ops couplings ----------------------------------------------
    dict(
        id="edge-hostnames",
        name="Caddy edge + 11 hostnames / 4 zones",
        kind="deploy",
        parties=["edge", "cloudflare"],
        status="current",
        owner="team",
        via="lat2 Caddy terminates finite.computer, brain/chat.finite.computer, "
            "identity.finite.vip, api.finite.chat, *.finite.chat, *.docs.finite.chat",
        summary="Verbatim proxy of each service's public listener (edge never "
                "filters). After the VPS split, per-host Caddys would "
                "multiply — 08 replaces them with Cloudflare tunnels.",
        death="08: all-CF via tunnels; SSE compatible as-shipped. The Caddy "
              "modules dissolve with the split.",
        evidence=["infra/nixos/modules/caddy.nix:2-83",
                  "essentials-audit 08"],
        campaigns=["cf-edge", "service-isolation"],
    ),
    dict(
        id="wireguard-mesh",
        name="WireGuard mesh (10.254.x, hub on lat2)",
        kind="deploy",
        parties=["runner", "core", "identity", "lat2", "lat3", "lat4"],
        status="current",
        owner="team",
        via="lat2 hub; lat3/lat4 spokes; /29 widening in flight for lat4",
        summary="Carries runner→core leases and runner→identity name "
                "registration today.",
        death="directory-claim-token removes the identity reader; the "
              "runner→core lane remains until launch auth is re-examined.",
        evidence=["infra/nixos/hosts/finite-lat-2/default.nix:210-235"],
        campaigns=["directory-claim-token"],
    ),
    dict(
        id="sops-env-distribution",
        name="operator env-file secret distribution",
        kind="deploy",
        parties=["dashboard", "hosted-device", "sites", "brain", "runner"],
        status="current",
        owner="team",
        via="sops-rendered /etc/finite* env files per host; secret-bootstrap-contract.json",
        summary="Two shared tokens span hosts after the split: the "
                "hosted-device vouch token (dashboard+hosted-device) and the "
                "sites viewer token (dashboard+sites). Everything else is "
                "per-service.",
        death="Shrinks with capability-token consolidation (05c); the "
              "bootstrap contract keeps the file inventory honest.",
        evidence=["docs/runs/lat1-lat3-sops-nix-inventory-baseline.md",
                  "infra/nixos/hosts/finite-lat-2/secret-bootstrap-contract.json"],
        campaigns=["capability-tokens"],
    ),
    dict(
        id="cd-conductor-lat1",
        name="CD conductor hard-wired to dead lat1",
        kind="deploy",
        parties=["ci", "lat1", "lat2"],
        status="dying",
        owner="alex",
        via="production-cd workflow targets the retired host; mutation_enabled = true",
        summary="Audit 10's flagged hazard: the protected CD path still "
                "points at the thermally dead chassis while production runs "
                "on lat2. Also the lat1↔lat2 topology tables drifted.",
        death="Must die before the isolation wave can deploy anything — the "
              "conductor has to target per-host closures (12) anyway.",
        evidence=["essentials-audit 10; infra/deployments/production.toml"],
        campaigns=["per-service-closures", "service-isolation"],
    ),
    dict(
        id="cargo-lock-vendor",
        name="whole-workspace Cargo.lock vendor derivation",
        kind="deploy",
        parties=["ci", "chat-server", "brain", "sites", "core", "identity"],
        status="current",
        owner="team",
        via="One root workspace, one root Cargo.lock, one vendored build",
        summary="Doctrine (one workspace) but the build unit is still the "
                "whole host: deploys restart everything (12's finding).",
        death="12: make the service the unit of build/restart/snapshot/"
              "deploy — five-step rollout, keeps the single Cargo.lock.",
        evidence=["essentials-audit 12; AGENTS.md workspace rule"],
        campaigns=["per-service-closures"],
    ),
    dict(
        id="monitoring-no-alerts",
        name="monitoring VPS has never alerted",
        kind="deploy",
        parties=["monitoring"],
        status="dying",
        owner="alex",
        via="Metrics scrape only; zero alert rules reach a human",
        summary="Audit 09: the observability box observes in silence. Fleet "
                "fits Grafana Cloud free tier.",
        death="09 (for Alex): repoint 2 remote-write URLs, port 15 alert "
              "rules, fix the finite-status storage-unit hardcode.",
        evidence=["essentials-audit 09; finite_status.py:1220-1223"],
        campaigns=["grafana-cloud"],
    ),
    dict(
        id="kata-runner-contract",
        name="kata-runner-host shared module contract",
        kind="deploy",
        parties=["runner", "lat3", "lat4"],
        status="kept",
        owner="team",
        via="modules/kata-runner-host.nix + just runner-host-contract CI pin",
        summary="One declaration of the Runner role across hosts; drift "
                "outside the declared per-host set fails CI. Runner-role "
                "changes go in the shared module.",
        death="Kept by design — this is what keeps the fleet from drifting.",
        evidence=["infra/nixos/README.md; scripts/check_runner_host_contract.py"],
        campaigns=[],
    ),
    dict(
        id="non-prod-launchers",
        name="phala + enclavia + docker launchers",
        kind="deploy",
        parties=["runner"],
        status="dying",
        owner="paul",
        via="FC_RUNNER_CLASS dispatch: kata (prod) + apple (local dev) remain",
        summary="−8,112 LOC: Phala canary, the undeployed Enclavia "
                "evaluation lane, and the unused Docker fallback. FC_RUNNER_"
                "CLASS becomes required — no silent default.",
        death="PR #791 (branch cleanup/delete-non-prod-launchers) — review "
              "advanced past the Phala CVM check; Phala half of the runtime-"
              "image contract already deleted on the branch.",
        evidence=["essentials-audit 02"],
        campaigns=["launcher-deletion"],
    ),
    dict(
        id="clawland-legacy-fleet",
        name="clawland legacy fleet + AEON worker",
        kind="deploy",
        parties=["legacy", "clawland"],
        status="dying",
        owner="austin",
        via="finite.vip NIP-05 proxy, ~50 agent namespaces (AEON worker REMOVED via #782)",
        summary="AEON teardown landed 2026-09-01 (#782: worker crate/image, "
                "agentd writer, runner shim). What remains on clawland is the "
                "legacy finite.vip fleet and the NIP-05 route proxy.",
        death="03 (narrowed): decommission the finite.vip fleet path; the "
              "NIP-05 route's successor already runs on lat2's edge.",
        evidence=["essentials-audit 03; infra/hosts/clawland/README.md"],
        campaigns=["aeon-clawland-decommission"],
    ),
    dict(
        id="legacy-whiteglove",
        name="legacy whiteglove product (finitecomputer)",
        kind="deploy",
        parties=["legacy"],
        status="current",
        owner="team",
        via="box1/TRF/smoke users unmigrated; dashboard relay loop",
        summary="The v1 product keeps running for unmigrated users; its "
                "migration bridge is the only reason it's alive.",
        death="Blocked on user migration; then the bridge, broad finitec/"
              "finited surface, and the smoke host all go.",
        evidence=["docs/architecture-overview.md ownership boundaries"],
        campaigns=[],
    ),

    # ---- duplication watchlist -------------------------------------------------
    dict(
        id="nip98-impls-4",
        name="NIP-98 implemented four times",
        kind="duplication",
        parties=["brain", "sites", "chat-server", "runner"],
        status="dying",
        owner="paul",
        via="Four in-tree copies of signed-request verification",
        summary="Same protocol, multiple implementations. Chat consolidated "
                "onto finite-nostr (merged); the sites branch is reviewed and "
                "unmerged; brain-core + dashboard TS copies remain.",
        death="Branch cleanup/single-nip98 @ 2a473720 (worktree "
              "finite-mono-worktrees/proposals/05a-single-nip98, session "
              "sess_4fa05740) consolidates sites; brain + dashboard follow.",
        evidence=["essentials-audit 05a"],
        campaigns=["single-nip98"],
    ),
    dict(
        id="grant-mechanisms-11",
        name="eleven grant/mint/redeem mechanisms",
        kind="duplication",
        parties=["brain", "sites", "chat-server", "identity", "dashboard"],
        status="proposed",
        owner="paul",
        via="fbit- tokens, viewer sessions, email links, vouch tokens, …",
        summary="Post auth-kernel there is no authority knot, but each "
                "product still rolls its own capability-ish machinery.",
        death="05c: adopt fbit- verbatim as Finite Capability Token v1; "
              "mounted variants deleted with conversion. ~5–6k LOC out.",
        evidence=["essentials-audit 05c"],
        campaigns=["capability-tokens"],
    ),
    dict(
        id="chat-stores-2",
        name="two chat stores (the 'legacy' one is authoritative)",
        kind="duplication",
        parties=["chat-server"],
        status="proposed",
        owner="paul",
        via="legacy_store.rs (live) vs the unwired normalized rewrite",
        summary="The clean rewrite sits dead next to the real store. The "
                "risk is finishing neither; the decision is finish-the-swap.",
        death="13: finish the swap (not delete-the-legacy-file), with "
              "schema/data droppables proven first.",
        evidence=["essentials-audit 13"],
        campaigns=["store-consolidation"],
    ),
    dict(
        id="dashboard-fetch-24",
        name="24 hand-rolled dashboard fetch mechanisms",
        kind="duplication",
        parties=["dashboard"],
        status="proposed",
        owner="team",
        via="Bespoke SSE/fetch caching per surface",
        summary="Every dashboard surface re-implements data fetching and "
                "cache invalidation slightly differently.",
        death="05b: SSE → TanStack Query Cache, five phases, ~800–1,300 LOC "
              "out. Spike renders clean.",
        evidence=["essentials-audit 05b"],
        campaigns=["tanstack-query"],
    ),
]

# ---------------------------------------------------------------------------
# Campaigns — the trimming efforts. A campaign exists to close contracts.
# ---------------------------------------------------------------------------

CAMPAIGNS = {
    "auth-kernel": dict(
        name="auth-kernel",
        status="done",
        summary="Products answer auth against their own tables; grants name "
                "keys or tokens; identity shrinks to the NIP-05 directory.",
        source="#784 (merged 2026-08-31), stack #652→#657; explainer 2026-08-25",
    ),
    "search-retirement": dict(
        name="finite-search retirement",
        status="done",
        summary="Zero live dependencies; config-first atomic deletion.",
        source="#774 (merged)",
    ),
    "stale-prune": dict(
        name="stale docs & scripts prune",
        status="done",
        summary="−17.8k lines of confusing docs/scripts that coupled readers "
                "to dead architecture.",
        source="#792 (merged 2026-08-31)",
    ),
    "launcher-deletion": dict(
        name="non-prod launcher deletion",
        status="dying",
        summary="Phala/Enclavia/Docker launchers out; Kata + Apple remain.",
        source="audit 02; branch cleanup/delete-non-prod-launchers @ 3440f022",
    ),
    "single-nip98": dict(
        name="single NIP-98 implementation",
        status="dying",
        summary="Four copies → finite-nostr.",
        source="audit 05a; branch cleanup/single-nip98 @ 2a473720",
    ),
    "service-isolation": dict(
        name="one VPS per product lane",
        status="dying",
        summary="Break the lat2 god-box. Plan of record (Paul, 2026-08-31): "
                "sites first (Alex), then brain, then chat+hosted-device "
                "last. NOTE: audit 11 recommended brain first (Litestream "
                "restore is the drilled path) and sites only after blobs "
                "move to R2 — reconcile the order before cutting.",
        source="audit 11; ADR 0007 lineage",
    ),
    "r2-blobs": dict(
        name="R2 as the single object substrate",
        status="proposed",
        summary="Chat + sites blobs to R2 (start fresh, no history copy); "
                "restores retention; zeroes Class-A cost with 1s→10s sync.",
        source="audit 07",
    ),
    "cf-edge": dict(
        name="all-Cloudflare edge via tunnels",
        status="proposed",
        summary="11 hostnames/4 zones → tunnels; per-host Caddys never "
                "multiply; SSE compatible as-shipped.",
        source="audit 08",
    ),
    "capability-tokens": dict(
        name="Finite Capability Token v1",
        status="proposed",
        summary="Eleven grant mechanisms → fbit- verbatim; shared tokens "
                "spanning hosts get one primitive.",
        source="audit 05c",
    ),
    "tanstack-query": dict(
        name="dashboard data layer",
        status="proposed",
        summary="24 hand-rolled fetch mechanisms → SSE→QueryCache.",
        source="audit 05b; spike/tanstack-query @ f35fa1a2",
    ),
    "store-consolidation": dict(
        name="finish the chat store swap",
        status="proposed",
        summary="legacy_store.rs is authoritative; the normalized rewrite "
                "is unwired. Finish the swap, don't delete the legacy file.",
        source="audit 13",
    ),
    "core-planetscale": dict(
        name="core → PlanetScale Postgres",
        status="proposed",
        summary="Dump-restore + one-secret swap; dialect risk LOW.",
        source="audit 15",
    ),
    "per-service-closures": dict(
        name="per-service build/deploy units",
        status="proposed",
        summary="Service becomes the unit of build/restart/snapshot/deploy; "
                "fixes the CD-conductor-to-lat1 hazard on the way.",
        source="audit 12",
    ),
    "backup-truth": dict(
        name="backup truth matrix",
        status="proposed",
        summary="≤24h everything, ≤10s chat+brain; runners' /data gets its "
                "first backup ever.",
        source="audit 14",
    ),
    "grafana-cloud": dict(
        name="observability that alerts",
        status="proposed",
        summary="Monitoring VPS → Grafana Cloud free tier; 15 alert rules "
                "ported; finite-status hardcode fixed. For Alex.",
        source="audit 09",
    ),
    "aeon-clawland-decommission": dict(
        name="AEON + clawland decommission",
        status="dying",
        summary="AEON worker teardown landed (#782). Remaining: the "
                "finite.vip fleet + NIP-05 route on clawland.",
        source="audit 03; #782 (worker teardown merged 2026-09-01)",
    ),
    "directory-claim-token": dict(
        name="runtime claims its own name",
        status="proposed",
        summary="Claim-token mint at the directory; runtimes claim names "
                "with their own keys; launch stops waiting on identity; the "
                "WireGuard identity reader and lat3 operator token go.",
        source="auth-kernel explainer follow-up (runner-identity-names)",
    ),
    "sleep-wake": dict(
        name="agent sleep / cheap compute",
        status="proposed",
        summary="Suspend-first two-tier (≈300–500ms resume); cron authority "
                "in Core ('Chronos'). Independent convergence with Hermes "
                "Cloud's Fly primitive.",
        source="audit 06",
    ),
    "chat-custody-epochs": dict(
        name="chat custody restore epochs",
        status="dying",
        summary="Landed 2026-09-02 (#810): the currency gate refuses device "
                "state behind the server (snapshot v10 high-water marks) and "
                "the silent same-device-id re-mint is deleted. Remaining: "
                "epoch-continuity probes + key-joins-restore-set bookkeeping.",
        source="audit 16; #810 merged",
    ),
}
