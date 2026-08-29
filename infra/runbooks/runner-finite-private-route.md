# Reconcile stale Runner Finite Private overrides

The shared Kata Runner role owns the Finite Private launch route and model in
`infra/nixos/modules/kata-runner-host.nix`. `/etc/finite/runner.env` loads
after that shared file so credentials, drain state, the promoted Runtime
Artifact pin, and bounded incident overrides can stay operator-owned.

Do not copy `FC_RUNNER_FINITE_PRIVATE_BASE_URL` or
`FC_RUNNER_FINITE_PRIVATE_MODEL` into the operator file. A stale copy wins over
the shared role and can send a new Agent launch to a retired endpoint.

## PRECONDITIONS

- The helper's source revision is reviewed and merged.
- `scripts/finite-status --json` has been captured before the change.
- Applying the change has explicit **Production Deploy** authorization.
- No other operator is editing `/etc/finite/runner.env`; sanctioned edits use
  this helper's per-file reconciliation lock.

The recovery boundary is the host-local operator file. The helper recognizes
only this exact pair:

- `https://kimi-k2-6.finite.containers.tinfoil.dev/v1`
- `deepseek-v4-flash-0731`

It is read-only by default. `--apply` removes only those two lines, preserves
the file's other bytes and metadata, and creates the hard-link rollback copy
`/etc/finite/runner.env.pre-glm53-route` before atomically replacing the live
path. Duplicate, partial, custom, symlinked, already-backed-up, or hard-linked
input fails closed. The command never prints values from the file. Synthetic
file tests prove byte and metadata preservation; they are not live-host proof.

## STEPS

### 1. Run the read-only preflight

Run the authoritative platform status command and retain its JSON before the
change. Then check both active Runner hosts:

```bash
scripts/finite-status --json > finite-status-before-runner-route.json

for host in root@207.188.7.157 root@152.236.34.15; do
  ssh -o BatchMode=yes "$host" \
    'bash -s -- --check /etc/finite/runner.env' \
    < scripts/reconcile-runner-finite-private-env
done
```

Expected output is `needs-migration` for the known stale pair or `clean` when
the operator file already defers to the shared Nix role. Any refusal is a stop
condition; inspect the file on-host without printing its secret values.

### 2. Apply the guarded change

**TODO: this two-host Production Deploy has not been exercised.** The first
authorized run must retain each helper result and confirm that both hosts
report `migrated` with their expected rollback path. The file replacement is
atomic, so a Runner invocation sees either the complete old file or the
complete new file.

```bash
for host in root@207.188.7.157 root@152.236.34.15; do
  ssh -o BatchMode=yes "$host" \
    'bash -s -- --apply /etc/finite/runner.env' \
    < scripts/reconcile-runner-finite-private-env
done
```

Expected behavior: the Runner timer reads the file on its next invocation, so
no service restart should be required. The first authorized run must verify
that expectation instead of treating it as prior evidence.

## VERIFY

Re-run `--check` on both hosts and require `clean`. Then run the same
authoritative status command and require the effective Finite Private
route/model to be green:

```bash
scripts/finite-status --json > finite-status-after-runner-route.json
```

**TODO: live verification is not complete** until both active hosts report the
canonical route/model and one fresh-Agent launch reaches chat readiness on GLM
without manual Runtime repair. Record that evidence in the deployment record.

## ROLLBACK

Rollback is host-local and restores the exact pre-change inode contents. Stop
if the rollback copy is missing or is not a regular, non-symlink file.

**TODO: the rollback path is synthetic-tested but has not been exercised on a
live Runner host.** The first authorized rollback must retain before/after
`scripts/finite-status` output and confirm that the target bytes match the
rollback copy without printing secret values.

```bash
ssh -o BatchMode=yes root@HOST 'bash -s' <<'ROLLBACK'
set -euo pipefail
backup=/etc/finite/runner.env.pre-glm53-route
target=/etc/finite/runner.env
[[ -f "$backup" && ! -L "$backup" ]]
rollback_path="$(mktemp /etc/finite/runner.env.rollback.XXXXXX)"
trap 'rm -f "$rollback_path"' EXIT
cp -p "$backup" "$rollback_path"
mv "$rollback_path" "$target"
rollback_path=""
ROLLBACK
```

After rollback, run `scripts/finite-status` again. Keep the rollback copy until
the post-change status evidence and one fresh-Agent launch are accepted; its
contents are secret-bearing host material and must never be committed.
