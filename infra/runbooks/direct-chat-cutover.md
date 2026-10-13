# Direct web chat cutover

Moves web chat from the hosted device's server-held MLS client to each
agent's own Finite Chat room log.

**What changes:**
- The runtime image sets `FINITECHAT_CHAT_TRANSPORT=direct`. On the first
  start after an upgrade, the sidecar folds the agent's MLS device into
  `/data/agent/direct/room-log.sqlite3` once. It reads the MLS store and never
  writes it.
- The dashboard reaches the log through the hosted-Hermes ingress, for runtimes
  in `FC_DIRECT_CHAT_RUNTIME_IDS`. Each request is signed by the person's key,
  which the hosted device holds.

Planning, dates and cohorts live in
[FIN-182](https://linear.app/finitecomputer/issue/FIN-182). Every production
step needs explicit authorization. The build, artifact and upgrade mechanics
are [runtime-image.md](runtime-image.md); the lat2 deploy is
[deploy-core.md](deploy-core.md).

## Invariants

- **Each agent runs exactly one chat mode.** `GET /v1/app/info` on the sidecar
  (through the plugin) reports `transport: "direct"`. A pending fold, an older
  image, or a stale log refused after a rollback all leave the agent serving
  MLS chat. The dashboard then uses the hosted device for it.
- **The dashboard decides on the agent's own word only.** Core refusing a
  session, or an ingress 404, leaves chat unavailable. It never routes to MLS.
- **Unchanged and still required:** `finitechat-server`, the hosted device
  (people's keys, Brain and Sites identity, signing) and their backups.
  Attachments sent before the fold are still fetched and decrypted from the
  chat server on demand.

## Before the window

1. **Code.** Merge the reviewed PR and build the runtime image from `main` per
   [runtime-image.md](runtime-image.md). Record the
   `ghcr.io/finitecomputer/agent-runtime@sha256:…` digest.
2. **lat2 closure.** Prepare a reviewed one-line change that sets
   `FC_DIRECT_CHAT_RUNTIME_IDS = "*"` in `infra/nixos/modules/dashboard.nix`.
   Build the window's lat2 closure from that commit. It carries the dashboard
   image with the direct bridge, and the hosted device with
   `/v1/chat/direct-authorization` and `digest`.
3. **Host list.** Get devops sign-off on the current agent host list:
   ```sql
   SELECT r.source_host_id, count(*) FROM project_runtime_links l
   JOIN agent_runtimes r ON r.id = l.agent_runtime_id WHERE l.active GROUP BY 1;
   ```
   A host outside lat3/lat4/lat5 also needs the upgrade wrapper and
   finite-status host lists extended first.
4. **Hosted access applied for every active runtime.** Any row that fails this
   check has no direct path: enable it, or leave that runtime off the list.
   ```sql
   SELECT r.source_host_id, r.id, c.hosted_enabled, c.hosted_apply_status,
          c.hosted_generation = c.hosted_applied_generation AS current, c.revoked
   FROM project_runtime_links l JOIN agent_runtimes r ON r.id = l.agent_runtime_id
   LEFT JOIN runtime_core_credentials c ON c.agent_runtime_id = r.id
   WHERE l.active ORDER BY 1, 2;
   ```
   `scripts/finite-status` must show hosted ingress serving on each host.

## In the window

### 1. Status and backups

1. Run `scripts/finite-status --json` on lat2 and every agent host. Keep the
   receipts.
2. Back up lat2 per [postgres-backup-restore.md](postgres-backup-restore.md),
   start the coordinated snapshot (`finite-hosted-web-chat-snapshot.service`)
   and verify a new Borg archive
   ([hosted-web-chat-recovery.md](hosted-web-chat-recovery.md)).
3. Record the rollback targets: each host's `readlink -f /run/current-system`,
   the current dashboard digest, and the current promoted runtime artifact ID.

### 2. Deploy lat2

Deploy lat2 with the dashboard list set to `*`. Agents that have not been
upgraded report no finitechat plugin and stay on the hosted device.
1. Activate the lat2 closure. Verify the running executables, hosted device
   `/healthz`, and a dashboard sign-in.
2. On lat2, as root, record each person's view from a scratch copy of the
   snapshot. Never point it at the live state or the snapshot itself: an
   SQLite reader can add `-shm` files, and those would break the snapshot's
   manifest. The output is hashes only.
   ```sh
   latest=/data/recovery-snapshots/hosted-web-chat/latest
   scratch=$(mktemp -d) && chmod 0700 "$scratch"
   cp -a "$latest/hosted-device" "$scratch/"
   bin=$(systemctl cat finitechat-hosted-device.service \
     | sed -n 's/^ExecStart=\([^ ]*\).*/\1/p')
   "$bin" --data-root "$scratch/hosted-device" \
     --server-url https://chat.finite.computer digest > /root/hosted-digests.jsonl
   chmod 0600 /root/hosted-digests.jsonl && rm -rf "$scratch"
   ```
   The scratch copy holds people's keys; keep it root-only and delete it.
   - A message sent through the hosted device after the snapshot shows up in
     the audit as a `message_count` difference. If that happens, take a fresh
     snapshot and recompute.

### 3. Canary

1. **One agent per host.** On each host, register a scoped canary artifact for
   one agent (`--promoted false --canary-runtime-id`, because `--promoted`
   defaults to true) and upgrade it with
   `scripts/rollout-lat1-runtime-artifact --roll-project-id`.
2. **Read its fold from the host.** Do not exec into the guest.
   ```sh
   cat /data/finite-saas-runner/kata/<agent runtime id>/agent/direct/fold-status.json
   ```
   `origin` must be `fold` and `owner_account_id` must be the owner's account.
   `scripts/finite-status` on the host counts every Agent's record in its
   Direct chat fold section (amber for pending, unrecorded or unreadable).
   - `pending` means the agent kept serving MLS chat. The `error` field says
     why, for example a room behind the chat server. Fix it and restart the
     agent.
3. **Audit the fold:**
   ```sh
   scripts/direct_chat_fold_audit.py hosted-digests.jsonl fold-status-*.json
   ```
   Every room must be `intact`. A difference names the topic or chat.
4. **Check the canary owner's web chat:**
   - same topics, chats, order, archived chats and history;
   - a send gets a reply;
   - new chat, rename, archive and attachment both ways work;
   - the Connections status loads.

### 4. Fleet

1. Register the same digest under a new, promoted artifact ID. Move each
   host's canary onto it, then update the host pins
   (`FC_RUNNER_RUNTIME_ARTIFACT_ID`).
2. Per host, run `--roll-all --roll-canary-project-id <canary>` with the
   wrapper's prepare/execute plan. It is serial, about 2 minutes per agent.
3. After each host:
   - run `scripts/finite-status`; Direct chat fold must not be amber;
   - copy that host's `fold-status.json` files and run the audit;
   - for any room that is not intact, keep that agent upgraded but take it off
     the list by naming the other runtimes explicitly instead of `*`, then
     investigate.

### 5. After

Keep `finitechat-server` and the hosted device running. Run finite-status on
every host. Record the audit results and their counts in FIN-182; do not
record transcript content.

## Rollback

**Before people have chatted directly (canary, or a failed audit):**
- Upgrade the agent forward to the previous artifact with the same wrapper. Its
  MLS store is untouched, so the agent serves MLS chat and the dashboard uses
  the hosted device for it.
- Leave `direct/` in place. The next direct start refuses it, because MLS
  advanced past the fold, and keeps serving MLS chat. Move it aside
  (`direct.rolled-back-<time>`) before upgrading again; that start folds afresh.

**After people have chatted directly:**
- Their direct-period messages exist only in `direct/` and in Hermes' sessions.
  The MLS path cannot show them.
- Prefer rolling forward. If you must roll back, keep `direct/`, move it aside
  as above, and record which runtimes did so in FIN-182.

**Dashboard only:** set the list back to `""` and deploy. This alone is not a
rollback for an upgraded agent: it no longer reads MLS, so its chat would stop.
Roll the agent back first.

**lat2 closure:** follow [deploy-core.md](deploy-core.md).
