# Needs deployment

Status: **OPERATIONAL HANDOFF — NOT DEPLOYMENT AUTHORITY**

This queue records merged work that is not yet known to be released or
deployed. Merging, appearing here, or sharing a source revision does not
authorize a release, production deploy, artifact promotion, or Agent Runtime
rollout. Each mutation still needs Paul's fresh approval and its owning
runbook.

## Queue

| Work | Merged source | Surface | Required next action | Close only after | Status |
|---|---|---|---|---|---|
| Finite Sites item 1: truthful publishing, automatic viewing, and human sharing | PRs [#194](https://github.com/finitecomputer/finite-mono/pull/194), [#195](https://github.com/finitecomputer/finite-mono/pull/195), and [#196](https://github.com/finitecomputer/finite-mono/pull/196); `main` merge `a912cd5159c25c5fca9c61913c86a26a7c2525da` | `fsite` component release | Cut the next `fsite/vX.Y.Z` tag from an accepted `main` revision and verify the rolling alias per [release-cli.md](runbooks/release-cli.md). | The versioned release and `fsite-latest` serve matching verified assets, and a field install reports the new version. | **NEEDS RELEASE** |
| Finite Sites item 1: truthful publishing, automatic viewing, and human sharing | Same source set | Sites server (`finitesitesd` on lat1) | Prebuild and deploy an exact accepted `main` revision through [deploy-sites.md](runbooks/deploy-sites.md). | The production edge returns `Cache-Control: no-store` for real HTML and assets; an ordinary v1 → v2 publish/reload returns v2; Git push completion reflects reconciliation; invite email copy is human-first. | **NEEDS SITES DEPLOY** |
| Finite Sites item 1: authenticated requester inference | PR [#196](https://github.com/finitecomputer/finite-mono/pull/196), included in merge `a912cd5159c25c5fca9c61913c86a26a7c2525da` | Agent Runtime image and existing-Agent rollout | Build and publish one digest-pinned Agent Runtime from the accepted source revision, promote it, prove a disposable canary, then use the reviewed prepare/execute flow in [runtime-image.md](runbooks/runtime-image.md) for any named existing-Agent cohort. | The exact image digest is recorded; a cached second Finite turn initializes and shares every declared output with its authenticated requester; a non-Finite, expired, internal/background, restarted, or mismatched turn does not infer one; each upgraded Agent retains its Principal and writable `/data`. | **NEEDS AGENT ROLLOUT** |
| finite-lat-2 emergency cutover, stage 1: wipe + capture + chassis artifact (ADR 0007) | The replacement scaffold PR (app-plane host config, contracts, `Lat2 NixOS Closure` workflow, capture script, runbooks) once merged | lat2 host (still Ubuntu) | Owner go for the wipe; rescue-mode storage capture (`infra/nixos/scripts/capture-lat2-host-evidence`); review `storage-ids.nix` + networking into `infra/nixos/hosts/finite-lat-2/`; merge and build the `lat2-nixos-closure-REV` artifact at the captured rev. | Capture verified (2+2 NVMe shape fits pinned geometry), captured rev merged on `main`, CI artifact built. | **NEEDS WIPE + GATE B** |
| finite-lat-2 emergency cutover, stage 2: install + import state | Stage 1 closed | lat2 host | Gate C: artifact-driven nixos-anywhere install (import-mode boot) per [lat2-replacement-cutover.md](runbooks/lat2-replacement-cutover.md); Gate D: place the banked secret files, pg_restore the coordinated dump, litestream-restore chat + brain from `finite-lat-1-litestream`, place sites/hosted-device/identity/core trees; run every offline verify (87-key invariant, SQLite integrity, row counts). | `[UU]` arrays with `finite-storage-health` green, ESP guard intact, all offline verifications pass, no product unit ever started. | **NEEDS INSTALL + IMPORT** |
| finite-lat-2 emergency cutover, stage 3: go live | Stage 2 closed | lat2 host, lat3 WG peer, DNS | Gate E: go-live closure (`finite.importMode.enable = false`) via `just deploy-lat2-closure`; deploy the lat3 peer flip; owner flips Namecheap + Cloudflare DNS to 64.34.80.19; post-cutover cleanup (unregister old-lat2 GitHub runners, rotate legacy credentials). | Full loopback verify passes under the go-live closure; every product name serves from lat2; lat3 handshake current; `scripts/finite-status` green. | **NEEDS GO-LIVE** |

No dashboard source changed in these PRs, so this work needs **no dashboard
rollout**.

## Queue discipline

- Add only merged source. Record the PRs and exact merge revision.
- Split rows by independently deployable surface. A coupled NixOS switch may
  satisfy more than one row, but it does not imply an Agent Runtime rollout.
- Replace a status only with immutable evidence: release tag, image digest,
  deployed Git revision/system closure, and the owning runbook's verification.
- Once every row for a work item is closed, record anything the sources of
  truth cannot express in [`deployment-changelog.md`](deployment-changelog.md)
  or the relevant production baseline, and remove the item from this queue.
