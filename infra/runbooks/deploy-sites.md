# Sites v2 validation and finite.site cutover

## Status and authority

As of 2026-09-08, this is a local preparation plan, not a completed rollout.
There is no provisioned v2 host or recorded live migration/auth proof. Production
mutation needs separate explicit authorization. Run `scripts/finite-status`
before and after each rollout; add missing fleet probes there.

[ADR 0028](../../finite-sites/docs/adr/0028-static-only-sites-platform-service.md)
defines the static-only independent service and prohibits a general migration
framework, dual writes, and runtime compatibility flags. The destination now
requested is `finite.site`, replacing that ADR's proposed `v2.finite.chat`
validation hostname. Existing checked-in infrastructure may still name the
validation hostname; reconcile the reviewed NixOS configuration, TLS, generated
URLs, and client validation before provisioning. This document does not change
DNS or that configuration.

Canonical production stays on finite-lat-2 in `legacy-canonical` mode, pinned to
`fsite/v0.5.3`. Keep `api.finite.chat`, `*.finite.chat`, and
`*.docs.finite.chat` serving their current contract until an explicit cutover.
Do not advance `fsite-latest` or remove the legacy dashboard/native viewing
clients merely because the new server code merges.

## Target boundary

- Dedicated NixOS host: `nixosConfigurations.finite-sites-v2`.
- Intended control and Git Remote origin: `https://finite.site`.
- Intended served Site URLs: `https://{site}.finite.site/`.
- Listener: `127.0.0.1:8787`; state: `/var/lib/finite-sites`.
- Health: `/api/v2/healthz`; unit: `finite-saas-sites.service`.
- Deploy a reviewed CI-built NixOS closure; nothing builds on the host.

The existing dashboard Account Auth session is the browser identity authority.
The separate Auth Gate accepts an authenticated server-to-server assertion of
that verified email and signs a short-lived, origin-bound vouch. It has no
second WorkOS login or independent browser session. Sites verifies the vouch
against a pinned public key, mints its own cookie, and checks its own current
permissions on each request. The gate owns no shares, publishing grants, or
account-to-Sites permission mirror. This revises PR 803's original independent
WorkOS/session gate; it does not move authorization into the dashboard.

A non-Finite recipient must still be able to authenticate an allowlisted email
without buying service or provisioning an Agent. Prove that journey against the
configured Account Auth flow before claiming guest sharing works. Bare npub
shares are not evidence of an email grant and must not be converted by guesswork.

## Recovery and preservation contract

Before writing an importer, inventory the actual source read-only and run the
Static Launch Check. Use `scripts/snapshot-sqlite` or a scratch copy for snapshot
inspection. Ambiguous ownership, name collisions, unsupported kinds, or multiple
Sites per Project stop the affected migration; never select a winner by row order.

The Sites Recovery Set and one-off copy must preserve:

- Site IDs, names/reservations, status, visibility, and active Version pointers.
- `shares(site_id, email)` and `sites.publisher_email_principal_id` with its
  `sites_email_principals` rows. These are the existing email viewing authority;
  do not rederive them from current dashboard accounts or native keys.
- Project IDs, repositories and refs, publication events, Version manifests,
  referenced blobs and their hashes, and audit history.
- Publishing/collaboration grants, native shares, Authorized Sites Keys, and
  their revocation state, even though browser v1 of the gate proves only email.
- Relevant service secrets and configuration as part of the recovery procedure;
  document names/locations only, never values in git or migration reports.

Include any Sites created during validation in the inventory. Prefer an empty
target plus one service-consistent restore; if two populated registries need
reconciliation, review a bounded one-off mapping with explicit conflict failures.
Do not introduce startup repair, ongoing synchronization, or a new permission
store. Cookie migration is unnecessary across domains: reauthenticate and check
the same preserved grants. Old authentication tokens must not be forwarded to
the new domain.

Prove on synthetic state written by the old released service, then restored into
an empty candidate target: identical committed bytes and active Versions;
allowlisted publisher and recipient can view; unlisted recipient cannot; revoked
share denies an existing session; disabled/private state stays protected; restart
preserves the result. Add an existing validation-Site fixture if such data exists.
Record source/candidate revisions, inventory and blob checks, failures, recovery
artifact location, and restore result. Candidate-only tests are insufficient.

## Staged rollout gates

1. **Host.** Build the exact reviewed closure and provision the dedicated host.
   Configure the daemon, gate trust, service secrets, backup timers, and off-host
   backup destination. Restore the Recovery Set onto an empty scratch target and
   record the result before it carries production data. Existing modules are
   `finitesitesd.nix`, `caddy-sites-v2.nix`, and
   `finite-sites-v2-backups.nix` under `infra/nixos/modules/`.

2. **Domain.** Point `finite.site` and `*.finite.site` at the new service/edge
   with covering TLS certificates. Verify health, API/Git origins, and Site URL
   generation using a disposable operator Project. This is not a redirect or
   cutover of old hostnames. All mutable HTML and asset responses retain
   `Cache-Control: no-store`.

3. **Data.** After the offline proof, schedule a bounded Publishing Write Freeze
   for all source writers: API mutations, Git pushes, publication reconciliation,
   sharing changes, and background state writers needed for a consistent copy.
   Already-published Sites continue serving. Take and name the final consistent
   backup; perform the reviewed one-off migration; verify the full preservation
   inventory and every migrated email allowlist before making new copies public.
   Keep the old deployment and Recovery Set intact.

4. **Authentication and viewing.** Before advertising migrated URLs, exercise a
   real browser with an existing finite.computer login and no gate/Sites cookies:
   publish, open the returned Site, and see content with no new challenge. Repeat
   for an allowlisted Finite recipient, a guest, and a denied recipient. Exercise
   the dashboard preview separately: an iframe redirect is not top-level login,
   and third-party cookie restrictions must not be assumed away. Test account
   switch/logout, new-domain reauthentication, replay, wrong-origin vouches,
   assets, and share revocation. Gate configured does not mean this proof passed.

5. **Clients and old links.** Only after data and auth evidence passes, switch
   the dashboard/native viewing path and publish the CLI targeting the v2
   contract. Selected validation builds use `FINITE_SITES_API=https://finite.site`
   beforehand; returned Git Remotes are authoritative. Keep release asset names
   unchanged. If retaining old Site links, use an explicit known-name mapping to
   new canonical Site URLs, preserving ordinary paths/queries and stripping auth
   credentials. Unknown/colliding mappings fail closed. A Site redirect does not
   migrate an old Git Remote or adapt v1 API requests; publish their transition
   instructions explicitly. Remove superseded client auth machinery once its
   production consumers have switched, as tracked in the
   [debt ledger](../../finite-sites/docs/technical-debt-ledger.md#11-legacy-viewing-clients-retained-until-sites-cutover).

## Rollback boundary

Before new production writes are accepted, rollback can restore old routing and
clients to the untouched legacy deployment. Keep the write freeze while resolving
any final-copy failure; do not purge either Recovery Set.

Once the target accepts writes, routing back to the stale source is data loss.
Freeze writes and preserve the new Recovery Set before deciding on a reviewed
reverse reconciliation or fixing forward. A previous NixOS generation alone is
not a data rollback. Record this point explicitly in the cutover log. No step in
this runbook authorizes deleting `/var/lib/finite-sites`.
