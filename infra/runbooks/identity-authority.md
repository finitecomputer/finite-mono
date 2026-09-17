# Finite Identity Authority on finite-lat-2

This runbook covers the production `finite-identity.service`, its public edge,
the shared managed-agent provisioning credential, and recovery of its
identity-owned SQLite state. The Authority stores public keys, bindings, and
audit metadata only. It never stores a user or Agent's secret Nostr key.

## Fixed production boundary

| Fact | Required value |
|---|---|
| Service unit | `finite-identity.service` |
| Binary | Nix-built `finite-identityd` from the deployed mono revision |
| Listener | `127.0.0.1:8790` (full router: public surface + loopback-only operator routes) |
| Public listener | `127.0.0.1:8791` (`public_router` only; the Caddy edge proxies this verbatim) |
| Public/signing origin | `https://identity.finite.vip` |
| Finite VIP Domain | `finite.vip` |
| Data directory | `/var/lib/finite-identity` (`StateDirectory`, mode `0700`) |
| SQLite database | `/var/lib/finite-identity/identity.db` |
| Operator credential | `/etc/finite/identity-operator.env`, `root:root`, mode `0600` |
| Mail provider credential | `RESEND_API_KEY` in `/etc/finite-saas/sites.env` |
| Mail sender | `Finite Identity <identity@finite.chat>` |
| Trusted same-host product URL | `http://127.0.0.1:8790` |

The operator environment contains exactly
`FINITE_IDENTITY_OPERATOR_TOKEN`. Systemd reads it for the Authority and both
trusted Runner processes. It must never enter `FC_RUNNER_RUNTIME_ENV_JSON`,
the Runtime secret environment, an Agent Runtime, a command argument, logs, or
this repository.

## Public and private routes

The Directory's public surface, proxied verbatim by Caddy, is exactly:

- `GET /health`
- `GET /.well-known/nostr.json`
- `POST /api/v1/email-challenges`
- `POST /api/v1/vip-email-bindings/redeem`
- `POST /api/v1/nip05-resolution`

Operator endpoints remain loopback-only, and the directory shrink deleted
Principal Resolution, Mailbox Proofs, Email-Only Principals, WorkOS account
bindings, brain resolution, and the sites-notification relay. A public request
to any of those must return `404`.

## Deployment and credentials

Use [Core deployment](deploy-core.md) for the reviewed app-plane closure.
The credential installer `scripts/install-identity-authority-credentials`
preserves a valid existing token and creates one only when absent. After host
loss, provision the same replacement token to the Authority and trusted Runner
processes before starting them; it is not identity data.

