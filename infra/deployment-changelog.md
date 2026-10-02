# Deployment evidence

Read current deployment facts from these authorities. Git history retains past
rollout narratives; outstanding work belongs in Linear.

For production questions, check the checkout revision against current `main`
and the deployed revision before using its documentation. Read `scripts/finite-status`
on the app plane and relevant Runner hosts for current observations. An old
worktree or an `UNKNOWN` result from an operator laptop does not establish
production state.

## Where the facts live

| Surface | Source of truth | How to read it |
|---|---|---|
| Dashboard image | `infra/nixos/modules/dashboard.nix` (`image = …@sha256:…`) | `git log -- infra/nixos/modules/dashboard.nix`; on lat2 `podman inspect finite-saas-dashboard --format '{{.ImageDigest}}'` |
| Agent Runtime image for **new** launches | Core's promoted runtime-artifact record plus `FC_RUNNER_RUNTIME_ARTIFACT_ID` in `/etc/finite/runner.env` on each Kata host (lat3, lat4, lat5) | `scripts/finite-status` reports the pin per host; promotion per [`runbooks/runtime-image.md`](runbooks/runtime-image.md) |
| Agent Runtime image for **existing** Agents | Core's per-Runtime record — Agents pin at launch and never auto-update | `scripts/finite-status`; serial upgrades per [`runbooks/runtime-image.md`](runbooks/runtime-image.md) §4a |
| CLI releases (`finitechat`, `fsite`, `fbrain`) | component-scoped source tags in finite-mono and public rolling alias releases in `finitecomputer/finite-releases` (`finitechat-latest`, `fsite-latest`, `fbrain-latest`) | `gh release list --repo finitecomputer/finite-releases`; `git tag -l 'finitechat/*' 'fsite/*' 'fbrain/*'` |
| Server binaries on lat2 (Core, chat, Hosted Device, Brain, Identity) | the NixOS closure built from `infra/nixos/` at the deployed revision | `readlink -f /run/current-system` on the host; `scripts/finite-status` |
| Finite Sites | the live Fly Machine image digest and configuration; desired configuration in `infra/fly/sites/fly.toml` | Machine inspection and `scripts/finite-status --sites-backup-state` per [`runbooks/deploy-sites.md`](runbooks/deploy-sites.md) |
| Finite Private (Tinfoil) | [`tinfoil/model-inventory.md`](tinfoil/model-inventory.md) plus checked-in candidate configs under `infra/tinfoil/` | `just finite-private-deepseek-contract` |
| Phala canary Runtime | `FC_RUNNER_RUNTIME_ARTIFACT_ID` in `/etc/finite/phala-runner.env` | the worker credential environment supplies the pin; [`runbooks/phala-confidential-runner.md`](runbooks/phala-confidential-runner.md) |

## 2026-10-02 — Brain 0.6.0 release

Shipped [fbrain 0.6.0](https://github.com/finitecomputer/finite-releases/releases/tag/fbrain/v0.6.0)
for administrator Brain renaming, invited-Folder sync recovery and one-hour
invitation transit tolerance ([source PR #1037](https://github.com/finitecomputer/finite-mono/pull/1037)).
The initial approved existing-Agent rollout completed at 2026-10-02T10:59:44Z,
preserving four excluded Runtimes. The subsequently authorized BrainBot, Jules,
and Cornelius work completed at 2026-10-02T23:06:35Z. BrainBot and Jules passed
cold backup and off-host restoration before retained-identity restart; after
daemon resume restored Cornelius's provider control path, it passed the supported
single-Agent upgrade. All three retain their original Principals, data roots,
and every baseline history row and run `fbrain 0.6.0`.

Final Core observations show 86 of 87 active Kata Agents on the release image
and all 87 ready. Only the explicitly excluded “testing again” remains on its
earlier image with a revoked credential and pending upgrade control. All three
Runner hosts retain the release pin and are undrained. Live-stream chat and
prior-history preservation passed after the final upgrade; the earlier fresh
Zen Dashboard launch and first reply qualify this same published digest. All
eight public release assets were independently downloaded and checksum-verified.
Core's per-Runtime records remain the image authority; aggregate status still
includes the excluded Runtime and pre-existing host-health findings.

## Compatibility boundaries

Hosted Agents pin their Runtime image at launch and do not auto-update. Guarded
same-volume upgrades preserve the Agent Principal and `/data`.

Server compatibility includes fielded clients, persisted state, and supported
recovery sets. Record required reader/writer compatibility in the owning
contract and tests, rather than duplicating version tables here.

### fbrain 0.6.0

Deploy the Brain server before publishing the CLI or promoting a Runtime that
bundles it. The server accepts signed administrator `rename-brain` records and
atomically updates only the Brain display name with its audit record. Brain IDs,
principals, grants, encryption keys, Folder IDs, Working Tree paths and content
remain unchanged. Replaying an accepted record cannot restore an earlier name.
Existing clients can continue reading and syncing; the new CLI refreshes its
cached display name through metadata. Older non-admin clients may retain the
previous cached label until refresh/reopen. An older server rejects rename without
mutation and can reopen the same database after newer rename writes. Rolling
back the server removes rename support while retaining the latest stored name;
retain the previous closure and a consistent, verified SQLite backup.

Invitation creation keeps the requested expiry and the existing maximum lifetime;
the lower-bound tolerance admits one-hour invitations delayed in transit. Existing
clients benefit from the server change. The CLI repairs the known invited-Folder
cache divergence during ordinary sync and preserves unsynced local edits while
access is unavailable. It does not grant access, bypass revocation, or repair an
unrelated missing/corrupt key bootstrap. The fielded 0.5.0 client remains supported;
only an updated CLI receives the cache repair.

Hosted Agents keep their pinned Runtime until explicitly upgraded. An upgraded
Runtime supplies the new CLI and invitation guidance; existing managed skills
update with `finite skills sync`, and Working Tree guidance refreshes on a
successful open/sync. These changes do not provision Personal Brains or repair
Chat's Organization Brain requester lease.
