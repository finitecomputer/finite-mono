# Migrate one box1 Hermes bot to lat3

This procedure creates a normal v2 Agent on lat3, converts compatible Hermes
state, and transfers a sealed copy of the complete legacy `/home/node` while
both sides are single-writer safe. It leaves box1 frozen for rollback and
never converts a box1 identity into a v2 identity.

Create a private evidence sheet for each migration. Keep owner names, account
emails, bot names, production identifiers, hashes, timelines, and detailed
exercise journals in the organization Brain or a mode-0700 operator evidence
directory. Do not commit them to this public repository.

This generic procedure recommends a minimum 24-hour observation window. A
shorter owner-approved canary window is a per-migration decision and does not
change the default.

## Private evidence sheet

Resolve and record every source value before mutation:

| Field | Approved source value |
| --- | --- |
| source host | `box1` |
| machine / namespace | `<SOURCE_MACHINE_ID>` / `<SOURCE_NAMESPACE>` |
| verified owner account | `<SOURCE_OWNER_EMAIL>` |
| authoritative SaaS login | account that owns the target Project |
| Sites mailbox | mailbox used for Sites grants; may differ from the SaaS login |
| StatefulSet / pod | `<SOURCE_STATEFULSET>` / `<SOURCE_POD>` |
| PVC | `<SOURCE_PVC>` |
| PV | `<SOURCE_PV>` |
| PV path | `<SOURCE_PV_PATH>` |
| source Hermes | `0.14.0` |
| source image reference | `<SOURCE_IMAGE_REFERENCE>` |
| source manifest digest | `<SOURCE_IMAGE_MANIFEST_DIGEST>` |
| source container image ID | `<SOURCE_CONTAINER_IMAGE_ID>` |
| target Hermes | `0.20.0` |
| durable source mount | `<SOURCE_PVC>` mounted at `/home/node` |
| non-PVC runtime storage | read-only root filesystem; `/tmp` and `/run` are ephemeral `emptyDir` mounts |
| complete source-home snapshot | frozen entry count, byte count, inventory hash, archive hash, and zero structurally blocked entries |
| unknown source data | automatically preserved inside `source-home.tar`; no owner classifies individual files |
| Hermes data | frozen session, message, transcript, and structured-memory counts and hashes |
| executable behavior | frozen skill and scheduled-job counts; preserve review-only and activate none during import |
| legacy Sites | authoritative endpoint records, route cross-check, and required source paths; publish none during import |
| external integrations | configuration names and migration policies without credential values; activate none during import |

Re-read every value before mutation. A name match is not authority. Stop if
the PVC UID/path, image digest, owner, or Hermes version changed.

For legacy version evidence, run only `hermes --version` inside the source
container. Do not run `hermes-agent --version`: the legacy entry point does
not support that flag and treats it as a normal Agent prompt, which can make a
provider request and write session/debug files to the user's PVC. If this has
already happened, stop and quarantine only the generated files after explicit
approval while the source is writer-free. Prove both Hermes databases stayed
unchanged.

Do not interpret an empty legacy `publishedEndpoints` export as proof that the
bot has no Sites. Compare it with the bot namespace's live Traefik routes,
Services, listeners, and preserved source paths. Resolve any disagreement into
one reviewed, values-free endpoint input before rehearsal; unknown ownership,
authentication, desired state, or source fails closed.

Inventory every symlink. Links contained within an admitted data root may be
preserved. Links from generated or quarantined trees remain inert metadata.
Any new escaping symlink in an active or unclassified path is a hard stop, not
an instruction to add `--dereference`.

The old Runtime could put durable data anywhere below `/home/node`. The three
admitted workspace roots are not proof that nothing else matters. Rehearsal
must scan the whole PVC and account for every file before this evidence sheet
is complete. The pod's root filesystem is declared read-only, so files outside
`/home/node` can only come from the immutable image, read-only mounts, or the
ephemeral `/tmp` and `/run` mounts. Step 3 rechecks that live storage shape. A
new writable mount is a hard stop.

User-authored files may exist outside the old active-path list. They require no
owner decision: unknown safe paths default to `preserve` and are sealed into
`source-home.tar`.
Known identities and executable behavior are `quarantine`; known generated
state is `rebuild`; both remain present in the snapshot but inactive. Only a
special file, unreadable entry, escaping symlink, concurrent writer, or
integrity mismatch blocks migration.

Record lat3 capacity immediately before target creation. A prior readiness
snapshot is not a reservation.

## Preconditions

- The reviewed commit passed the migration unit tests and the real
  v0.14-export-to-v0.20-import compatibility test.
- Before capturing the rehearsal Recovery Set, the values-free storage check
  in step 3 passed against the live source pod. It proved a read-only root,
  the source PVC at `/home/node`, and no other writable durable mount. Repeat
  the check immediately before cutover.
- The intended target uses an already published, digest-pinned canonical
  Runtime image whose durable smoke proves Hermes v0.20. This migration does
  not require publishing or rolling a new image.
