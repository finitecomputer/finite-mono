# Single-project routing and publishing research

Research date: 2026-09-15. Proposal only; no Vercel or DNS resources changed.

## Recommendation

Use exactly one Vercel project for the Sites Platform Service, including
authentication, publishing APIs, and all served Sites. Route by hostname:

| Proposed hostname | Responsibility in the same deployment |
| --- | --- |
| `alpha.poc.finite.site` | Alpha Site content and viewer-session callback |
| `beta.poc.finite.site` | Beta Site content and viewer-session callback |
| A dedicated control hostname | Authentication, sharing, publishing, operator UI |

These are proposed names, not verified DNS allocations. Use a Validation Site
Base Domain for the experiment; eventual `{name}.finite.site` follows the
existing one-label Site Name model in `finite-sites/CONTEXT.md`.

Vercel explicitly supports many tenant domains in one deployment. A wildcard
delivers requests to that deployment; Finite resolves the Site and authorizes
the request. Site creation becomes a database operation under an existing
wildcard, rather than project or per-site DNS provisioning. This last point is
the proposed application design. [Vercel multi-tenant overview](https://vercel.com/kb/guide/nextjs-multi-tenant-application)

## Why subdomains, rather than `/alpha` and `/beta`

Different paths on one hostname share a browser origin. Consequently,
untrusted JavaScript in Alpha could make same-origin authenticated requests to
Beta and read the response if the viewer has access to both. Separate hostnames
give Sites distinct origins and separate browser storage. Cookie `Path` is
not an isolation boundary. [Mozilla same-origin policy](https://developer.mozilla.org/en-US/docs/Web/Security/Defenses/Same-origin_policy),
[Mozilla cookie scope](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie)

Subdomains still share a browser *site* until the shared suffix is on the Public
Suffix List. Vercel recommends submitting the tenant suffix, keeping control/auth
on a different apex domain, host-only `__Host-` session cookies, and Origin or
CSRF-token validation for mutations. A different control hostname can still
belong to the same Vercel project. PSL publication is a production follow-up,
not something this research performed. [Vercel tenant-domain isolation](https://vercel.com/docs/platforms/multi-tenant-platforms/configuring-domains#protecting-tenant-subdomains-with-the-public-suffix-list)

For the proof, use synthetic identities, no parent-domain session cookie, and
prove cross-Site isolation in a real browser. Central sign-in should return a
short-lived, single-use code bound to the destination Site and callback; that
callback establishes a host-only viewer session. This is a design recommendation,
not a Vercel-provided Finite login integration.

## Wildcard DNS and certificates

- Standard supported setup: configure the wildcard on the project and use
  Vercel nameservers so Vercel can solve DNS challenges and renew TLS
  certificates. A wildcard CNAME by itself does not establish that certificate
  workflow. [Vercel wildcard setup](https://vercel.com/docs/domains/working-with-domains)
- Keep `*.poc.finite.site` confined to the experiment. Whether the actual domain
  can be delegated to Vercel as a child zone requires a disposable setup check;
  current general docs do not clearly document that exact arrangement.
- There is official precedent for retaining apex DNS: Vercel documents
  `_acme-challenge.preview` NS delegation to its nameservers, plus a wildcard
  CNAME and enabling Vercel DNS, for **Preview Deployment Suffix**. That is a
  preview-specific guide, not proof that arbitrary production wildcard
  onboarding accepts the same configuration. The older general wildcard
  workaround URL now redirects to the standard nameserver instructions.
  [Vercel delegated preview certificate setup](https://vercel.com/kb/guide/preview-deployment-suffix-without-vercel-nameservers)
- DNS-01 certificate validation itself supports CNAME or NS delegation, so the
  remaining question is Vercel's provisioning interface and renewal behavior.
  [Let's Encrypt DNS-01](https://letsencrypt.org/docs/challenge-types/#dns-01-challenge)

If wildcard delegation needs vendor clarification, the proof can start with
two individually registered hostnames and CNAME records pointing to the same
project. This retains the single-project and separate-origin design without
moving apex nameservers. Use the exact DNS targets returned for the project.
[Vercel external DNS setup](https://vercel.com/docs/domains/set-up-custom-domain)

## Request boundary

Proposed flow: **validated hostname → Site ID → current Share check → Active
Version and path → content**. Control handlers are exposed only on the exact
control hostname; viewer callbacks have an explicit narrow route on Site hosts.

Normalize and validate the entire hostname against either the configured
one-label suffix or an explicitly verified custom-domain mapping. Reject unknown
hosts, extra labels, reserved names, and client-supplied tenant headers. The
Vercel examples show hostname resolution and stripping inbound tenant headers;
their broad matcher exclusions are examples, not suitable blanket exceptions
for private Site assets. [Vercel proxy and routing](https://vercel.com/docs/platforms/multi-tenant-platforms/middleware-and-routing)

Generated deployment URLs must not gain tenant authority from a path, query
parameter, or arbitrary first hostname label. Reject tenant serving there in the
initial proof. If per-tenant previews are added, explicitly validate the preview
mapping and retain current authorization. Vercel supports tenant-prefixed
preview URLs with a paid Preview Deployment Suffix; that feature handles routing,
not Finite permissions. [Vercel tenant preview URLs](https://vercel.com/changelog/preview-urls-optimized-for-multi-tenant-platforms)

Custom domains later attach to this same project, after ownership verification,
then map to one Site ID. Vercel handles certificate provisioning; Finite keeps
domain assignment authoritative. [Vercel custom domains](https://vercel.com/docs/platforms/multi-tenant-platforms/configuring-domains#offering-custom-domains)

## Storage and independent publication

The current experiment puts each Site's files in its own Vercel deployment.
Under the requested single-project design, deploy only the shared platform code.
Store Site files outside that deployment so publishing Alpha does not rebuild
Beta or change platform code. The following is a proposed design, not an
implemented or measured replacement for the existing experiment.

- **One private Vercel Blob store** holds files at immutable keys such as
  `sites/<site-id>/versions/<version-id>/<path>`.
- **The existing managed Neon database** holds Sites, Shares, viewer sessions,
  version manifests, and each Site's Active Version pointer.
- **One serving handler** validates the hostname and requested path, checks
  current Finite authorization, resolves the active manifest, and streams the
  corresponding private blob. Control/auth handlers run in this same project.

Vercel explicitly documents private Blob delivery through an application route
that authenticates the request and streams `get()` results. Private blob URLs
are not publicly accessible. The initial proof should not redirect viewers to
public objects or bearer signed URLs: those would establish a separate access
path whose expiry/revocation behavior needs its own contract. Use
`Cache-Control: private, no-store` for viewer responses initially and keep the
current grant check on every HTML and asset request. Blob's internal CDN cache
can still cache the immutable bytes behind the handler.
[Vercel private storage](https://vercel.com/docs/vercel-blob/private-storage)

Publishing should upload and verify a complete immutable version, then commit
its manifest and move the Active Version pointer transactionally. A partial
upload must never become active. A rollback selects an earlier complete version
without reverting Shares or sessions. Concurrent publication and ambiguous
network failures require idempotency/reconciliation before production use.
Vercel documents read-after-write consistency for new private blobs; overwrites
can return stale cached bytes for up to 60 seconds unless reads bypass the
cache. Immutable version keys avoid relying on overwrite invalidation.
[Vercel private Blob consistency](https://vercel.com/changelog/vercel-blob-now-supports-consistent-reads-on-private-storage)

This trades Vercel's per-project deployment history for a small Finite version
catalog. Vercel deployment rollback applies to shared platform code; Site
content rollback is a database pointer change. Managed storage removes the need
to operate a volume/blob server, but does not itself prove recovery of Git
sources, permissions, manifests, and content together. Source hosting and a
recoverable copy of authoritative state remain separate decisions. The proof
should also measure handler/database latency and asset-serving cost before
choosing cache optimizations.

Extend the routing proof with independent Alpha publication while Beta stays
unchanged, failed-upload nonactivation, direct Blob access denial, and content
rollback that preserves current revocation. Do not redeploy the platform app
during the content-publication tests.

## Limits and proof criteria

Current general limits list 50 domains per Hobby project and soft limits of
100,000 on Pro and 1,000,000 on Enterprise. Pro has 2,048 configured routes per
deployment: use a shared resolver rather than one generated rewrite per Site.
The current API table lists 100 project-domain mutations per minute per owner.
[Vercel general limits](https://vercel.com/docs/limits)

The multi-tenant limits page contradicts parts of that page: it also says
unlimited custom domains on all plans and quotes lower hourly domain API limits.
Treat account/API-reported limits as a validation item before projecting bulk
custom-domain throughput. Wildcard Site creation should not need those calls.
[Vercel multi-tenant limits](https://vercel.com/docs/platforms/multi-tenant-platforms/limits)

Prove two Sites on one project; authorized and denied HTML/assets; no cross-Site
session or browser-storage access; unknown/generated hostname rejection; forged
tenant-header rejection; revocation on the next request; and content rollback
without restoring old permissions. Confirm wildcard TLS issuance and renewal
configuration separately. This research does not establish any live DNS result.
