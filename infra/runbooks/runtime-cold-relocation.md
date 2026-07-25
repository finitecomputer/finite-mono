# Cold-relocating one stopped Kata Runtime

This is an operator-only, one-Runtime move between Finite-owned Kata hosts.
It preserves the existing Runtime ID, Agent Principal, durable state ID, image
artifact, and state schema. It does not retire or purge the source. The source
compute and state remain stopped and intact until the target has passed its
observation window.

The first intended drill is the existing Upgrade Canary 0715 from lat1 to
lat3. Replace every placeholder below with a fresh read-only observation; the
name is not authority to select a Runtime.

## PRECONDITIONS

- The Core and both Runner hosts run the reviewed generation that contains the
  `runtime_relocation.v1` contract.
- A full lat1 Borg archive completed successfully after quiescing the hosted
  services, and its archive is visible from the independently held recovery
  credentials.
- Core shows the exact Kata Runtime bound to the expected source host and
  machine. Capture its Runtime ID, durable state ID, artifact, state schema,
  and Agent Principal (`npub`).
- The target Runner uses a different `source_host_id`, supports the same
  artifact/schema, has enough space, and has no compute or durable directory
  for this Runtime.
- There are no pending/running controls or retirement snapshot for the Runtime.
- The normal typed `stop` request has succeeded. Do not substitute
  `nerdctl stop`; Core must also record the Runtime offline.
- Both Runner timers are drained while staging and reviewing the request.

Abort on any mismatch. Do not delete, rename, or modify source state as part of
this procedure.

## STEPS

### 1. Capture the exact stopped source

From Core and the source host, record:

```text
PROJECT_ID
RUNTIME_ID
SOURCE_HOST_ID
SOURCE_MACHINE_ID
DURABLE_STATE_ID
RUNTIME_ARTIFACT_ID
STATE_SCHEMA_VERSION
EXPECTED_AGENT_NPUB
```

Verify the canonical source container exists and is stopped, the Core stop
receipt succeeded for this exact binding, and the durable tree is the one
named by the RuntimeSpec. Keep the source compute stopped.

Locate the deployed Runner binary from
`systemctl cat finite-saas-runner.service`. Use that exact binary on both
hosts so the manifest algorithm is identical:

```sh
sudo <runner-bin> state-manifest \
  --path '<source-work-root>/kata/<durable-state-id>'
```

Record the 64-character `SOURCE_MANIFEST`. The command follows no symlinks,
hashes file contents, paths, modes, and symlink targets, and rejects special
files.

### 2. Stage a provider-independent copy

The transfer below is initiated on the operator Mac. SSH encrypts both hops;
the Mac forwards the stream and does not retain a plaintext copy. The full Borg
archive remains the off-host, independently recoverable worst-case copy.

First create only the exact absent target parent:

```sh
ssh <target-host> \
  "sudo install -d -m 0700 '<target-work-root>/kata'"
```

Then stream one stopped durable directory, preserving ownership, modes, ACLs,
xattrs, hard links, and sparse files:

```sh
ssh <source-host> \
  "sudo tar --acls --xattrs --numeric-owner --sparse -C '<source-work-root>/kata' -cpf - '<durable-state-id>'" \
| ssh <target-host> \
  "sudo tar --acls --xattrs --numeric-owner --sparse -C '<target-work-root>/kata' -xpf -"
```

Do not add `--dereference`. Do not use a recursive copy that can cross into
another Runtime.

On the target, compute `TARGET_MANIFEST` using its deployed Runner binary:

```sh
sudo <runner-bin> state-manifest \
  --path '<target-work-root>/kata/<durable-state-id>'
```

Require `TARGET_MANIFEST` to equal `SOURCE_MANIFEST` exactly. Also require that
the target has no container named `SOURCE_MACHINE_ID`.

### 3. Enqueue the exact relocation

On the Core host, load `/etc/finite/core.env` in a root shell without printing
it, then invoke the system-installed Core CLI:

```sh
sudo sh -c '
  set -a
  . /etc/finite/core.env
  set +a
  exec /run/current-system/sw/bin/finite-saas-core runtime-cold-relocate-exact \
  --project-id "<project-id>" \
  --expected-agent-runtime-id "<runtime-id>" \
  --expected-source-host-id "<source-host-id>" \
  --expected-source-machine-id "<source-machine-id>" \
  --target-source-host-id "<target-source-host-id>" \
  --expected-agent-npub "<expected-agent-npub>" \
  --durable-state-manifest-sha256 "<source-manifest>" \
  --admin-email "<operator-email>" \
  --admin-workos-user-id "<operator-workos-user-id>"
'
```

Review the returned request. It must contain the existing Runtime ID, exact
target host, and `runtime_relocation.v1` envelope. Re-enable only the target
Runner timer.

The target Runner fails closed unless:

- the request is leased by the exact target host;
- RuntimeSpec, Runtime ID, durable state ID, machine name, and target path all
  agree;
- the staged tree still matches the approved manifest;
- `agent/identity.json` is a regular file;
- target compute is absent before launch; and
- the launched `/contact` endpoint exposes `EXPECTED_AGENT_NPUB`.

Only after those checks does Core replace the source binding. The Runner
resolves fresh target-host secrets through the normal launch path; durable
state is never used as the secret transport.

## VERIFY

- The relocation creation request is `running`.
- Core still has the same Project, Runtime ID, artifact, state schema, and
  Agent Principal, now bound to the target host and same machine name.
- The target container is running and healthy.
- Finite Chat receives a round trip from the existing Agent Principal.
- Sites, Brain, workspace files, Hermes memory, and installed skills expected
  for the canary are present.
- Source compute remains stopped and source durable state still exists.
- No source Runner work was allowed to restart the old binding.

Observe the canary before broad use. Record request ID, both manifests, exact
source/target bindings, Borg archive name, timestamps, and verification result;
record no secret values.

## ROLLBACK

Before Core switches the binding, a failed request removes only target compute;
it preserves the existing Core Runtime/link and both durable trees. Fix or
discard the target staging directory only after diagnosing the failure.

After Core switches the binding, do not manually start source compute: that
would create two writers. Stop the target through Core first. A reverse
relocation requires a new exact transaction, but the old source canonical path
now contains a stale copy. Preserve that stale directory under an explicitly
approved, non-canonical rollback name; then stage the stopped target tree into
the absent canonical path, verify its manifest, and use the same contract with
source/target reversed. The current implementation intentionally does not
rename or delete either copy automatically.

If the target modified durable state and cannot be stopped cleanly, fail closed.
Preserve both sides and restore the named pre-move Borg archive to an empty
recovery target rather than guessing which tree is canonical.