- `scripts/finite-status --json` passes the status gate below. If the installed
  shortcut is absent, stage `scripts/finite-status` and
  `scripts/finite_status.py` together as described in
  [the runbook index](README.md#standing-rules); this is the canonical status
  check, not a product called “Finite Status.”
- A fresh box1 off-host backup completed, and the Recovery Set can restore to
  an empty scratch target. Record the archive name without recording keys.
  A rehearsal restore includes both the complete source PVC subtree and the
  archive's matching SQLite snapshot tree. The backup intentionally excludes
  live `-wal` and `-shm` files, so a raw database file from the PVC subtree is
  preservation evidence, not a valid rehearsal export source by itself.
- The exact reviewed tools completed a real-data rehearsal against an isolated
  restore of the source Recovery Set. Record its manifest hash, counts, duration,
  source-volume inventory hash, source-home snapshot hash, automatic
  dispositions, media-path result, and cleanup outcome. The inventory and
  snapshot must cover the entire restored `/home/node` tree and report zero
  structurally blocked entries. A synthetic test is not this gate.
- The rehearsal produced `sites.json` from the authoritative legacy control
  plane and proved every local Site source path exists in the source snapshot.
  It also produced `integrations.json`, containing configuration names and
  migration policies but no secret values. Neither command republishes a Site
  or activates an integration.
- The owner has explicitly authorized target creation and the later cutover
  outage. These are separate from authorization to decommission box1.
- The private evidence sheet names the authorized canary and current order.
  The reviewed PR and this runbook do not grant execution authority.
  Code review, target creation, source freeze, import, channel re-pairing,
  behavior restoration, and decommission remain separate approvals.
- lat3 has one free 4-vCPU/8-GiB Runtime slot and free disk of at least three
  times the sealed bundle size plus 10 GiB.
- The target is a fresh Runtime under the exact verified owner account. Record
  `PROJECT_ID`, `RUNTIME_ID`, `MACHINE_ID`, `DURABLE_STATE_ID`, artifact,
  schema, host, `/data` path, and target Agent `npub`. Its structured-memory
  store must contain zero facts; the importer refuses a non-empty store.
- The evidence sheet distinguishes the authoritative SaaS login from any Sites
  mailbox. A Sites grant does not transfer Project ownership. Before cutover,
  prove that the authoritative SaaS login can see the exact target Project. A
  secondary mailbox must not create a second Agent.
- Known box1 credential stores, old identities, and active gateway state are
  preserved only inside the root-only source snapshot. They are never copied
  into active target paths. Treat the entire snapshot and all user-authored
  files as sensitive. Cron definitions, helper scripts, and legacy skills are
  also retained in review-only target paths; none is activated by this canary.
- Hermes cache-only audio and image media stays in the sealed source snapshot.
  Identify one old conversation containing media for step 8, or explicitly
  record that the frozen export contains none. Paths into admitted uploads,
  workspace, and dev trees are rewritten; cache-only paths are not.
- The legacy local FiniteBrain working tree and identity are preserved in the
  sealed snapshot but never activated. Plan a fresh Agent Principal
  delegation, Folder Key Grant, and sync using
  [the post-cutover repair brief](../../finitecomputer-v2/docs/legacy-hermes-post-cutover-repair.md).

### Finite status gate

Green is the normal requirement. A pre-existing non-green result may carry
through only when it predates this migration and was already reviewed as
unrelated to it. Record the exact section, status, and identifiers in the
private evidence sheet. Do not put user or bot names in Git.

The exception must be unchanged in the before and after reports. It cannot
touch the source bot, owner binding, target host, Runner drain, capacity,
Runtime artifact, lifecycle, identity, Chat, or recovery. Any new or worsened
result stops the migration. If the relationship is uncertain, stop. A carried
exception is not green and does not authorize its repair.

Abort on any mismatch. Do not delete source compute, PVC data, backup data, or
target pre-import state in this runbook.

## Steps

### 1. Stage and prove the reviewed migration tool

Record the existing digest-pinned target image as `TARGET_RUNTIME_IMAGE` and
its successful durable-smoke artifact. Archive only the reviewed migration
modules and source launcher from the reviewed commit, then calculate and
approve that archive's exact hash. Stage it into a new root-only lat3
directory; do not copy the checkout or build a new Runtime image.

```sh
TARGET_RUNTIME_IMAGE='<EXISTING_TARGET_IMAGE@sha256:...>'
REVIEWED_CHECKOUT='<REVIEWED_CHECKOUT>'
REVIEWED_COMMIT='<REVIEWED_COMMIT>'
MIGRATION_TOOL_ARCHIVE='<LOCAL_TOOL_ARCHIVE>'
MIGRATION_TOOL_DIR='<LAT3_TOOL_DIR>'
MIGRATION_TOOL_SHA256='<REVIEWED_TOOL_SHA256>'

git -C "$REVIEWED_CHECKOUT" archive --format=tar \
  "$REVIEWED_COMMIT" -- \
  scripts/legacy_hermes_migration.py \
  scripts/legacy_hermes_contract.py \
  scripts/legacy_hermes_source.py \
  scripts/legacy_hermes_target.py \
  scripts/legacy-hermes-source \
  >"$MIGRATION_TOOL_ARCHIVE"
test "$(sha256sum "$MIGRATION_TOOL_ARCHIVE" | awk '{print $1}')" \
  = "$MIGRATION_TOOL_SHA256"
ssh lat3 "sudo install -d -m 0700 '$MIGRATION_TOOL_DIR'"
ssh lat3 "sudo tee '$MIGRATION_TOOL_DIR/tool.tar' >/dev/null" \
  <"$MIGRATION_TOOL_ARCHIVE"
ssh lat3 "echo '$MIGRATION_TOOL_SHA256  $MIGRATION_TOOL_DIR/tool.tar' \
    | sudo sha256sum --check && \
  sudo tar -C '$MIGRATION_TOOL_DIR' --strip-components=1 -xpf \
    '$MIGRATION_TOOL_DIR/tool.tar' && \
  sudo chmod 0500 \
    '$MIGRATION_TOOL_DIR/legacy-hermes-source' \
    '$MIGRATION_TOOL_DIR/legacy_hermes_migration.py' \
    '$MIGRATION_TOOL_DIR/legacy_hermes_contract.py' \
    '$MIGRATION_TOOL_DIR/legacy_hermes_source.py' \
    '$MIGRATION_TOOL_DIR/legacy_hermes_target.py'"

sudo nerdctl --namespace finite run --rm --network none \
  --mount \
    'type=bind,src=<LAT3_TOOL_DIR>,dst=/opt/migration,ro' \
  --entrypoint /usr/local/bin/python3 \
  "$TARGET_RUNTIME_IMAGE" \
  /opt/migration/legacy_hermes_migration.py --help
```

### 2. Create and identify the source target

Create one normal Agent while signed in as `<SOURCE_OWNER_EMAIL>`. Confirm Core
placed it on `finite-lat-3`. Send one Finite Chat round trip and record the
target Agent `npub`. Do not take a live filesystem archive or treat a hash of a
running SQLite database as the rollback boundary; step 6 does that after the
typed stop.

### 3. Freeze the source bot on box1

Start the outage only after a go/no-go review of steps 1–2. Scale the exact
StatefulSet to zero through k3s. Immediately before scaling, re-prove the
running container image ID and the source storage contract. This values-free
check requires a read-only root filesystem, one PVC at `/home/node`, and only
ephemeral writable mounts at `/tmp` and `/run`:

```sh
ssh box1 "sudo k3s kubectl get pod \
  --namespace <SOURCE_NAMESPACE> <SOURCE_POD> -o json | jq -e '
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
        .persistentVolumeClaim.claimName == \"<SOURCE_PVC>\")] |
      length) == 1 and
    ([.spec.volumes[] |
      select((.name == \"tmp\" or .name == \"run\") and has(\"emptyDir\"))] |
      length) == 2
  ' >/dev/null"
test "$(ssh box1 sudo k3s kubectl get pod \
  --namespace <SOURCE_NAMESPACE> <SOURCE_POD> \
  -o jsonpath='{.status.containerStatuses[0].imageID}')" \
  = '<SOURCE_CONTAINER_IMAGE_ID>'
ssh box1 sudo k3s kubectl scale \
  --namespace <SOURCE_NAMESPACE> statefulset/<SOURCE_STATEFULSET> --replicas=0
ssh box1 sudo k3s kubectl wait \
  --namespace <SOURCE_NAMESPACE> --for=delete pod/<SOURCE_POD> --timeout=120s
ssh box1 sudo k3s kubectl get pvc \
  --namespace <SOURCE_NAMESPACE> <SOURCE_PVC> -o wide
```

Do not manually restart box1 after this point. It is the rollback copy and must
remain single-writer safe.

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
  sudo chmod 0500 \
    '<BOX1_STAGE>/tool/legacy-hermes-source' \
    '<BOX1_STAGE>/tool/legacy_hermes_migration.py' \
    '<BOX1_STAGE>/tool/legacy_hermes_contract.py' \
    '<BOX1_STAGE>/tool/legacy_hermes_source.py' \
    '<BOX1_STAGE>/tool/legacy_hermes_target.py'"
```

Immediately before export, require the local source tag to resolve to the
recorded manifest digest and the frozen pod's last container image ID to match
the evidence sheet. Prove no remaining host process has the source PVC open
for writing; retain the JSON result. The check must run as root on box1 and
fails closed if it cannot inspect the real process table:

```sh
ssh box1 "sudo python3 '<BOX1_STAGE>/tool/legacy_hermes_migration.py' \
  source-writer-check --source-root '<SOURCE_PV_PATH>'"
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

```sh
SOURCE_IMAGE_REFERENCE='docker.io/library/fc-agent-runtime:main'
SOURCE_IMAGE_MANIFEST_DIGEST='<SOURCE_IMAGE_MANIFEST_DIGEST>'

test "$(sudo ctr --namespace k8s.io images ls \
  "name==$SOURCE_IMAGE_REFERENCE" | awk 'NR == 2 { print $3 }')" \
  = "$SOURCE_IMAGE_MANIFEST_DIGEST"
sudo install -d -m 0700 '<BOX1_STAGE>'
sudo ctr --namespace k8s.io run --rm \
  --user 0:0 \
  --mount 'type=bind,src=<SOURCE_PV_PATH>,dst=/source,options=rbind:ro' \
  --mount 'type=bind,src=<BOX1_STAGE>,dst=/migration,options=rbind:rw' \
  --mount 'type=bind,src=<BOX1_STAGE>/tool,dst=/opt/migration,options=rbind:ro' \
  "$SOURCE_IMAGE_REFERENCE" 'legacy-hermes-volume-inventory' \
  /opt/migration/legacy-hermes-source source-volume-inventory \
  --source-root /source \
  --output /migration/source-volume-inventory.json
sudo chmod 0600 '<BOX1_STAGE>/source-volume-inventory.json'
sudo sha256sum '<BOX1_STAGE>/source-volume-inventory.json'

sudo sh -c 'umask 077; \
  /run/current-system/sw/bin/finited \
  --workspace-root /etc/nixos/workspaces/ovh-fc-1 \
  --control-plane-root /var/lib/finitecomputer/control-plane \
  list-published-endpoints \
  --payload '"'"'{"machineId":"<SOURCE_MACHINE_ID>"}'"'"' \
  > <BOX1_STAGE>/published-endpoints.json'

sudo ctr --namespace k8s.io run --rm \
  --user 0:0 \
  --mount 'type=bind,src=<SOURCE_PV_PATH>,dst=/source,options=rbind:ro' \
  --mount 'type=bind,src=<BOX1_STAGE>,dst=/migration,options=rbind:rw' \
  --mount 'type=bind,src=<BOX1_STAGE>/tool,dst=/opt/migration,options=rbind:ro' \
  "$SOURCE_IMAGE_REFERENCE" 'legacy-hermes-sites-inventory' \
  /opt/migration/legacy-hermes-source source-sites-inventory \
  --published-endpoints /migration/published-endpoints.json \
  --source-volume-inventory /migration/source-volume-inventory.json \
  --expected-machine-id <SOURCE_MACHINE_ID> \
  --output /migration/sites.json

sudo ctr --namespace k8s.io run --rm \
  --user 0:0 \
  --mount 'type=bind,src=<SOURCE_PV_PATH>,dst=/source,options=rbind:ro' \
  --mount 'type=bind,src=<BOX1_STAGE>,dst=/migration,options=rbind:rw' \
  --mount 'type=bind,src=<BOX1_STAGE>/tool,dst=/opt/migration,options=rbind:ro' \
  "$SOURCE_IMAGE_REFERENCE" 'legacy-hermes-integrations-inventory' \
  /opt/migration/legacy-hermes-source source-integrations-inventory \
  --source-root /source \
  --source-volume-inventory /migration/source-volume-inventory.json \
  --output /migration/integrations.json

sudo ctr --namespace k8s.io run --rm \
  --user 0:0 \
  --mount 'type=bind,src=<SOURCE_PV_PATH>,dst=/source,options=rbind:ro' \
  --mount 'type=bind,src=<BOX1_STAGE>,dst=/migration,options=rbind:rw' \
  --mount 'type=bind,src=<BOX1_STAGE>/tool,dst=/opt/migration,options=rbind:ro' \
  --env HERMES_HOME=/source/.hermes \
  "$SOURCE_IMAGE_REFERENCE" 'legacy-hermes-export' \
  /opt/migration/legacy-hermes-source source-export \
  --source-database /source/.hermes/state.db \
  --output /migration/sessions.jsonl

sudo ctr --namespace k8s.io run --rm \
  --user 0:0 \
  --mount 'type=bind,src=<SOURCE_PV_PATH>,dst=/source,options=rbind:ro' \
  --mount 'type=bind,src=<BOX1_STAGE>,dst=/migration,options=rbind:rw' \
  --mount 'type=bind,src=<BOX1_STAGE>/tool,dst=/opt/migration,options=rbind:ro' \
  --env HERMES_HOME=/source/.hermes \
  "$SOURCE_IMAGE_REFERENCE" 'legacy-hermes-memory-export' \
  /opt/migration/legacy-hermes-source source-memory-snapshot \
  --source-database /source/.hermes/memory_store.db \
  --output /migration/memory_store.db
```

The Sites command reads only the authoritative control-plane export and
source-volume inventory. Before continuing, compare its endpoint count with
the read-only rehearsal inventory. If rehearsal found one or more endpoints
but the frozen export returns zero, stop: a wrong control-plane root can
produce a valid-looking empty response. The command also fails if a locally
run Site points outside `/home/node` or its source is missing. The integrations
command reads configuration without executing it and emits names and policies
only. Inspect both root-only files; they must contain no credential values.

The session exporter first uses SQLite's backup API, then reads the scratch
database through Hermes v0.14. Structured memory normally uses the same backup
path. If its first 40 SQLite header bytes are damaged, the exporter may rebuild
a private copy only when one page layout exposes the expected facts table and
the remaining integrity result contains orphaned pages and nothing else. The
source stays unchanged, SQLite rewrites the recovered copy, and the final
snapshot must pass `quick_check`. Record the command's `recovery` field; any
other result is a hard stop. Retain the inventory hash and both export
commands' counts, byte counts, and SHA-256 output.

For a rehearsal against an off-host backup, keep the restored PVC unchanged
and point the two database export commands at the matching files below the
archive's `var/lib/finitecomputer/backups/sqlite-snapshots/<ARCHIVE>/` tree.
Require `MANIFEST.tsv` to map both exact source database paths, require neither
path in `SKIPPED.tsv`, and run the same exporters against those consistent
copies. During the real cutover, use the frozen PVC paths shown above because
the source pod is stopped and the database files are quiescent.

### 5. Stage the complete snapshot and active bundle

Run these commands from the trusted operator workstation. Fill only the two
target paths; the source values are deliberately concrete. The first
stream captures all of `/home/node` without following symlinks. The remaining
streams stage the subset that the importer converts or places into active and
review-only target paths.

```sh
set -euo pipefail

SOURCE_PV_PATH='<SOURCE_PV_PATH>'
BOX1_STAGE='<BOX1_STAGE>'
BUNDLE='<LAT3_BUNDLE>'
TRANSPORT_ARCHIVE='<LAT3_ROOT_ONLY_STAGE>/source-home.tar.zst'
SOURCE_VOLUME_INVENTORY_SHA256='<SOURCE_VOLUME_INVENTORY_SHA256>'

ssh lat3 "sudo install -d -m 0700 \
  '$BUNDLE/payload/hermes' '$BUNDLE/payload/home'"
ssh box1 "sudo tar --sort=name --numeric-owner --format=pax \
  --hard-dereference --one-file-system --acls --xattrs \
  -C '$SOURCE_PV_PATH' -cpf - . \
  | zstd -T0 -3 -c" \
  | ssh lat3 "sudo sh -c 'umask 077; cat >\"$TRANSPORT_ARCHIVE\"'"
ssh lat3 "sudo sha256sum '$TRANSPORT_ARCHIVE'"
ssh lat3 "sudo sh -c 'umask 077; zstd -T0 -d -c \
  \"$TRANSPORT_ARCHIVE\" \
  >\"$BUNDLE/payload/source-home.tar\"'"
HERMES_PAYLOAD_PATHS="$(ssh box1 "sudo bash -c '
  set -euo pipefail
  cd \"\$1\"
  for path in memories skills cron/jobs.json scripts; do
    [[ ! -e \"\$path\" && ! -L \"\$path\" ]] || printf \"%s\\n\" \"\$path\"
  done
' _ '$SOURCE_PV_PATH/.hermes'")"
if [[ -n "$HERMES_PAYLOAD_PATHS" ]]; then
  ssh box1 "sudo tar -C '$SOURCE_PV_PATH/.hermes' -cpf - \
    $HERMES_PAYLOAD_PATHS" \
    | ssh lat3 "sudo tar -C '$BUNDLE/payload/hermes' -xpf -"
fi
HOME_PAYLOAD_PATHS="$(ssh box1 "sudo bash -c '
  set -euo pipefail
  cd \"\$1\"
  for path in workspace dev uploads; do
    [[ ! -e \"\$path\" && ! -L \"\$path\" ]] || printf \"%s\\n\" \"\$path\"
  done
' _ '$SOURCE_PV_PATH'")"
if [[ -n "$HOME_PAYLOAD_PATHS" ]]; then
  ssh box1 "sudo tar -C '$SOURCE_PV_PATH' -cpf - \
    $HOME_PAYLOAD_PATHS" \
    | ssh lat3 "sudo tar -C '$BUNDLE/payload/home' -xpf -"
fi
ssh box1 "sudo tar -C '$BOX1_STAGE' -cpf - sessions.jsonl" \
  | ssh lat3 "sudo tar -C '$BUNDLE/payload' -xpf -"
ssh box1 "sudo tar -C '$BOX1_STAGE' -cpf - memory_store.db" \
  | ssh lat3 "sudo tar -C '$BUNDLE/payload' -xpf -"
ssh box1 "sudo tar -C '$BOX1_STAGE' -cpf - source-volume-inventory.json" \
  | ssh lat3 "sudo tar -C '$BUNDLE' -xpf -"
ssh box1 "sudo tar -C '$BOX1_STAGE' -cpf - sites.json integrations.json" \
  | ssh lat3 "sudo tar -C '$BUNDLE' -xpf -"
```

Record the compressed archive's SHA-256 before decompression and retain it
beside, not inside, the bundle until independent verification passes. The
bundle validator rejects unknown payload files, while the manifest binds the
uncompressed `source-home.tar`. Compress before transport; an uncompressed
whole-home transfer can turn a short cutover into a multi-hour outage.

Do not add an exclusion to the complete source snapshot. `--hard-dereference`
stores hard-linked file content independently; it does not follow symlinks.
Never add `--dereference`. If the inventory or manifest finds a special file,
unreadable entry, an escaping link outside known generated or quarantined
state, a missing path, or a digest mismatch, stop and fix the fleet policy or
source condition in review. Credentials, old identity,
cron output, raw databases, venvs, logs, and caches are present only inside the
root-only `source-home.tar`; they never receive active target mappings.

Seal and verify the bundle with the reviewed tool mounted read-only into the
existing target image:

```sh
sudo nerdctl --namespace finite run --rm --network none \
  --mount 'type=bind,src=<BUNDLE>,dst=/migration' \
  --mount \
    'type=bind,src=<LAT3_TOOL_DIR>,dst=/opt/migration,ro' \
  --entrypoint /usr/local/bin/python3 \
  "$TARGET_RUNTIME_IMAGE" \
  /opt/migration/legacy_hermes_migration.py manifest \
  --bundle /migration \
  --source-host-id box1 \
  --source-machine-id <SOURCE_MACHINE_ID> \
  --source-owner-email <SOURCE_OWNER_EMAIL> \
  --source-hermes-version 0.14.0 \
  --source-image-reference 'docker.io/library/fc-agent-runtime:main' \
  --source-image-manifest-digest \
    '<SOURCE_IMAGE_MANIFEST_DIGEST>' \
  --source-container-image-id \
    '<SOURCE_CONTAINER_IMAGE_ID>' \
  --source-volume-inventory-sha256 \
    "$SOURCE_VOLUME_INVENTORY_SHA256"

sudo nerdctl --namespace finite run --rm --network none --read-only \
  --mount 'type=bind,src=<BUNDLE>,dst=/migration,ro' \
  --mount \
    'type=bind,src=<LAT3_TOOL_DIR>,dst=/opt/migration,ro' \
  --entrypoint /usr/local/bin/python3 \
  "$TARGET_RUNTIME_IMAGE" \
  /opt/migration/legacy_hermes_migration.py verify --bundle /migration

sudo sha256sum '<BUNDLE>/manifest.json'
```

Record `MANIFEST_SHA256`, bundle size, and the `source_snapshot` summary.
Compare the manifest session, message, and memory-fact counts with step 4.
The manifest must prove that every inventory entry exists in `source-home.tar`
with matching type, mode, size, symlink target, and content hash. It must have
zero structurally blocked entries. It must also say legacy skills are
review-only, the Brain working tree must be freshly authorized and synced,
and how many session paths were rewritten, preserved as cache media, or remain
unmapped in active state.
It must also bind the complete Sites and integrations inventories. Require all
local Site source paths present, no automatic republication, every integration
assigned a policy, no secret values, and every integration inactive.

### 6. Stop the target through Core

As the verified owner, submit the typed stop request for the exact `PROJECT_ID`:

```text
POST /api/core/v1/me/projects/<PROJECT_ID>/runtime/stop
```

Wait for the request to succeed. Require Core to report the target offline,
the lat3 container/task absent or stopped as defined by the Runner, and the
durable root still present. Do not substitute `nerdctl stop`.

Now capture the quiescent protected hashes:

```sh
sudo sha256sum \
  '<TARGET_DATA>/agent/identity/identity.json' \
  '<TARGET_DATA>/agent/client.sqlite3'
```

Record `TARGET_IDENTITY_SHA256` and `TARGET_CHAT_CLIENT_SHA256` separately.
Take a scoped pre-import archive of the exact fresh `<TARGET_DATA>` and verify
that archive can restore to an empty scratch directory. The importer will also
prove that the fresh structured-memory store has zero facts before replacing
it.

### 7. Install offline into the exact target

Run one networkless importer against the stopped target. The bundle is
read-only; only the exact target `/data` and a private, bounded `/tmp` tmpfs
are writable. The tmpfs is required because SQLite can spill temporary work
during a large session import; without it, the read-only container root causes
a late `disk I/O error` even though `/data` remains healthy:

```sh
sudo nerdctl --namespace finite run --rm --network none --read-only \
  --tmpfs '/tmp:rw,nosuid,nodev,noexec,size=4294967296' \
  --mount 'type=bind,src=<BUNDLE>,dst=/migration,ro' \
  --mount 'type=bind,src=<TARGET_DATA>,dst=/data' \
  --mount \
    'type=bind,src=<LAT3_TOOL_DIR>,dst=/opt/migration,ro' \
  --entrypoint /usr/local/bin/python3 \
  "$TARGET_RUNTIME_IMAGE" \
  /opt/migration/legacy_hermes_migration.py install \
  --bundle /migration \
  --target-root /data \
  --expected-source-machine-id <SOURCE_MACHINE_ID> \
  --expected-manifest-sha256 '<MANIFEST_SHA256>' \
  --expected-target-identity-sha256 '<TARGET_IDENTITY_SHA256>' \
  --expected-target-chat-client-sha256 '<TARGET_CHAT_CLIENT_SHA256>'
```

Require a receipt at
`<TARGET_DATA>/migration/legacy-hermes-v2/receipt.json`. Its status must be
`installed-offline-awaiting-runtime-verification`; its manifest hash and
source machine must match approval; its protected hashes must match step 6.

### 8. Restart and verify

As the verified owner, submit the typed restart request:

```text
POST /api/core/v1/me/projects/<PROJECT_ID>/runtime/restart
```

Submit it once. Record the control-request ID and wait for Core and Runner to
reach a terminal result. A stale dashboard button is not evidence that the
request failed. Do not submit another restart while the first request is
pending or while the Runtime is still converging.

Require every verification check below. Keep box1 at zero replicas and keep both
recovery archives through a minimum 24-hour observation window.

## Verify

- Core still shows the exact target Project, Runtime, artifact, schema, host,
  and Agent `npub` recorded in step 2.
- The target identity and Chat client SHA-256 values are unchanged.
- The receipt session/message counts equal the sealed manifest.
- The receipt Sites summary equals the manifest. Every legacy endpoint is
  listed, every local source path is preserved, and no Site was republished by
  the importer.
- The receipt integrations summary equals the manifest. Telegram and Signal
  remain transfer candidates, Google Workspace and Brain require fresh
  authorization, target-managed model credentials are not copied, and every
  other detected connection remains preserved but disabled.
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
- the source bot's imported memories are visible. Legacy skills exist only under
  `/data/migration/legacy-hermes-v2/review-only/skills`; none shadow or merge
  into the active managed skill tree.
- The receipt and rebuilt memory store contain exactly the fact count sealed
  in the frozen manifest.
- The receipt records exactly the source cron-job count sealed in the frozen
  manifest, and no job exists in the active
  target cron path; the review copy is under the migration receipt directory.
- Files exist under `/data/workspace/legacy-box1/{workspace,dev,uploads}` and a
  sample of manifested hashes and contained symlinks matches.
- The manifest and receipt contain no write target for the target's managed
  Hermes configuration, active skills, identity, or Chat state. Protected
  identity and Chat hashes still match step 6.
- Brain access is recorded as pending fresh target authorization and sync. The
  source Brain working tree and identity exist only inside `source-home.tar`.
- A new Finite Chat message receives exactly one target reply.
- The owner can open the migrated Project with the authoritative SaaS login.
  Any separate Sites mailbox sees only its intended Sites access and must not
  create a second Agent.
- Acceptance covers fresh Chat plus Telegram text, voice, and image handling
  when those channels are in scope. Record the owner's commentary preference
  and voice-transcript echo preference before calling behavior equivalent.
- box1 has zero source-bot pods; its PVC and off-host archive remain intact.
- No other box1 or lat3 bot restarted or changed artifact.
- `scripts/finite-status --json` passes the same status gate used before the
  migration. Every carried exception is unchanged, and no new or worsened
  result appears.

Use a fresh Chat for the acceptance message. If a pre-cutover turn resumes
after restart, use Hermes `/stop` before sending the probe. A large interrupted
session can take minutes while a fresh Chat replies in seconds, so stale-session
latency is not a useful availability measurement.

Runtime health and channel health are separate gates. Re-anchor the observation
window to the last successful post-restart proof across every restored channel.
Signal may need one watchdog interval plus Hermes reconnect backoff after a
Runtime restart. Measure and record that interval before starting observation.

Do not use `nerdctl exec`, `ctr task exec`, or another live in-VM shell for
post-cutover verification, even for a small read-only status command. Canary
exercises reproduced stale Kata task state with both a heavy verifier and a
small stdin-based configuration probe. The networkless rehearsal already opened the
imported databases through Hermes v0.20, and the offline receipt binds the
frozen counts and protected hashes. Use external Core, Runner, containerd,
health, and Chat evidence while the Runtime is live. If durable-file inspection
is required, stop the Runtime through Core first and inspect through a separate
networkless scratch container.

When host evidence is necessary, request only the fields under review. For
example, use `nerdctl inspect --format` for the artifact label and `/data`
mount. Never retain or paste raw `nerdctl inspect` output; it includes the
Runtime's injected environment.

If Core reports a failed restart while the host shows `Unknown`, stop. Do not
click Restart again and do not restart containerd. Record the exact control
request and collect source-bot-only task, shim, VM, mount, and open-file evidence.
Per-container cleanup is a break-glass repair with its own evidence and
approval, not a normal migration step.

Do not remove the canonical Runtime container, even when its task is stale.
Current restart, known-good Chat recovery, and upgrade operations all require
that owned container to exist. None can recreate it from an intact durable
root. If break-glass cleanup has already removed the canonical container, leave
the durable root untouched and roll back to box1. Do not reconstruct the
container by hand or treat Core's recorded `Online` state as live-compute proof.

A later retry may use the reviewed absent-compute variant in
`runtime-cold-relocation.md`: preserve a scoped archive, prove container and
task absence, recover the exact durable tree to a healthy stopped Runtime,
then perform a normal cold relocation. That is a new Core-recorded recovery
transaction, not an extension of the failed restart.

Complete the receipt status in the retained operator evidence, not by editing
the receipt inside `/data`. Record IDs, digests, hashes, counts, timestamps,
and outcomes; record no token, key, message content, or secret value.

## Restore executable behavior

The data import and behavior restoration are separate approval phases. That
separation protects target identity and external ownership; it does not require
an owner to decide the fate of every file, skill, or job.

Build one hash-bound behavior bundle from the frozen inventory and source
`jobs.json`, not through a live Agent turn. Test it
networkless against a restored target archive in the exact production image.
The converter must apply these rules automatically:

- a target-managed skill wins over an equivalent legacy skill;
- a compatible user-authored skill absent from the target installs in one
  reviewed batch;
- obsolete, generated, identity-bearing, or unsafe executable state stays
  inert in the complete Recovery Set;
- every source job is transformed into the current schema and created paused;
- the manifest binds every staged file and the complete job plan, including
  schedule, prompt, origin, working directory, and paused state;
- native clients and helpers retain explicit executable modes;
- copied helpers use `/data` paths, an explicit migrated home, and account
  identity derived from the staged target-compatible store, never a copied
  source literal.

For each skill selected for promotion, test its target tools, absolute paths,
configuration keys, and credential locations in the exact target image. An
optional dependency must fail cleanly. A skill must never compensate for a
missing credential by searching unrelated databases, session state, or secret
locations. If a live turn reveals a stale assumption, quarantine the complete
skill atomically outside the active tree, create and verify a root-only rollback
archive, prove the active skill scan no longer finds it, and verify that managed
configuration hashes did not change. Do not restart the Runtime when Hermes's
skill discovery correctly invalidates from the active-tree signature.

When creating a transport archive on macOS, set `COPYFILE_DISABLE=1`; unsigned
AppleDouble files must fail verification. Run the target image's current cron
guard against the converted jobs. Rewrite old absolute work paths to admitted
`/data` workdirs and use relative references inside job prompts where the guard
would otherwise interpret examples as executable paths.

Restore external ownership only through supported target flows:

- grant and sync FiniteBrain by full Brain ID; do not select a checkout by a
  short name or copy the source Brain identity into active state;
- pair Telegram through the target flow, submit each pairing code once, and
  wait for refreshed connection state before retrying. A timed-out request may
  still complete and a retry may create a second pending request. Select the
  approved chat as the home chat and leave duplicate requests unapproved;
- obtain fresh Google Workspace OAuth rather than copying legacy OAuth state;
- preserve compatible Signal state, but move its daemon off Runtime-reserved
  port 8080, use an available non-reserved port, and verify that its watchdog
  recurs forever rather than completing as a one-shot.

Republish Sites as a separate Project-first phase after the Hermes data canary:

- preserve every legacy endpoint and readable source automatically, then
  publish only endpoints proven live and target-compatible at the frozen
  boundary; stopped, broken, and source-less endpoints remain inactive
  evidence without owner-by-owner file classification;
- never serve a broad legacy workspace when a dedicated output directory can
  express the Site. Static outputs must contain the exact served bytes, and app
  outputs must use the target-provided `PORT` and place all mutable state under
  `DATA_DIR`;
- run the static tree checks and every app's read/write smoke networkless in the
  exact target image before any Project is created;
- add the target's shared Finite identity to the owner's Sites mailbox keyset
  through `fsite auth sites-key request` and `fsite auth sites-key add`. Agent
  launch, Chat identity, and channel pairing do not substitute for this
  product-scoped mailbox proof;
- when the publishing tool uses a stopped target identity, mount only
  `identity.json` read-only inside a writable scratch Finite Home. `fsite`
  creates an adjacent `.lock`, so mounting the entire identity directory
  read-only fails before authentication;
- run `fsite project init --dry-run` for the complete candidate set before
  applying any Project. Then initialize, obtain a scoped Git credential with
  `--store`, commit only `finite.toml` and the selected output tree, push the
  deploy branch, and reproduce the frozen owner/email access map with explicit
  Shares;
- when packaging Project sources on macOS, set `COPYFILE_DISABLE=1` so the
  transport archive contains no AppleDouble or provenance-only metadata; and
- require every push to return an active Version, then verify the exact Sites
  URLs and access policy externally. A partially accepted Git ref is repaired
  with a new correcting commit, never by replaying the same push blindly.

After the behavior bundle passes against the restored target, stop the real
target through Core, prove no writer, take and restore a complete pre-behavior
archive, then install the same bundle networkless. Restart once through Core
and wait for the control request. Verify Web Chat and each restored external
channel before enabling jobs.

Enable source-equivalent jobs in dependency order through Hermes's supported
scheduler. Compare behavior with the frozen source outcome rather than an
idealized dependency list. A missing optional credential is not a blocker when
the source also skipped that integration and continued useful work. Require no
duplicate, past-due, one-shot-converted-from-recurring, or early manual runs.

Follow [the post-cutover repair brief](../../finitecomputer-v2/docs/legacy-hermes-post-cutover-repair.md)
for stale-path repair and the retained evidence checklist. Do not decommission
box1 until every required skill, job, Brain workspace, and external behavior
has a recorded target outcome.

## Rollback

Before target restart, leave box1 stopped. If install fails, retain the failed
bundle and receipt work directory for diagnosis, restore the scoped fresh
target archive to an empty `<TARGET_DATA>`, and re-check its original identity
and Chat hashes. Never repair a partial target by hand.

After target restart, stop the target through Core before any rollback. Restore
the fresh target archive if the v2 target itself must be reset. To resume
box1, first prove target compute is stopped, then scale only
`statefulset/<SOURCE_STATEFULSET>` back to one and verify the original bot.
Never run both writers.

Rollback verification is behavior-level, not just pod readiness. Require the
source pod to be Ready with no unexpected restarts, then prove the original
Chat or messaging route, credentials needed by active jobs, scheduled-job
state, Brain sync, and source-hosted Sites. Run `fbrain repair` before sync if
the restored Working Tree fails its permission boundary. Record any external
or source-less Site failure separately instead of attributing it to the source
Runtime. Finish with `scripts/finite-status --json` and keep the offline target
durable root and pre-import archive intact.

Do not delete the target, source PVC, staging bundle, or either recovery
archive during rollback. Decommission and credential reauthorization are later
changes with separate approval.

After migration acceptance, the non-destructive box1 disable boundary is the
source StatefulSet at zero replicas, no source pod, and the PVC still bound.
The legacy control plane has no separate disabled flag. Do not substitute
`finitec machine deprovision`: it deletes machine control-plane state and
secrets and may drive resource removal. Deprovisioning, secret removal, and PVC
deletion require a later destructive approval and a separate retention plan.
