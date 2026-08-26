# Deploy the commercial register

This runbook deploys the internal Twenty service to the same dedicated VM as
Grafana. It does not seed customer data. The synthetic NED fixture is a test
contract and must never be applied to production.

## Preconditions

- The reviewed change is merged to `origin/main`, and deployment runs from a
  clean checkout at that exact revision.
- `scripts/finite-status --json`, staged on `finite-monitoring-1-81926`, is
  green. On a first deploy it checks the five existing monitoring services,
  their loopback endpoints, disk, and memory; after installation it also checks
  the CRM units, four exact image digests, backups, and restore health.
- `/etc/finite/commercial-register/borg.env` and its private SSH key have been
  provisioned without printing values. The repository is dedicated to the
  commercial register and reachable with append-only credentials where the
  provider supports them.
- The Borg passphrase, SSH credential, and Twenty `ENCRYPTION_KEY` have an
  independently custodied recovery copy.
- The Namecheap `finite.computer` zone can receive an `A` record for `crm`.
  The target is the monitoring VM, `152.236.5.27`.

## Deploy the private origin

The first deploy may generate the Twenty-only secrets directly on the VM:

```console
infra/commercial-register/ubuntu/deploy --activate --bootstrap-secrets
```

Later upgrades omit `--bootstrap-secrets`:

```console
infra/commercial-register/ubuntu/deploy --activate
```

The command is the mutation boundary. It records canonical status before and
after, pulls only the checked-in image digests, starts Twenty on loopback,
creates and ships a service-consistent backup, restores that Borg archive into
an empty disposable PostgreSQL target, and enables backup/health timers. An
upgrade takes another backup before replacing the compose definition.

If pre-status, backup, restore, or post-status is not green, stop. Do not make
the service public and do not add commercial facts.

## Claim the one workspace before public DNS

Do not expose an unclaimed single-workspace instance: its first user becomes
the administrator. Keep the origin loopback-only and tunnel it to the operator
machine first:

```console
ssh -N -L 3020:127.0.0.1:3020 ubuntu@152.236.5.27
```

Open `http://localhost:3020` and create the one private Finite workspace. The
first-boot `SERVER_URL` deliberately matches that tunnel. Confirm the first user
has full admin access and that a second workspace cannot be created.

Close the tunnel, then switch the durable origin to its public URL:

```console
ssh -T ubuntu@152.236.5.27 \
  sudo /opt/finite-commercial-register/bin/publish-url --activate
```

That command restarts Twenty, creates a new off-host Recovery Set containing
the claimed workspace and public URL, proves it on an empty target, and retains
canonical pre/post status under
`/var/lib/finite-commercial-register/publications/`. Do not publish DNS if it
fails.

## Publish `crm.finite.computer`

1. Create the Namecheap DNS record `A crm 152.236.5.27`. Keep the initial TTL
   low enough to correct a mistake quickly.
2. Wait until `dig +short A crm.finite.computer` returns `152.236.5.27` from a
   public resolver.
3. From the same clean `main` revision, deploy the repo-owned monitoring edge:

   ```console
   infra/monitoring/ubuntu/deploy --replace-compose ubuntu@152.236.5.27
   ```

The edge proxies Twenty's complete public listener to `127.0.0.1:3020`; it has
no route allowlist. Caddy obtains the public certificate only after DNS points
at the host. Prometheus then probes `https://crm.finite.computer/healthz`.

## Install the versioned application

1. Sign in at `https://crm.finite.computer` with the already-claimed admin and
   create one narrowly scoped API key for deploying the versioned application.
   Keep the value outside git and shell history.
2. Configure the Twenty CLI remote, preview the application plan, then apply it:

   ```console
   cd commercial-register
   corepack yarn twenty remote:add --as finite-production --url https://crm.finite.computer --api-key "$FINITE_COMMERCIAL_TWENTY_API_KEY"
   corepack yarn twenty --remote finite-production plan
   corepack yarn twenty --remote finite-production apply
   ```

3. Create a separate runtime API key for the commercial-update agent with only
   the object permissions it needs. Store it where the invoking Brain agent can
   read it, never in this repo.
4. Begin with a real, sourced NED record. Do not import
   `commercial-register/tests/fixtures/ned-update.json`; its people and amounts
   are deliberately synthetic.

## Verify

```console
curl --fail https://crm.finite.computer/healthz
ssh -T ubuntu@152.236.5.27 \
  sudo cat /var/lib/finite-commercial-register/deployments/REVIEWED_SHA/finite-status-after.json
```

The retained evidence is also under
`/var/lib/finite-commercial-register/deployments/REVIEWED_SHA/`. Verify that
the public certificate is valid, the application views exist, the production
agent can read an organization, and no synthetic customer records exist.

## Rollback

The boundary is the pre-upgrade Borg archive plus the previous checked-in
compose/image digest. Twenty runs database migrations at startup, so changing
only the image backward is not a safe rollback.

1. Remove `crm.finite.computer` from the Caddy config or temporarily point DNS
   away if the public origin must be fenced.
2. Stop `finite-commercial-register.service`.
3. Restore the pre-upgrade Borg archive to an empty target and verify its
   manifest before touching the live directories.
4. Move the failed `/var/lib/finite-commercial-register` tree aside; do not
   overwrite it in place. Restore PostgreSQL, local storage, `twenty.env`, and
   `postgres.env` together from the same Recovery Set.
5. Install the previous reviewed compose definition and start the service.
6. Run `scripts/finite-status --json` and the public health check again before
   re-exposing DNS.

Do not invent a partial database repair or reuse a newer database with an older
Twenty image.
