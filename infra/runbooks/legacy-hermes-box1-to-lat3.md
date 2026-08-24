# Migrate one box1 Hermes bot to lat3

This procedure creates a normal v2 Agent on lat3, imports one allow-listed
legacy bundle while both sides are single-writer safe, and leaves box1 frozen
for rollback. It never converts a box1 identity into a v2 identity.

The first canary is Austin. Do not substitute another bot into the Austin
commands. Repeat the generic procedure later with a new evidence sheet and
approval.

TODO(first production exercise): retain Austin's actual export, transfer,
import, and verification durations plus session/message counts. Remove this
note only after the 24-hour observation window closes successfully.

## Austin evidence sheet

Fresh read-only inventory on 2026-08-22 established:

| Field | Approved source value |
| --- | --- |
| source host | `box1` |
| machine / namespace / owner | `austin-finite` |
| owner email | `austin@finite.vip` |
| StatefulSet pod | `austin-finite-0` |
| PVC | `home-austin-finite-0` |
| PV | `pvc-96716337-df1e-4b28-9692-0263d4672085` |
| PV path | `/var/lib/rancher/k3s/storage/pvc-96716337-df1e-4b28-9692-0263d4672085_austin-finite_home-austin-finite-0` |
| source Hermes | `0.14.0` |
| source image reference | `docker.io/library/fc-agent-runtime:main` |
| source manifest digest | `sha256:d6e7b42a8044fbfee94edbce0884a3900678c580a23ea792f2d8aa8c2a5276f5` |
| source container image ID | `sha256:6f2efdb34f4ea2cccbbe50e5dec5c49f11b766970a693f99bb7bf0cf02dd90db` |
| target Hermes | `0.20.0` |
| durable source mount | the `home-austin-finite-0` PVC mounted at `/home/node` |
| non-PVC runtime storage | read-only root filesystem; `/tmp` and `/run` are ephemeral `emptyDir` mounts |
| admitted file trees before session export | 67,003 regular files / 58 symlinks / about 5.9 GB in the first inventory; Austin later observed 67,001 / 57 while live, so the frozen manifest is authoritative |
| whole-volume inventory | TODO(real-data rehearsal): record the root-only inventory hash, every disposition, and zero unresolved entries |
| legacy session database input | about 3.0 GB and 2,761 sessions in Austin's live self-audit; JSONL size and frozen counts are learned during export |
| structured memory | 80 facts in a 648 KiB SQLite file |
| cron | 12 enabled definitions; preserve review-only and activate none during canary |

Re-read every value before mutation. A name match is not authority. Stop if
the PVC UID/path, image digest, owner, or Hermes version changed.

The first Austin inventory found 61 workspace symlinks. Three were inside the
archived `dev/reap-video/venv`; that 13 MB venv is explicitly excluded and can
be rebuilt. The other 58 stayed within their admitted workspace/dev/upload
root. The later live count drifted by one, so the frozen scan is authoritative.
A new escape is a hard stop, not an instruction to add `--dereference`.

The old Runtime could put durable data anywhere below `/home/node`. The three
admitted workspace roots are not proof that nothing else matters. Rehearsal
must scan the whole PVC and account for every file before this evidence sheet
is complete. The pod's root filesystem is declared read-only, so files outside
`/home/node` can only come from the immutable image, read-only mounts, or the
ephemeral `/tmp` and `/run` mounts. Step 3 rechecks that live storage shape. A
new writable mount is a hard stop.

The 2026-08-22 lat3 readiness snapshot showed 30 running sandboxes against the
declared limit of 32 and 1.6 TiB free on `/data`. These values are not a
reservation; re-check them immediately before target creation.

## Preconditions

- The reviewed commit passed the migration unit tests and the real
  v0.14-export-to-v0.20-import compatibility test.
- Before capturing the rehearsal Recovery Set, the values-free storage check
  in step 3 passed against the live source pod. It proved a read-only root,
  the Austin PVC at `/home/node`, and no other writable durable mount. Repeat
  the check immediately before cutover.
