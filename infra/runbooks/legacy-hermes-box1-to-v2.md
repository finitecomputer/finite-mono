# Migrate one box1 Hermes bot to a v2 Runtime

This procedure creates a normal v2 Agent on whichever Runner host Core places
it, converts compatible Hermes state, and transfers a sealed copy of the
complete legacy `/home/node` while both sides are single-writer safe. It
leaves box1 frozen for rollback and never converts a box1 identity into a v2
identity.

It has two modes. The default mode activates memories and the workspace, dev
and uploads trees, and keeps skills, cron and integrations review-only. The
opt-in **full-fidelity** mode (`--mode full-fidelity` on both `manifest` and
`install`) also activates the bot's persona, user-facing settings, Telegram
connection and pairing list, user-created skills, helper scripts, scheduled
jobs and loose home files, so the person keeps talking to the same bot in the
same Telegram chat. [The contract](../../finitecomputer-v2/docs/legacy-hermes-migration-contract.md#full-fidelity-mode)
lists what it activates and what it reports as left out.

Record the explicitly authorized source owner, namespace, StatefulSet, PVC,
PV path, image reference, image digest, container image ID and Hermes version
from fresh read-only evidence for this bot. Record the target Runtime/Project,
Principal, Runner host and durable path separately. A matching display name is
not sufficient authority. The source snapshot must account for every path below
`/home/node`; unknown safe data is preserved, executable/identity material is
quarantined, and unsafe or unreadable entries fail closed. Verify capacity
immediately before target creation.

Commands below use placeholders. Substitute only identities from that private
evidence record. Product rollout work is tracked in [FIN-44](https://linear.app/finitecomputer/issue/FIN-44).

| Placeholder | Source of truth |
| --- | --- |
| `SOURCE_NAMESPACE` | the bot's box1 namespace (also its StatefulSet name) |
| `SOURCE_PV_PATH` | `/var/lib/rancher/k3s/storage/<pv>_SOURCE_NAMESPACE_home-SOURCE_NAMESPACE-0`, where `<pv>` is `kubectl get pvc home-SOURCE_NAMESPACE-0 -o jsonpath='{.spec.volumeName}'`; never reuse another bot's path |
| `SOURCE_IMAGE_REFERENCE`, `SOURCE_IMAGE_MANIFEST_DIGEST`, `SOURCE_CONTAINER_IMAGE_ID` | the running pod's image, `ctr images ls` digest and `containerStatuses[0].imageID` |
| `TARGET_HOST` | the Runner host Core placed the new Runtime on (Core placement record), for example `lat5` |
| `TARGET_DATA` | that Runtime's durable `/data` path on `TARGET_HOST` |
| `MODE` | `default` or `full-fidelity`; the same value for `manifest` and `install` |

## Preconditions

- The reviewed commit passed the migration unit tests and the real v0.14
  export/import compatibility test against the exact intended target image.
  Supported target package versions are 0.20.0 and 0.21.0; a version match
  alone does not prove an image. Full-fidelity mode also requires
  `finitechat/tests/hermes/test_legacy_hermes_full_fidelity.py` to pass
  against that image's Hermes environment.
- Full-fidelity mode with Telegram needs a Runtime image built with
  `infra/images/patches/hermes-telegram-preserve-pending.patch`. Without it the
  target gateway drops every Telegram message queued during the outage, and
  the importer refuses to install.
- Before capturing the rehearsal Recovery Set, the values-free storage check
  in step 3 passed against the live source pod. It proved a read-only root,
  the selected bot PVC at `/home/node`, and no other writable durable mount.
  Repeat the check immediately before cutover.
- The intended target uses an already published, digest-pinned canonical
  Runtime image whose durable smoke proves the selected Hermes version. The
  offline import receipt must record that version.
- `scripts/finite-status --json` is retained and green. If the installed
  shortcut is absent, stage every file named in `STATUS_BUNDLE_FILES`
  (`scripts/finite_status.py`) into one directory, as described in
  [the runbook index](README.md#standing-boundaries); this is the canonical status
  check, not a product called "Finite Status."
- A fresh box1 off-host backup completed, and the Recovery Set can restore to
  an empty scratch target. Record the archive name without recording keys.
- The exact reviewed tools completed a real-data rehearsal, in the same mode,
  against an isolated restore of the selected bot's Recovery Set. Record its
  manifest hash, counts, duration, source-volume inventory hash, source-home
  snapshot hash, automatic dispositions, media-path result, and cleanup
  outcome. In full-fidelity mode also record the `fidelity` plan summary:
  activated and review-only skills, cron outcomes, placed and withheld home
  entries, and left-out channels, tool keys and Google state. The inventory
  and snapshot must cover the entire restored `/home/node` tree and report
  zero structurally blocked entries. A synthetic test is not this gate.
- The rehearsal produced `sites.json` from the authoritative legacy control
  plane and proved every local Site source path exists in the source snapshot.
  It also produced `integrations.json`, containing configuration names and
  migration policies but no secret values. Neither command republishes a Site.
- Austin decided, per bot, what happens to each item the plan leaves out
  (Matrix, Signal, Google, tool keys without a target equivalent, paused jobs).
  The importer never copies a shared tool key.
- The owner has explicitly authorized target creation and the later cutover
  outage. These are separate from authorization to decommission box1.
- `TARGET_HOST` has a free 4-vCPU/8-GiB Runtime slot and free disk of at least
  three times the sealed bundle size plus 10 GiB.
- The target is a fresh Runtime under the exact verified account. Record
  `PROJECT_ID`, `RUNTIME_ID`, `MACHINE_ID`, `DURABLE_STATE_ID`, artifact,
  schema, `TARGET_HOST`, `TARGET_DATA`, and target Agent `npub`. Its
  structured-memory store must contain zero facts, and in full-fidelity mode it
  must have no Telegram connection, no Telegram pairing entries, no cron jobs
  and an uncustomized `SOUL.md`; the importer refuses otherwise.
- Known box1 credential stores, old identities, and active gateway state are
  preserved only inside the root-only source snapshot. Treat the entire
  snapshot and all user-authored files as sensitive. In full-fidelity mode the
  Telegram bot token is the only source credential written to an active target
  path, exactly where finite-agentd's Telegram connect would put it.
- Hermes cache-only audio and image media stays in the sealed source snapshot.
  Identify one old conversation containing media for step 8, or explicitly
  record that the frozen export contains none.
- The legacy local FiniteBrain working tree and identity are preserved in the
  sealed snapshot but never activated. Active Brain access requires an explicit
  grant to the new Agent Principal and a fresh Working Tree.

Abort on any mismatch. Do not delete source compute, PVC data, backup data, or
target pre-import state in this runbook.

## Steps

### 1. Stage and prove the reviewed migration tool

Record the existing digest-pinned target image as `TARGET_RUNTIME_IMAGE` and
its successful durable-smoke artifact. Archive only the reviewed migration
modules and source launcher from the reviewed commit, then calculate and
approve that archive's exact hash. Stage it into a new root-only directory on
`TARGET_HOST`; do not copy the checkout or build a new Runtime image.

```sh
TARGET_HOST='<RUNNER_HOST_FROM_CORE_PLACEMENT>'
TARGET_RUNTIME_IMAGE='<EXISTING_TARGET_IMAGE@sha256:...>'
REVIEWED_CHECKOUT='<REVIEWED_CHECKOUT>'
REVIEWED_COMMIT='<REVIEWED_COMMIT>'
MIGRATION_TOOL_ARCHIVE='<LOCAL_TOOL_ARCHIVE>'
MIGRATION_TOOL_DIR='<TARGET_HOST_TOOL_DIR>'
MIGRATION_TOOL_SHA256='<REVIEWED_TOOL_SHA256>'
MIGRATION_TOOL_FILES='legacy-hermes-source legacy_hermes_migration.py
  legacy_hermes_contract.py legacy_hermes_source.py legacy_hermes_target.py
  legacy_hermes_fidelity.py'

git -C "$REVIEWED_CHECKOUT" archive --format=tar \
  "$REVIEWED_COMMIT" -- \
  scripts/legacy_hermes_migration.py \
  scripts/legacy_hermes_contract.py \
  scripts/legacy_hermes_source.py \
  scripts/legacy_hermes_target.py \
  scripts/legacy_hermes_fidelity.py \
  scripts/legacy-hermes-source \
  >"$MIGRATION_TOOL_ARCHIVE"
test "$(sha256sum "$MIGRATION_TOOL_ARCHIVE" | awk '{print $1}')" \
  = "$MIGRATION_TOOL_SHA256"
ssh "$TARGET_HOST" "sudo install -d -m 0700 '$MIGRATION_TOOL_DIR'"
ssh "$TARGET_HOST" "sudo tee '$MIGRATION_TOOL_DIR/tool.tar' >/dev/null" \
  <"$MIGRATION_TOOL_ARCHIVE"
ssh "$TARGET_HOST" "echo '$MIGRATION_TOOL_SHA256  $MIGRATION_TOOL_DIR/tool.tar' \
    | sudo sha256sum --check && \
  sudo tar -C '$MIGRATION_TOOL_DIR' --strip-components=1 -xpf \
    '$MIGRATION_TOOL_DIR/tool.tar' && \
  cd '$MIGRATION_TOOL_DIR' && sudo chmod 0500 $MIGRATION_TOOL_FILES"

ssh "$TARGET_HOST" "sudo nerdctl --namespace finite run --rm --network none \
  --mount 'type=bind,src=$MIGRATION_TOOL_DIR,dst=/opt/migration,ro' \
  --entrypoint /usr/local/bin/python3 \
  '$TARGET_RUNTIME_IMAGE' \
  /opt/migration/legacy_hermes_migration.py --help"
```

### 2. Create and identify the target

Create one normal Agent while signed in as `SOURCE_OWNER_EMAIL`. Record the
Runner host Core placed it on as `TARGET_HOST` and its durable root as
`TARGET_DATA`; do not assume a host. Send one Finite Chat round trip and record
the target Agent `npub`. Do not connect Telegram on the target. Do not take a
live filesystem archive or treat a hash of a running SQLite database as the
rollback boundary; step 6 does that after the typed stop.

### 3. Freeze the bot on box1

Start the outage only after a go/no-go review of steps 1 and 2. Freezing box1
first is what makes the Telegram hand-over safe: only one process may poll a
bot token, and a second poller makes Telegram return a conflict that drops
queued messages. Scale the exact StatefulSet to zero through k3s. Immediately
before scaling, re-prove the running container image ID and the source storage
contract. This values-free check requires a read-only root filesystem, one PVC
at `/home/node`, and only ephemeral writable mounts at `/tmp` and `/run`:

```sh
ssh box1 "sudo k3s kubectl get pod \
  --namespace SOURCE_NAMESPACE SOURCE_NAMESPACE-0 -o json | jq -e '
    (.spec.containers[] | select(.name == \"runtime\")) as \$runtime |
    \$runtime.securityContext.readOnlyRootFilesystem == true and
    ([\$runtime.volumeMounts[] |
      select((.readOnly // false) == false) | .name] | sort) ==
      [\"home\", \"run\", \"tmp\"] and
    ([\$runtime.volumeMounts[] |
      select(.name == \"home\" and .mountPath == \"/home/node\")] |
      length) == 1 and
    ([.spec.volumes[] |
      select(.name == \"home\" and
        .persistentVolumeClaim.claimName == \"home-SOURCE_NAMESPACE-0\")] |
      length) == 1 and
    ([.spec.volumes[] |
      select((.name == \"tmp\" or .name == \"run\") and has(\"emptyDir\"))] |
      length) == 2
  ' >/dev/null"
test "$(ssh box1 sudo k3s kubectl get pod \
  --namespace SOURCE_NAMESPACE SOURCE_NAMESPACE-0 \
  -o jsonpath='{.status.containerStatuses[0].imageID}')" \
  = 'SOURCE_CONTAINER_IMAGE_ID'
ssh box1 sudo k3s kubectl scale \
  --namespace SOURCE_NAMESPACE statefulset/SOURCE_NAMESPACE --replicas=0
ssh box1 sudo k3s kubectl wait \
  --namespace SOURCE_NAMESPACE --for=delete pod/SOURCE_NAMESPACE-0 --timeout=120s
ssh box1 sudo k3s kubectl get pvc \
  --namespace SOURCE_NAMESPACE home-SOURCE_NAMESPACE-0 -o wide
```

Record the freeze time. Messages the person sends from now on wait in
Telegram's queue (Telegram keeps them for 24 hours) and are answered when the
target starts in full-fidelity mode. Do not manually restart box1 after this
point. It is the rollback copy and must remain single-writer safe.

### 4. Inventory and export the frozen PVC

Create a root-only box1 staging directory outside the PVC. Transfer the exact
tool archive approved in step 1; do not copy a checkout or secret file. Verify
its hash before extracting. The single source launcher accepts only the volume,
Sites, and integrations inventories plus the two database export commands. It
reuses the source image's immutable Hermes wrapper environment instead of the
mutable user venv.

```sh
ssh box1 "sudo install -d -m 0700 '<BOX1_STAGE>'"
ssh box1 "sudo tee '<BOX1_STAGE>/tool.tar' >/dev/null" \
  <"$MIGRATION_TOOL_ARCHIVE"
ssh box1 "echo '$MIGRATION_TOOL_SHA256  <BOX1_STAGE>/tool.tar' \
    | sudo sha256sum --check && \
  sudo install -d -m 0700 '<BOX1_STAGE>/tool' && \
  sudo tar -C '<BOX1_STAGE>/tool' --strip-components=1 -xpf \
    '<BOX1_STAGE>/tool.tar' && \
  cd '<BOX1_STAGE>/tool' && sudo chmod 0500 $MIGRATION_TOOL_FILES"
```

Immediately before export, require the local source tag to resolve to the
recorded manifest digest and the frozen pod's last container image ID to match
the evidence sheet. Prove no remaining host process has the source PVC open
for writing; retain the JSON result. The check must run as root on box1 and
fails closed if it cannot inspect the real process table:

```sh
ssh box1 "sudo python3 '<BOX1_STAGE>/tool/legacy_hermes_migration.py' \
  source-writer-check --source-root 'SOURCE_PV_PATH'"
```

Inventory every entry on the PVC before staging the bundle. The root-only
inventory records paths, kinds, modes, sizes, link targets, dispositions, and
content hashes; it never embeds file contents. Known compatible paths are
`activate` or `converted`. Known identity and executable state is
`quarantine`. Known generated state is `rebuild`. Every other safe path
defaults to `preserve`. The command exits nonzero only for structurally unsafe
or unreadable entries.

External symlinks below known generated or quarantined roots are recorded as
inert metadata and keep those dispositions. They are not followed or copied
into active target state. An external symlink in an active or otherwise
unclassified path remains structurally blocked.

Run the following on box1. `SOURCE_PV_PATH` is this bot's own PV path from the
evidence record.

```sh
SOURCE_PV_PATH='SOURCE_PV_PATH'
SOURCE_IMAGE_REFERENCE='SOURCE_IMAGE_REFERENCE'
SOURCE_IMAGE_MANIFEST_DIGEST='SOURCE_IMAGE_MANIFEST_DIGEST'

test "$(sudo ctr --namespace k8s.io images ls \
  "name==$SOURCE_IMAGE_REFERENCE" | awk 'NR == 2 { print $3 }')" \
  = "$SOURCE_IMAGE_MANIFEST_DIGEST"
sudo install -d -m 0700 '<BOX1_STAGE>'
sudo ctr --namespace k8s.io run --rm \
  --user 0:0 \
  --mount "type=bind,src=$SOURCE_PV_PATH,dst=/source,options=rbind:ro" \
  --mount 'type=bind,src=<BOX1_STAGE>,dst=/migration,options=rbind:rw' \
  --mount 'type=bind,src=<BOX1_STAGE>/tool,dst=/opt/migration,options=rbind:ro' \
  "$SOURCE_IMAGE_REFERENCE" 'SOURCE_NAMESPACE-hermes-volume-inventory' \
  /opt/migration/legacy-hermes-source source-volume-inventory \
  --source-root /source \
  --output /migration/source-volume-inventory.json
sudo chmod 0600 '<BOX1_STAGE>/source-volume-inventory.json'
sudo sha256sum '<BOX1_STAGE>/source-volume-inventory.json'

SOURCE_CONTROL_PLANE_ROOT='<VERIFIED_SOURCE_CONTROL_PLANE_ROOT>'
SOURCE_SITES_SCRATCH='<BOX1_STAGE>/sites-control-plane-read'
SOURCE_PUBLISHED_ENDPOINTS='<BOX1_STAGE>/published-endpoints.json'
sudo python3 - "$SOURCE_CONTROL_PLANE_ROOT/control-plane.sqlite" \
  "$SOURCE_SITES_SCRATCH/control-plane.sqlite" <<'PY'
import os
from pathlib import Path
import sqlite3
import sys

source, snapshot = map(Path, sys.argv[1:])
if not source.is_file() or source.is_symlink():
    raise SystemExit("verified source control database is missing or unsafe")
os.umask(0o077)
snapshot.parent.mkdir(mode=0o700, exist_ok=False)
with sqlite3.connect(source.resolve().as_uri() + "?mode=ro", uri=True) as reader:
    reader.execute("PRAGMA query_only=ON")
    with sqlite3.connect(snapshot) as writer:
        reader.backup(writer)
PY

sudo sh -c 'umask 077; \
  /run/current-system/sw/bin/finited \
  --workspace-root /etc/nixos/workspaces/ovh-fc-1 \
  --control-plane-root "$1" \
  list-published-endpoints \
  --payload "$2" > "$3"' sh "$SOURCE_SITES_SCRATCH" \
  '{"machineId":"SOURCE_NAMESPACE"}' "$SOURCE_PUBLISHED_ENDPOINTS"

sudo ctr --namespace k8s.io run --rm \
  --user 0:0 \
  --mount "type=bind,src=$SOURCE_PV_PATH,dst=/source,options=rbind:ro" \
  --mount 'type=bind,src=<BOX1_STAGE>,dst=/migration,options=rbind:rw' \
  --mount 'type=bind,src=<BOX1_STAGE>/tool,dst=/opt/migration,options=rbind:ro' \
  "$SOURCE_IMAGE_REFERENCE" 'SOURCE_NAMESPACE-hermes-sites-inventory' \
  /opt/migration/legacy-hermes-source source-sites-inventory \
  --published-endpoints /migration/published-endpoints.json \
  --source-volume-inventory /migration/source-volume-inventory.json \
  --expected-machine-id SOURCE_NAMESPACE \
  --output /migration/sites.json

sudo ctr --namespace k8s.io run --rm \
  --user 0:0 \
  --mount "type=bind,src=$SOURCE_PV_PATH,dst=/source,options=rbind:ro" \
  --mount 'type=bind,src=<BOX1_STAGE>,dst=/migration,options=rbind:rw' \
  --mount 'type=bind,src=<BOX1_STAGE>/tool,dst=/opt/migration,options=rbind:ro' \
  "$SOURCE_IMAGE_REFERENCE" 'SOURCE_NAMESPACE-hermes-integrations-inventory' \
  /opt/migration/legacy-hermes-source source-integrations-inventory \
  --source-root /source \
  --source-volume-inventory /migration/source-volume-inventory.json \
  --output /migration/integrations.json

sudo ctr --namespace k8s.io run --rm \
  --user 0:0 \
  --mount "type=bind,src=$SOURCE_PV_PATH,dst=/source,options=rbind:ro" \
  --mount 'type=bind,src=<BOX1_STAGE>,dst=/migration,options=rbind:rw' \
  --mount 'type=bind,src=<BOX1_STAGE>/tool,dst=/opt/migration,options=rbind:ro' \
  --env HERMES_HOME=/source/.hermes \
  "$SOURCE_IMAGE_REFERENCE" 'SOURCE_NAMESPACE-hermes-export' \
  /opt/migration/legacy-hermes-source source-export \
  --source-database /source/.hermes/state.db \
  --output /migration/sessions.jsonl

sudo ctr --namespace k8s.io run --rm \
  --user 0:0 \
  --mount "type=bind,src=$SOURCE_PV_PATH,dst=/source,options=rbind:ro" \
  --mount 'type=bind,src=<BOX1_STAGE>,dst=/migration,options=rbind:rw' \
  --mount 'type=bind,src=<BOX1_STAGE>/tool,dst=/opt/migration,options=rbind:ro' \
  --env HERMES_HOME=/source/.hermes \
  "$SOURCE_IMAGE_REFERENCE" 'SOURCE_NAMESPACE-hermes-memory-export' \
  /opt/migration/legacy-hermes-source source-memory-snapshot \
  --source-database /source/.hermes/memory_store.db \
  --output /migration/memory_store.db
```

The Sites command reads only the control-plane export and source-volume
inventory. It fails if a locally run Site points outside `/home/node` or its
source is missing. Verify the live control database location before taking
the read snapshot; do not infer it from the parent directory. Invoke
`finited` against the scratch copy because opening a live or incorrect control
root can initialize or update administrative state. The integrations command
reads configuration without executing it and emits names and policies only.
Inspect both root-only files; they must contain no credential values.

The session exporter first uses SQLite's backup API, then reads the scratch
database through Hermes v0.14. The memory snapshot also uses SQLite's backup
API. Retain the inventory hash and both export commands' counts, byte counts,
and SHA-256 output.

### 5. Stage the complete snapshot and active bundle

Run these commands from the trusted operator workstation. The first stream
captures all of `/home/node` without following symlinks. The remaining
streams stage the subset that the importer converts or places into active and
review-only target paths. Full-fidelity mode needs no extra staging: it reads
SOUL.md, config.yaml, the Telegram token, both pairing stores and the loose
home files from the hash-verified `source-home.tar`.

```sh
SOURCE_PV_PATH='SOURCE_PV_PATH'
BOX1_STAGE='<BOX1_STAGE>'
BUNDLE='<TARGET_HOST_BUNDLE>'
SOURCE_VOLUME_INVENTORY_SHA256='<SOURCE_VOLUME_INVENTORY_SHA256>'

ssh "$TARGET_HOST" "sudo install -d -m 0700 \
  '$BUNDLE/payload/hermes' '$BUNDLE/payload/home'"
ssh box1 "sudo tar --sort=name --numeric-owner --format=pax \
  --hard-dereference --one-file-system --acls --xattrs \
  -C '$SOURCE_PV_PATH' -cpf - ." \
  | ssh "$TARGET_HOST" "sudo sh -c 'umask 077; cat >\"$BUNDLE/payload/source-home.tar\"'"
ssh box1 "sudo tar -C '$SOURCE_PV_PATH/.hermes' -cpf - \
  memories skills cron/jobs.json scripts" \
  | ssh "$TARGET_HOST" "sudo tar -C '$BUNDLE/payload/hermes' -xpf -"
ssh box1 "sudo tar -C '$SOURCE_PV_PATH' \
  --exclude='dev/reap-video/venv' -cpf - workspace dev uploads" \
  | ssh "$TARGET_HOST" "sudo tar -C '$BUNDLE/payload/home' -xpf -"
ssh box1 "sudo tar -C '$BOX1_STAGE' -cpf - sessions.jsonl memory_store.db" \
  | ssh "$TARGET_HOST" "sudo tar -C '$BUNDLE/payload' -xpf -"
ssh box1 "sudo tar -C '$BOX1_STAGE' -cpf - \
  source-volume-inventory.json sites.json integrations.json" \
  | ssh "$TARGET_HOST" "sudo tar -C '$BUNDLE' -xpf -"
```

If the bot lacks one of `scripts`, `cron/jobs.json`, `workspace`, `dev` or
`uploads`, drop it from the matching `tar` list rather than creating it. Do not
add an exclusion to the complete source snapshot. `--hard-dereference` stores
hard-linked file content independently; it does not follow symlinks. Never add
`--dereference`. If the inventory or manifest finds a special file, unreadable
entry, an escaping link outside known generated or quarantined state, a missing
path, or a digest mismatch, stop and fix the fleet policy or source condition
in review.

Seal and verify the bundle with the reviewed tool mounted read-only into the
existing target image. `MODE` is `default` or `full-fidelity`:

```sh
ssh "$TARGET_HOST" "sudo nerdctl --namespace finite run --rm --network none \
  --mount 'type=bind,src=$BUNDLE,dst=/migration' \
  --mount 'type=bind,src=$MIGRATION_TOOL_DIR,dst=/opt/migration,ro' \
  --entrypoint /usr/local/bin/python3 \
  '$TARGET_RUNTIME_IMAGE' \
  /opt/migration/legacy_hermes_migration.py manifest \
  --bundle /migration \
  --mode '$MODE' \
  --source-host-id box1 \
  --source-machine-id SOURCE_NAMESPACE \
  --source-owner-email SOURCE_OWNER_EMAIL \
  --source-hermes-version 0.14.0 \
  --source-image-reference 'SOURCE_IMAGE_REFERENCE' \
  --source-image-manifest-digest 'SOURCE_IMAGE_MANIFEST_DIGEST' \
  --source-container-image-id 'SOURCE_CONTAINER_IMAGE_ID' \
  --source-volume-inventory-sha256 '$SOURCE_VOLUME_INVENTORY_SHA256'"

ssh "$TARGET_HOST" "sudo nerdctl --namespace finite run --rm --network none --read-only \
  --mount 'type=bind,src=$BUNDLE,dst=/migration,ro' \
  --mount 'type=bind,src=$MIGRATION_TOOL_DIR,dst=/opt/migration,ro' \
  --entrypoint /usr/local/bin/python3 \
  '$TARGET_RUNTIME_IMAGE' \
  /opt/migration/legacy_hermes_migration.py verify --bundle /migration"

ssh "$TARGET_HOST" "sudo sha256sum '$BUNDLE/manifest.json'"
```

Record `MANIFEST_SHA256`, bundle size, and the `source_snapshot` summary.
Compare the manifest session, message, and memory-fact counts with step 4.
The manifest must prove that every inventory entry exists in `source-home.tar`
with matching type, mode, size, symlink target, and content hash. It must have
zero structurally blocked entries. It must also bind the complete Sites and
integrations inventories with no secret values.

In full-fidelity mode, review the manifest's `fidelity` section before
approval and compare it with the rehearsal record: `telegram.activate`,
token and home-channel sources and pairing counts; `skills.user` and
`skills.managed_not_activated`; `home.placed` and `home.withheld`;
`config.adopted_keys` and `config.withheld_top_level_keys`; every cron job;
and `omitted` (channels, tool keys, Google, Sites). It contains names and
counts only. `grep` the manifest for the token's numeric bot id and stop if it
appears.

### 6. Stop the target through Core

As the selected bot, submit the typed stop request for the exact `PROJECT_ID`:

```text
POST /api/core/v1/me/projects/<PROJECT_ID>/runtime/stop
```

Wait for the request to succeed. Require Core to report the target offline,
the container/task on `TARGET_HOST` absent or stopped as defined by the Runner,
and the durable root still present. Do not substitute `nerdctl stop`.

Now capture the quiescent protected hashes:

```sh
ssh "$TARGET_HOST" "sudo sha256sum \
  '$TARGET_DATA/agent/identity/identity.json' \
  '$TARGET_DATA/agent/client.sqlite3'"
```

Record `TARGET_IDENTITY_SHA256` and `TARGET_CHAT_CLIENT_SHA256` separately.
Take a scoped pre-import archive of the exact fresh `TARGET_DATA` and verify
that archive can restore to an empty scratch directory. The importer will also
prove that the fresh structured-memory store has zero facts before replacing
it.

### 7. Install offline into the exact target

Run one networkless importer against the stopped target, in the same `MODE`
as the manifest. The bundle is read-only; only the exact target `/data` and a
private, bounded `/tmp` tmpfs are writable. The tmpfs is required because
SQLite can spill temporary work during a large session import; without it, the
read-only container root causes a late `disk I/O error` even though `/data`
remains healthy:

```sh
ssh "$TARGET_HOST" "sudo nerdctl --namespace finite run --rm --network none --read-only \
  --tmpfs '/tmp:rw,nosuid,nodev,noexec,size=4294967296' \
  --mount 'type=bind,src=$BUNDLE,dst=/migration,ro' \
  --mount 'type=bind,src=$TARGET_DATA,dst=/data' \
  --mount 'type=bind,src=$MIGRATION_TOOL_DIR,dst=/opt/migration,ro' \
  --entrypoint /usr/local/bin/python3 \
  '$TARGET_RUNTIME_IMAGE' \
  /opt/migration/legacy_hermes_migration.py install \
  --bundle /migration \
  --target-root /data \
  --mode '$MODE' \
  --expected-source-machine-id SOURCE_NAMESPACE \
  --expected-manifest-sha256 '<MANIFEST_SHA256>' \
  --expected-target-identity-sha256 '<TARGET_IDENTITY_SHA256>' \
  --expected-target-chat-client-sha256 '<TARGET_CHAT_CLIENT_SHA256>'"
```

Require a receipt at `TARGET_DATA/migration/legacy-hermes-v2/receipt.json`.
Its status must be `installed-offline-awaiting-runtime-verification`; its
manifest hash and source machine must match approval; its protected hashes
must match step 6. In full-fidelity mode its `fidelity.installed` section
records every activation and omission, including cron outcomes per job id.
The importer also keeps the target's pre-import `config.yaml` and `SOUL.md`
under `migration/legacy-hermes-v2/preimport-state/full-fidelity/`.

In full-fidelity mode this step is when Telegram is connected: the token is
written into the stopped target's `config.yaml` only now, after box1 is
frozen. Nothing polls until step 8.

### 8. Restart and verify

As the selected bot, submit the typed restart request:

```text
POST /api/core/v1/me/projects/<PROJECT_ID>/runtime/restart
```

Require every verification check below. Keep box1 at zero replicas and keep both
recovery archives through a minimum 24-hour observation window.

## Verify

- Core still shows the exact target Project, Runtime, artifact, schema,
  `TARGET_HOST`, and Agent `npub` recorded in step 2.
- The target identity and Chat client SHA-256 values are unchanged.
- The receipt session/message counts equal the sealed manifest.
- The receipt Sites summary equals the manifest. Every legacy endpoint is
  listed, every local source path is preserved, and no Site was republished by
  the importer.
- The receipt integrations summary equals the manifest.
- The manifest binds the whole-volume inventory and `source-home.tar`. Every
  source entry appears in the sealed snapshot with matching metadata and hash,
  and the inventory has zero structurally blocked entries.
- The installed snapshot exists at
  `/data/migration/legacy-hermes-v2/preserved/source-home.tar`, is mode `0600`,
  matches the manifest hash, and restores into an empty scratch directory.
- A sampled old Hermes conversation is present but does not own a Telegram,
  webhook, or other live gateway route. For the preselected media sample, an
  admitted file path resolves under `/data/workspace/legacy-box1`; a
  cache-only media path is explicitly recorded as preserved in the sealed
  snapshot and is not presented as active.
- The bot's imported memories are visible, and the rebuilt memory store holds
  exactly the fact count sealed in the frozen manifest.
- Files exist under `/data/workspace/legacy-box1/` and a sample of manifested
  hashes and contained symlinks matches.
- Brain access is recorded as pending fresh target authorization and sync.
- A new Finite Chat message receives exactly one target reply.
- Default mode only: legacy skills exist only under
  `/data/migration/legacy-hermes-v2/review-only/skills`, no job exists in the
  active target cron path, and the manifest and receipt contain no write
  target for the target's managed Hermes configuration.
- Full-fidelity mode only:
  - The dashboard shows Telegram connected with the expected default chat and
    the approved people from the receipt's pairing count.
  - A message the operator or owner sent during the outage (or the owner's
    first message after restart) is answered in the same Telegram chat with no
    pairing prompt. An unknown account still receives a pairing code.
  - `hermes cron list` inside the Runtime shows every job from the receipt.
    Recurring jobs have a next run in the future and none fired at start.
    Jobs the receipt lists as `paused:*` (missed one-shots, Matrix or Signal
    delivery, a missing script) are paused with their reason; tell Austin
    which ones they are.
  - The persona answers in its usual voice; `SOUL.md` in
    `/data/agent/hermes-home` matches the source with `/home/node` paths
    rewritten.
  - Activated user skills appear under `/data/agent/hermes-home/skills`; the
    receipt's review-only clashes stay under the review-only area.
- box1 has zero pods for the bot; its PVC and off-host archive remain intact.
- No other box1 bot or Runtime on `TARGET_HOST` restarted or changed artifact.
- `scripts/finite-status --json` is retained and green.

Complete the receipt status in the retained operator evidence, not by editing
the receipt inside `/data`. Record IDs, digests, hashes, counts, timestamps,
and outcomes; record no token, key, message content, or secret value.

## Rollback

Before target restart, leave box1 stopped. If install fails, retain the failed
bundle and receipt work directory for diagnosis, restore the scoped fresh
target archive to an empty `TARGET_DATA`, and re-check its original identity
and Chat hashes. Never repair a partial target by hand.

After target restart, stop the target through Core before any rollback. In
full-fidelity mode the target now owns the Telegram bot token, so disconnect it
on the target before box1 polls again: with the target stopped, restore the
scoped fresh target archive (which has no Telegram connection), or, if the
target must keep its other state, copy
`migration/legacy-hermes-v2/preimport-state/full-fidelity/agent__hermes-home__config.yaml`
back over `agent/hermes-home/config.yaml`. Confirm `gateway.platforms.telegram`
holds no token before continuing. Only then, and after proving target compute
is stopped, scale only `statefulset/SOURCE_NAMESPACE` back to one and verify
the original bot answers in Telegram. Never run both writers, and never let
both hosts poll the same bot token.

Do not delete the target, source PVC, staging bundle, or either recovery
archive during rollback. Decommission and credential reauthorization are later
changes with separate approval.
