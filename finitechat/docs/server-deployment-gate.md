# Finite Chat Server Deployment Gate

This is the server-side release gate for native app, TestFlight, and Friends
Alpha builds that use `https://chat.finite.computer`.

Finite Chat owns:

- the `finitechat-server` source and HTTP route contracts;
- the server build provenance exposed by `GET /health`;
- the compatibility decision for a finite-chat app/server pair;
- the release-blocking verification that production is running the expected
  server commit.

`../finitecomputer-v2` owns the hosted SaaS deploy mechanics: current lat1
systemd/k3s/Traefik rollout, future image/release artifacts, stack deploy
coordination, and hosted runtime health gates. The legacy `../finitecomputer`
repo remains for box1/TRF deployments only. This split does not make server
deployment optional for this repo. If an app change depends on server behavior,
stop and loop Paul into the v2 deploy lane before distributing the app.

## Required Production Check

Before any phone or TestFlight build is handed to testers:

```sh
export FINITECHAT_RELEASE_COMMIT="$(git rev-parse --short=12 HEAD)"
scripts/server-contract-gate.py \
  --server https://chat.finite.computer \
  --expected-source "$FINITECHAT_RELEASE_COMMIT"
```

The deployed health response must include:

```json
{
  "status": "ok",
  "server_contract_version": 3,
  "server_version": "0.1.0",
  "source_commit": "<finite-chat commit>",
  "source_dirty": false
}
```

The release is blocked when any of these are true:

- `/health` omits `source_commit` or `server_version`;
- `/health` omits `server_contract_version`;
- `server_contract_version` is not the exact contract version expected by the
  app, CLI, Hermes bridge, and runtime image being shipped;
- `source_commit` is not the finite-chat commit expected by the app build;
- `source_dirty` is `true`;
- a server-side route or DTO changed but production still reports an older
  compatible-looking build;
- the app requires a companion service change such as `push-drain`, blob
  storage policy, or Hermes bridge behavior that has not been deployed.

This deploy gate is intentionally stricter than normal client/server
interoperability. The gate proves production is running the exact finitechat
server build selected for a release. Runtime clients should treat
`server_contract_version` as a minimum server-visible transport/admission
contract: a newer server may be accepted when it still preserves the older
delivery behavior. Encrypted app-message protocol compatibility belongs to the
clients in the room, not to the server health check.

## Handoff To finitecomputer-v2

When production needs a server update, loop Paul into `../finitecomputer-v2`
with:

- finite-chat branch and full commit SHA to deploy;
- whether the deployment needs only `finitechat-server` or also a companion
  worker such as `push-drain`;
- the finite-chat checks already run locally;
- any server data/backfill/rollback notes;
- the expected post-deploy `/health` payload.

The current v2 deployment lane is documented in
`../finitecomputer-v2/docs/finite-stack-deployment.md` and currently uses:

```sh
(
  cd ../finitecomputer-v2
  scripts/deploy_finitechat_server_lat1.sh \
    deploy/finite-chat/lat1 \
    <finitechat-commit>
)
```

Treat the exact deploy command as owned by v2. The required finite-chat
acceptance criterion is that production `/health` reports the expected
finite-chat commit and the app-facing smoke tests pass against
`https://chat.finite.computer`.

## Post-Deploy Smoke

After Paul deploys the server, run:

```sh
cargo run -q -p finitechat-cli -- http --server https://chat.finite.computer health
scripts/server-contract-gate.py \
  --server https://chat.finite.computer \
  --expected-source "$(git rev-parse --short=12 HEAD)"
cargo test -p finitechat-server --test http_routes
cargo test -p finitechat-server --test http_persistence
```

For Friends Alpha, continue with `docs/friends-alpha-integration-runbook.md`.
For TestFlight, continue with `docs/testflight-runbook.md`.