- The intended target uses an already published, digest-pinned canonical
  Runtime image whose durable smoke proves Hermes v0.20. This migration does
  not require publishing or rolling a new image.
- `scripts/finite-status --json` is retained and green. If the installed
  shortcut is absent, stage `scripts/finite-status` and
  `scripts/finite_status.py` together as described in
  [the runbook index](README.md#standing-rules); this is the canonical status
  check, not a product called “Finite Status.”
- A fresh box1 off-host backup completed, and the Recovery Set can restore to
  an empty scratch target. Record the archive name without recording keys.
- The exact reviewed tools completed a real-data rehearsal against an isolated
  restore of Austin's Recovery Set. Record its manifest hash, counts, duration,
  source-volume inventory hash, path dispositions, media-path result, and
  cleanup outcome. The inventory must cover the entire restored `/home/node`
  tree and report zero unresolved entries. A synthetic test is not this gate.
- The owner has explicitly authorized target creation and the later cutover
  outage. These are separate from authorization to decommission box1.
- Austin is the owner-approved first canary for this migration path. The
  reviewed PR and this runbook define the bounded procedure; they do not grant
  execution authority. Code review, target creation, source freeze, import,
  behavior restoration, and decommission remain separate approvals.
- lat3 has one free 4-vCPU/8-GiB Runtime slot and free disk of at least three
  times the sealed bundle size plus 10 GiB.
- The target is a fresh Runtime under the exact verified Austin account. Record
  `PROJECT_ID`, `RUNTIME_ID`, `MACHINE_ID`, `DURABLE_STATE_ID`, artifact,
  schema, host, `/data` path, and target Agent `npub`. Its structured-memory
  store must contain zero facts; the importer refuses a non-empty store.
- Known box1 credential stores and active gateway state will not be copied.
  Treat admitted user-authored files and scripts as potentially sensitive.
  Cron definitions, helper scripts, and legacy skills will be retained in
  review-only target paths; none will be activated by this canary.
- Hermes cache-only audio and image media stays in the frozen Recovery Set.
  Identify one old conversation containing media for step 8, or explicitly
  record that the frozen export contains none. Paths into admitted uploads,
  workspace, and dev trees are rewritten; cache-only paths are not.
- The legacy local FiniteBrain working tree and its identity are not admitted.
  Plan a fresh Agent Principal delegation, Folder Key Grant, and sync using
  [the post-cutover repair brief](../../finitecomputer-v2/docs/legacy-hermes-post-cutover-repair.md).

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
  sudo chmod 0500 '$MIGRATION_TOOL_DIR'/legacy-hermes-source \
    '$MIGRATION_TOOL_DIR'/legacy_hermes_*.py"

sudo nerdctl --namespace finite run --rm --network none \
  --mount \
    'type=bind,src=<LAT3_TOOL_DIR>,dst=/opt/migration,options=rbind:ro' \
  --entrypoint /usr/local/bin/python3 \
  "$TARGET_RUNTIME_IMAGE" \
  /opt/migration/legacy_hermes_migration.py --help
```

### 2. Create and identify the Austin target

Create one normal Agent while signed in as `austin@finite.vip`. Confirm Core
placed it on `finite-lat-3`. Send one Finite Chat round trip and record the
target Agent `npub`. Do not take a live filesystem archive or treat a hash of a
running SQLite database as the rollback boundary; step 6 does that after the
typed stop.

### 3. Freeze Austin on box1

Start the outage only after a go/no-go review of steps 1–2. Scale the exact
StatefulSet to zero through k3s. Immediately before scaling, re-prove the
running container image ID and the source storage contract. This values-free
check requires a read-only root filesystem, one PVC at `/home/node`, and only
ephemeral writable mounts at `/tmp` and `/run`:

```sh
ssh box1 "sudo k3s kubectl get pod \
  --namespace austin-finite austin-finite-0 -o json | jq -e '
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
        .persistentVolumeClaim.claimName == \"home-austin-finite-0\")] |
      length) == 1 and
    ([.spec.volumes[] |
      select((.name == \"tmp\" or .name == \"run\") and has(\"emptyDir\"))] |
      length) == 2
  ' >/dev/null"
test "$(ssh box1 sudo k3s kubectl get pod \
  --namespace austin-finite austin-finite-0 \
  -o jsonpath='{.status.containerStatuses[0].imageID}')" \
  = 'sha256:6f2efdb34f4ea2cccbbe50e5dec5c49f11b766970a693f99bb7bf0cf02dd90db'
ssh box1 sudo k3s kubectl scale \
  --namespace austin-finite statefulset/austin-finite --replicas=0
ssh box1 sudo k3s kubectl wait \
  --namespace austin-finite --for=delete pod/austin-finite-0 --timeout=120s
ssh box1 sudo k3s kubectl get pvc \
  --namespace austin-finite home-austin-finite-0 -o wide
```

Do not manually restart box1 after this point. It is the rollback copy and must
remain single-writer safe.

### 4. Inventory and export the frozen PVC

Create a root-only box1 staging directory outside the PVC. Transfer the exact
tool archive approved in step 1; do not copy a checkout or secret file. Verify
its hash before extracting. The single source launcher accepts only the volume
inventory and two export commands. It reuses the source image's immutable
Hermes wrapper environment instead of the mutable user venv.

```sh
ssh box1 "sudo install -d -m 0700 '<BOX1_STAGE>'"
ssh box1 "sudo tee '<BOX1_STAGE>/tool.tar' >/dev/null" \
  <"$MIGRATION_TOOL_ARCHIVE"
ssh box1 "echo '$MIGRATION_TOOL_SHA256  <BOX1_STAGE>/tool.tar' \
    | sudo sha256sum --check && \
  sudo install -d -m 0700 '<BOX1_STAGE>/tool' && \
  sudo tar -C '<BOX1_STAGE>/tool' --strip-components=1 -xpf \
    '<BOX1_STAGE>/tool.tar' && \
  sudo chmod 0500 '<BOX1_STAGE>/tool'/legacy-hermes-source \
    '<BOX1_STAGE>/tool'/legacy_hermes_*.py"
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

Inventory every file on the PVC before selecting bundle inputs. The command
writes no file contents. It exits nonzero after writing the root-only evidence
file if any path lacks a reviewed disposition. During rehearsal, an unresolved
path means the policy, tests, and this evidence sheet must return to review.
During cutover, any unresolved path is a hard stop.

```sh
SOURCE_IMAGE_REFERENCE='docker.io/library/fc-agent-runtime:main'
SOURCE_IMAGE_MANIFEST_DIGEST='sha256:d6e7b42a8044fbfee94edbce0884a3900678c580a23ea792f2d8aa8c2a5276f5'

test "$(sudo ctr --namespace k8s.io images ls \
  "name==$SOURCE_IMAGE_REFERENCE" | awk 'NR == 2 { print $3 }')" \
  = "$SOURCE_IMAGE_MANIFEST_DIGEST"
sudo install -d -m 0700 '<BOX1_STAGE>'
sudo ctr --namespace k8s.io run --rm \
  --user 0:0 \
  --mount 'type=bind,src=<SOURCE_PV_PATH>,dst=/source,options=rbind:ro' \
  --mount 'type=bind,src=<BOX1_STAGE>,dst=/migration,options=rbind:rw' \
  --mount 'type=bind,src=<BOX1_STAGE>/tool,dst=/opt/migration,options=rbind:ro' \
  "$SOURCE_IMAGE_REFERENCE" 'austin-hermes-volume-inventory' \
  /opt/migration/legacy-hermes-source source-volume-inventory \
  --source-root /source \
  --output /migration/source-volume-inventory.json
sudo chmod 0600 '<BOX1_STAGE>/source-volume-inventory.json'
sudo sha256sum '<BOX1_STAGE>/source-volume-inventory.json'

sudo ctr --namespace k8s.io run --rm \
  --user 0:0 \
  --mount 'type=bind,src=<SOURCE_PV_PATH>,dst=/source,options=rbind:ro' \
  --mount 'type=bind,src=<BOX1_STAGE>,dst=/migration,options=rbind:rw' \
  --mount 'type=bind,src=<BOX1_STAGE>/tool,dst=/opt/migration,options=rbind:ro' \
  --env HERMES_HOME=/source/.hermes \
  "$SOURCE_IMAGE_REFERENCE" 'austin-hermes-export' \
  /opt/migration/legacy-hermes-source source-export \
  --source-database /source/.hermes/state.db \
  --output /migration/sessions.jsonl

sudo ctr --namespace k8s.io run --rm \
  --user 0:0 \
  --mount 'type=bind,src=<SOURCE_PV_PATH>,dst=/source,options=rbind:ro' \
  --mount 'type=bind,src=<BOX1_STAGE>,dst=/migration,options=rbind:rw' \
  --mount 'type=bind,src=<BOX1_STAGE>/tool,dst=/opt/migration,options=rbind:ro' \
  --env HERMES_HOME=/source/.hermes \
  "$SOURCE_IMAGE_REFERENCE" 'austin-hermes-memory-export' \
  /opt/migration/legacy-hermes-source source-memory-snapshot \
  --source-database /source/.hermes/memory_store.db \
  --output /migration/memory_store.db
```

The exporter first uses SQLite's backup API, then reads the scratch database
through Hermes v0.14. The memory snapshot also uses SQLite's backup API and
must report 80 facts. Retain the inventory hash and both export commands'
counts, byte counts, and SHA-256 output.

### 5. Stage only the admitted bundle

Run these commands from the trusted operator workstation. Fill only the two
target paths; the Austin source values are deliberately concrete. The streams
preserve modes and symlinks without following them, and the sole exclusion is
the archived venv identified above.

```sh
SOURCE_PV_PATH='/var/lib/rancher/k3s/storage/pvc-96716337-df1e-4b28-9692-0263d4672085_austin-finite_home-austin-finite-0'
BOX1_STAGE='<BOX1_STAGE>'
BUNDLE='<LAT3_BUNDLE>'
SOURCE_VOLUME_INVENTORY_SHA256='<SOURCE_VOLUME_INVENTORY_SHA256>'

ssh lat3 "sudo install -d -m 0700 \
  '$BUNDLE/payload/hermes' '$BUNDLE/payload/home'"
ssh box1 "sudo tar -C '$SOURCE_PV_PATH/.hermes' -cpf - \
  memories skills cron/jobs.json scripts" \
  | ssh lat3 "sudo tar -C '$BUNDLE/payload/hermes' -xpf -"
ssh box1 "sudo tar -C '$SOURCE_PV_PATH' \
  --exclude='dev/reap-video/venv' -cpf - workspace dev uploads" \
  | ssh lat3 "sudo tar -C '$BUNDLE/payload/home' -xpf -"
ssh box1 "sudo tar -C '$BOX1_STAGE' -cpf - sessions.jsonl" \
  | ssh lat3 "sudo tar -C '$BUNDLE/payload' -xpf -"
ssh box1 "sudo tar -C '$BOX1_STAGE' -cpf - memory_store.db" \
  | ssh lat3 "sudo tar -C '$BUNDLE/payload' -xpf -"
ssh box1 "sudo tar -C '$BOX1_STAGE' -cpf - source-volume-inventory.json" \
  | ssh lat3 "sudo tar -C '$BUNDLE' -xpf -"
```

Do not add another exclusion during cutover. If the manifest rejects a new
special file or escaping symlink, stop and amend the evidence sheet under
review. Never add `--dereference`. Do not stage top-level `.env`, `auth.json`,
config, tokens, cron output, `.finite`, raw session/auxiliary SQLite,
Hermes-managed venvs, logs, or caches. They remain inside the frozen Recovery
Set; user project dependencies, cron definitions, the structured-memory
snapshot, and files inside the admitted roots remain sensitive bundle data and
stay root-only.

Seal and verify the bundle with the reviewed tool mounted read-only into the
existing target image:

```sh
sudo nerdctl --namespace finite run --rm --network none \
  --mount 'type=bind,src=<BUNDLE>,dst=/migration,options=rbind:rw' \
  --mount \
    'type=bind,src=<LAT3_TOOL_DIR>,dst=/opt/migration,options=rbind:ro' \
  --entrypoint /usr/local/bin/python3 \
  "$TARGET_RUNTIME_IMAGE" \
  /opt/migration/legacy_hermes_migration.py manifest \
  --bundle /migration \
  --source-host-id box1 \
  --source-machine-id austin-finite \
  --source-owner-email austin@finite.vip \
  --source-hermes-version 0.14.0 \
  --source-image-reference 'docker.io/library/fc-agent-runtime:main' \
  --source-image-manifest-digest \
    'sha256:d6e7b42a8044fbfee94edbce0884a3900678c580a23ea792f2d8aa8c2a5276f5' \
  --source-container-image-id \
    'sha256:6f2efdb34f4ea2cccbbe50e5dec5c49f11b766970a693f99bb7bf0cf02dd90db' \
  --source-volume-inventory-sha256 \
    "$SOURCE_VOLUME_INVENTORY_SHA256"

sudo nerdctl --namespace finite run --rm --network none --read-only \
  --mount 'type=bind,src=<BUNDLE>,dst=/migration,options=rbind:ro' \
  --mount \
    'type=bind,src=<LAT3_TOOL_DIR>,dst=/opt/migration,options=rbind:ro' \
  --entrypoint /usr/local/bin/python3 \
  "$TARGET_RUNTIME_IMAGE" \
  /opt/migration/legacy_hermes_migration.py verify --bundle /migration

sudo sha256sum '<BUNDLE>/manifest.json'
```

Record `MANIFEST_SHA256` and bundle size. Compare the manifest session/message
counts and 80 memory facts with step 4. Review every `archived_only` class and
the compatibility summary. The manifest's source-inventory hash must match the
reviewed step 4 evidence, and the inventory must have zero unresolved entries.
It must also say legacy skills are review-only, the Brain working tree must be
freshly authorized and synced, and how many session paths were rewritten, how
many cache-media paths remain archive-only, and how many other legacy paths
remain unmapped.

### 6. Stop the target through Core

As Austin, submit the typed stop request for the exact `PROJECT_ID`:

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
read-only; only the exact target `/data` is writable:

```sh
sudo nerdctl --namespace finite run --rm --network none --read-only \
  --mount 'type=bind,src=<BUNDLE>,dst=/migration,options=rbind:ro' \
  --mount 'type=bind,src=<TARGET_DATA>,dst=/data,options=rbind:rw' \
  --mount \
    'type=bind,src=<LAT3_TOOL_DIR>,dst=/opt/migration,options=rbind:ro' \
  --entrypoint /usr/local/bin/python3 \
  "$TARGET_RUNTIME_IMAGE" \
  /opt/migration/legacy_hermes_migration.py install \
  --bundle /migration \
  --target-root /data \
  --expected-source-machine-id austin-finite \
  --expected-manifest-sha256 '<MANIFEST_SHA256>' \
  --expected-target-identity-sha256 '<TARGET_IDENTITY_SHA256>' \
  --expected-target-chat-client-sha256 '<TARGET_CHAT_CLIENT_SHA256>'
```

Require a receipt at
`<TARGET_DATA>/migration/legacy-hermes-v1/receipt.json`. Its status must be
`installed-offline-awaiting-runtime-verification`; its manifest hash and
source machine must match approval; its protected hashes must match step 6.

### 8. Restart and verify

As Austin, submit the typed restart request:

```text
POST /api/core/v1/me/projects/<PROJECT_ID>/runtime/restart
```

Require every verification check below. Keep box1 at zero replicas and keep both
recovery archives through a minimum 24-hour observation window.

## Verify

- Core still shows the exact target Project, Runtime, artifact, schema, host,
  and Agent `npub` recorded in step 2.
- The target identity and Chat client SHA-256 values are unchanged.
- The receipt session/message counts equal the sealed manifest.
- The manifest binds the reviewed whole-volume inventory, and that inventory
  accounts for every non-directory entry on the source PVC with zero
  unresolved entries.
- A sampled old Hermes conversation is present but does not own a Telegram,
  webhook, or other live gateway route. For the preselected media sample, an
  admitted file path resolves under `/data/workspace/legacy-box1`; a
  cache-only media path is explicitly recorded as archive-only and is not
  presented as migrated.
- Austin's imported memories are visible. Legacy skills exist only under
  `/data/migration/legacy-hermes-v1/review-only/skills`; none shadow or merge
  into the active managed skill tree.
- The receipt and rebuilt memory store contain 80 structured facts.
- The receipt records 12 source cron jobs, and no job exists in the active
  target cron path; the review copy is under the migration receipt directory.
- Files exist under `/data/workspace/legacy-box1/{workspace,dev,uploads}` and a
  sample of manifested hashes and contained symlinks matches.
- The manifest and receipt contain no write target for the target's managed
  Hermes configuration, active skills, identity, or Chat state. Protected
  identity and Chat hashes still match step 6.
- Brain access is recorded as pending fresh target authorization and sync; no
  source Brain identity or working tree was copied.
- A new Finite Chat message receives exactly one target reply.
- box1 has zero Austin pods; its PVC and off-host archive remain intact.
- No other box1 or lat3 bot restarted or changed artifact.
- `scripts/finite-status --json` is retained and green.

Complete the receipt status in the retained operator evidence, not by editing
the receipt inside `/data`. Record IDs, digests, hashes, counts, timestamps,
and outcomes; record no token, key, message content, or secret value.

## Post-canary promotion

The data canary is complete without activating Austin's 12 scheduled jobs or
old external credentials. Restoring those behaviors is a later production
change, not a reason to weaken this import boundary.

After the observation window, follow
[the post-cutover repair brief](../../finitecomputer-v2/docs/legacy-hermes-post-cutover-repair.md).
Authorize and sync a fresh FiniteBrain working tree, repair stale absolute
paths, and compare each quarantined legacy skill against the managed target
baseline before promoting or recreating it one at a time.

Build a private disposition sheet for each review-only job: retire, recreate
paused, or replace. Review its schedule, delivery target, tool access, helper
script, and rewritten working directory. Reauthorize only the credentials
still required through the target's normal secret path; never copy the box1
`.env`, tokens, pairing state, or channel identity. Recreate jobs paused while
box1 remains frozen, then enable and verify one at a time under a separate
approval. Do not decommission box1 until every required skill, job, Brain
workspace, and external behavior has a recorded disposition.

## Rollback

Before target restart, leave box1 stopped. If install fails, retain the failed
bundle and receipt work directory for diagnosis, restore the scoped fresh
target archive to an empty `<TARGET_DATA>`, and re-check its original identity
and Chat hashes. Never repair a partial target by hand.

After target restart, stop the target through Core before any rollback. Restore
the fresh target archive if the v2 target itself must be reset. To resume
box1, first prove target compute is stopped, then scale only
`statefulset/austin-finite` back to one and verify the original bot. Never run
both writers.

Do not delete the target, source PVC, staging bundle, or either recovery
archive during rollback. Decommission and credential reauthorization are later
changes with separate approval.
