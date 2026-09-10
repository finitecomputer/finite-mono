# Finite Business production service

Finite Business is Finite's private internal business hub, built on Twenty. Its
first module is the commercial relationship register. It runs beside Grafana on
the dedicated monitoring VM (`152.236.5.27`), isolated from the product
database, and binds only to `127.0.0.1:3020`; the monitoring VM's one Caddy edge
publishes it at `https://business.finite.computer`.

The active Ubuntu/systemd definition is [`ubuntu/`](ubuntu/). It uses the
official Twenty, PostgreSQL, and Redis images by immutable digest. Nothing is
built on the host. The Twenty server and worker share local file storage, while
PostgreSQL and Redis each have separate persistent bind mounts under
`/var/lib/finite-commercial-register`.

Twenty is deployed unmodified from its official image. The upstream project is
mostly AGPL-3.0 with separately marked enterprise files and MIT-licensed SDK/app
packages; see the [upstream license](https://github.com/twentyhq/twenty/blob/main/LICENSE).
Finite's application uses the published application interfaces and MIT-licensed
SDK. Do not add an enterprise-marked Twenty file or patch the server image
without a fresh license and source-offer review.

## Secrets

No values belong in this public repository. The host owns:

| File | Required names | Purpose |
|---|---|---|
| `/etc/finite/commercial-register/twenty.env` | `PG_DATABASE_URL`, `SERVER_URL`, `REDIS_URL`, `STORAGE_TYPE`, `ENCRYPTION_KEY`, `APP_SECRET`, `IS_MULTIWORKSPACE_ENABLED`, `IS_CONFIG_VARIABLES_IN_DB_ENABLED` | Twenty infrastructure and at-rest encryption |
| `/etc/finite/commercial-register/postgres.env` | `POSTGRES_DB`, `POSTGRES_USER`, `POSTGRES_PASSWORD` | Database bootstrap; not exposed to the Twenty server or worker |
| `/etc/finite/commercial-register/borg.env` | `BORG_REPO`, `BORG_PASSPHRASE`, `BORG_RSH` | Dedicated off-host encrypted backup repository |
| Path named inside `BORG_RSH` | private SSH key | Append-only backup transport credential |

All files are root-owned mode `0600`. `--bootstrap-secrets` creates only a new
Twenty database password and encryption/session keys. It never creates, copies,
or guesses the independently custodied Borg credentials.

The generated first-boot `SERVER_URL` is `http://localhost:3020` so the first
administrator can claim the only workspace through an SSH tunnel without
exposing an unclaimed instance. The reviewed `publish-url --activate` command
changes it to `https://business.finite.computer`, restarts Twenty, and proves a
new off-host Recovery Set before the DNS/Caddy publication boundary.

The encryption key is part of the encrypted Recovery Set because losing it
makes encrypted values in PostgreSQL unreadable. The Borg passphrase and SSH
credential must also exist in independent off-host custody; a backup containing
its only decryption key is not a recovery boundary.

## Recovery

The daily backup briefly stops only the Twenty server and worker, takes a
transaction-consistent PostgreSQL custom dump, archives local file storage and
the secret environment required to decrypt restored rows, seals the files with
SHA-256, restarts the writers, and copies the Recovery Set to Borg. Monitoring
stays online during the fence.

PostgreSQL plus local file storage are authoritative. Redis holds only
rebuildable cache/queue state for this MVP and is deliberately restored empty;
do not enable an ingestion or automation path whose sole durable record is a
Redis job without first expanding the Recovery Set.

`/opt/finite-commercial-register/bin/restore-check` extracts the newest Borg
archive into scratch space and restores the dump into an empty, disposable,
digest-pinned PostgreSQL container. It then starts disposable Redis and Twenty
containers against the recovered database and storage and requires the restored
application health endpoint to pass. A successful first run is required by the
deploy before this service is considered healthy. The check never opens or
rewrites the production database.

Deployment, DNS, schema installation, verification, and rollback are in
[`../runbooks/deploy-commercial-register.md`](../runbooks/deploy-commercial-register.md).