Verify exact DNS and NIP-05 routing before changing the edge. The historical
clawland manifest still names retired lat1; it is not current deployment
authority and must not be applied unchanged. TODO: reconcile retained apex
routing evidence in [FIN-95](https://linear.app/finitecomputer/issue/FIN-95).

## Verification

Inspect names and state, never environment values:

```sh
systemctl is-active finite-identity.service
systemctl is-active finite-saas-core.service
systemctl is-active finite-saas-runner-phala.service
systemctl is-active finite-saas-runner.timer
systemctl show finite-saas-runner.service \
  --property=After --property=Requires --property=EnvironmentFiles
systemctl show finite-saas-runner-phala.service \
  --property=After --property=Requires --property=EnvironmentFiles
curl --fail --silent http://127.0.0.1:8790/health
curl --fail --silent https://identity.finite.vip/health
curl --fail --silent \
  'https://identity.finite.vip/.well-known/nostr.json?name=definitely-unknown'
curl --fail --silent \
  'https://finite.vip/.well-known/nostr.json?name=definitely-unknown'
```

Expected health JSON identifies `finite-identity` with status `ok`. The
two unknown NIP-05 requests must return byte-identical JSON with an empty
`names` map, proving both public origins reach one Authority. The daemon also
serves the service-owned public surface (`public_router`) on a second loopback
listener, which the Caddy edge proxies verbatim:

```sh
curl --fail --silent http://127.0.0.1:8791/health
```

Verify that operator routes are not exposed and that the retired routes stay
gone (every one must return 404):

```sh
test "$(curl --silent --output /dev/null --write-out '%{http_code}' \
  https://identity.finite.vip/api/v1/operator/inspect)" = 404
test "$(curl --silent --output /dev/null --write-out '%{http_code}' \
  -X POST -H 'Content-Type: application/json' -d '{}' \
  https://identity.finite.vip/api/v1/mailbox-proofs/consume)" = 404
test "$(curl --silent --output /dev/null --write-out '%{http_code}' \
  -X POST -H 'Content-Type: application/json' -d '{}' \
  https://identity.finite.vip/api/v1/principal-resolution/satisfies-grant)" = 404
```

The full edge contract — the public surface is exactly the checked-in
manifest, and every manifest route must answer non-404 — is checked by
`just identity-edge-contract` (static) and
`finite-identity/scripts/identity-edge-contract-gate.py` (live probe against production or a
`--target` override).

The final product acceptance is one normal managed-agent creation. The Runner
must fetch the Runtime's public `agent_npub`, bind its Core-assigned managed
email through the loopback operator endpoint, and complete creation. An exact
retry of the same email/key must be idempotent; a different key for that email
must fail closed. Do not create a synthetic production binding merely to test
the service because v1 bindings are intentionally immutable.

## Backup and restore

`finite-identity-backup.timer` takes an online SQLite backup every six hours,
validates it, retains 14 days locally, and includes the backup directory in
the existing daily off-host Borg job. Its latest backup must remain less than
seven hours old and pass its recorded SHA-256 check.

The deploy/manual-triggered coordinated
`finite-hosted-web-chat-snapshot.service` also fences both Runner workers,
Core, Brain, Hosted Device, Chat, and the Authority, then copies `identity.db`
with SQLite's backup API into:

```text
/data/recovery-snapshots/hosted-web-chat/<stamp>/finite-identity/identity.db
```

The database is covered by the snapshot SHA-256 manifest and the existing
off-host Borg job. The snapshot fails closed if the Authority database is
missing. Never back up the live SQLite file by plain copy while the service is
running.

Inspect the snapshot before restoration only through the snapshot helper:

```sh
scripts/snapshot-sqlite integrity-check /path/to/snapshot/finite-identity/identity.db
```

It copies the database and any WAL/SHM sidecars to a private scratch directory;
expected output is exactly `ok`. For an authorized production restore:

1. Stop both Runner workers and their timer, then stop
   `finite-identity.service`.
2. Preserve the current `/var/lib/finite-identity` directory as the named data
   rollback boundary.
3. Restore `identity.db` as the service StateDirectory owner with mode `0600`.
4. Start the Authority and verify loopback health and representative public
   resolution.
5. Start Core, then the Runner workers/timer.

A NixOS generation rollback does not roll back this database. Restoring it
restores public binding state but cannot restore Agent Local Identity Keys.
Agent `/data` recovery remains the authority for those secret keys.

## Rollback

If the new service or edge fails before accepting a binding:

1. Stop both Runner workers so managed creation fails closed.
2. Switch to the previously recorded NixOS system closure.
3. Leave the new empty StateDirectory and operator file in place; neither is
   consulted by the old generation.

If the Authority accepted any binding, preserve its database before a Nix
rollback. Do not replace it with an older snapshot merely to make a launch
retry: immutable email/key conflicts require investigation, not state rewrite.

The apex route has an independent rollback boundary. Delete only the objects in
its checked-in manifest:

```sh
kubectl delete -f infra/hosts/clawland/finite-identity-nip05-route.yaml
```

This restores the previous legacy apex routing and does not touch the Authority
database, `identity.finite.vip`, or any other legacy fleet route.
