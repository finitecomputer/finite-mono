# Single-project experiment results

Date: 2026-09-15. Synthetic data only; no production cutover.

## Higher-fidelity follow-up

The next slice is implemented; see [HIGH-FIDELITY.md](HIGH-FIDELITY.md). It proves
real `fsite` signature interoperability, native sharing/revocation, managed Git
push publication, and an empty logical target restore of content, permissions,
and editable source. The older observations below describe the first slice;
its source-hosting and native-auth omissions are superseded only to the extent
explicitly proven by the follow-up. Live wildcard DNS/TLS and sibling browser
isolation now pass on `*.sites-poc.lwn.lol`; see `evidence/wildcard-live.json`.
Production account login, provider-loss recovery, complete Git/CLI compatibility,
and certificate renewal remain open.

## Answer

**Yes: one Vercel project can serve multiple independently published private
Finite Sites, including the shared control/auth API.** Alpha and Beta are
hostname mappings and stored versions, not Vercel projects. Each content
publication and rollback leaves the shared Vercel deployment unchanged.

This supersedes the per-project experiment at commit `b0a20308`. Its code and
original evidence remain in Git history. The former control project was renamed
`finite-sites-poc`; the two obsolete per-site scratch projects were removed.

## Live shape

- Project: `finite-sites-poc` (`prj_11v04LOfFRaXQgqqeE8RUZmhUKjE`).
- Control: https://finite-sites-poc.vercel.app.
- Alpha: https://finite-sites-poc-one-alpha.vercel.app.
- Beta: https://finite-sites-poc-one-beta.vercel.app.
- Private content: Vercel Blob `finite-sites-poc-content` in `iad1`.
- Authority: existing scratch Neon `finite-sites-poc-db`, new `poc2_*` tables.
- Local synthetic issuer/operator console: http://127.0.0.1:4319.

The exact deployment is recorded in `evidence/single-project-publication.json`.
Both Sites finish private with the synthetic viewer revoked. The console can
issue fresh grants and sign-in handoffs. Alpha finishes at v2, Beta at v1.

## Evidence

| Proof | Result / artifact |
| --- | --- |
| Independent publication | Four content publications; identical platform deployment before/after; Alpha publication leaves Beta's pointer unchanged. `single-project-publication.json` |
| Hosted request boundary | 49 checks: anonymous HTML/assets/helpers, grant replay, issuer/admin separation, concurrent proof redemption, cookie isolation, forged headers, failed-upload nonactivation, stale activation rejection, rollback, disable, and revocation on GET/HEAD/conditional/range. `single-project-live.json` |
| Actual DB/Blob behavior | 8 checks using scratch Neon/Blob and a fresh local handler: unknown hosts, nonexistent wildcard tenant, direct private-Blob denial, max-size upload and interrupted-upload replay, expired proof/session, persisted state, database outage. `single-project-storage.json` |
| Real browser | Two sign-in flows, private CSS, separate browser storage, cross-origin response unreadability, host-only cookies, rollback/restore, and independent revocation. `single-project-browser.json` and screenshots |
| Generated deployment host | Finite returns 404 after `vercel curl` bypasses outer Vercel protection. `single-project-generated-host.json` |
| Input validation | 7 Node tests: wildcard labels/exact aliases, immutable manifest identity, unsafe paths, and Git deploy-tree rejection. |
| Repository gate | `cargo clippy --all-targets -- -D warnings` passed. No Rust code changed. |

Artifacts above are in `evidence/`. `scripts/finite-status --json` was run before
and after the rollout; this local machine reports `unknown` because production
host evidence is unavailable. This is not production health validation.

## What this means

There is no per-site machine, volume, container, Vercel project, injected gate
credential, or content deployment. Finite retains a small relational catalog:
Sites, grants, sessions, immutable versions/file manifests, and Active Version
pointers. The shared handler checks current grants before reading private Blob.

A successful Blob write is read back and hashed before its file row becomes
uploaded. Replays verify existing immutable bytes, including a simulated crash
between the Blob write and the database acknowledgement. Completing a version
requires every file. Activation uses a compare-and-swap pointer; permissions are
never part of content rollback. Private responses disable browser/CDN caching;
Blob's internal object cache remains behind the authorization boundary.

## Deliberately unproven

- **Actual wildcard DNS/TLS.** Hostname routing is implemented and tested, but
  live URLs use two exact Vercel-provided hostnames on the same project. No
  `finite.site` or other production DNS changed. Domain delegation/certificate
  renewal is still a separate live validation. Configuring a suffix does not
  provision DNS by itself.
- **Production identity and fsite/Git integration.** Synthetic issuer, email
  grants, and opaque sessions model the boundary; they are not WorkOS/native
  principal or existing-cookie compatibility. Source Git remains local.
- **Recovery.** No complete empty-target recovery of sources, grants, sessions,
  manifests, and content has been qualified. Managed storage is not the entire
  Recovery Set or a backup proof. No customer data is in these resources.
- **Full serving contract.** Private-only; no public/private transitions, SPA
  fallback, generated llms.txt, preview-per-version URLs, range streaming,
  conditional 304 responses, iframe behavior, or broad MIME coverage. Unknown
  file types download as binary. Range gets the full file after authorization.
- **Operations.** No cleanup of pending versions/orphan objects/expired sessions,
  account-level publish permissions, quota enforcement beyond the bounded
  fixture limits, audit log, load test, or cost/latency qualification. The
  publisher is an operator tool, not a tenant API. One platform deployment
  affects every Site's serving code.
- **Production browser isolation.** Live aliases are under Vercel's suffix. A
  custom shared suffix needs its own cookie/CSRF/PSL validation before arbitrary
  customer content is hosted there.

Retire the experiment or replace these shortcuts with approved Sites contracts
before any customer migration; see the technical debt ledger.
