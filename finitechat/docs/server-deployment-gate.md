# Finite Chat Server Deployment Gate

This is the server-side release gate for native app, TestFlight, and Friends
Alpha builds that use `https://chat.finite.computer`.

Finite Chat owns:

- the `finitechat-server` source and HTTP route contracts;
- the server build provenance exposed by `GET /health`;
- the compatibility decision for a finite-chat app/server pair;
- the release-blocking verification that production is running the expected
  server artifact.

`../finitecomputer-v2` owns the hosted SaaS deploy mechanics: current lat1
systemd/k3s/Traefik rollout, future image/release artifacts, stack deploy
coordination, and hosted runtime health gates. The legacy `../finitecomputer`
repo remains for box1/TRF deployments only. This split does not make server
deployment optional for this repo. If an app change depends on server behavior,
stop and loop Paul into the v2 deploy lane before distributing the app.

## Required Production Check

Before any phone or TestFlight build is handed to testers:

```sh
export FINITECHAT_SOURCE_FINGERPRINT="$(
  nix eval --raw .#packages.x86_64-linux.finitechat-server.sourceFingerprint
)"
finitechat/scripts/server-contract-gate.py \
  --server https://chat.finite.computer \
  --expected-fingerprint "$FINITECHAT_SOURCE_FINGERPRINT"
```

Run this from the root of the exact clean monorepo revision whose NixOS closure
was deployed. Nix derives the fingerprint from Chat's scoped package inputs;
there is no revision value to copy into a source file or release manifest.
The fingerprint covers the complete Nix artifact input, so an intentional
Nixpkgs or toolchain update may rotate it even when Chat source is unchanged.

The deployed health response must include:

```json
{
  "status": "ok",
  "server_contract_version": 6,
  "server_version": "0.1.0",
  "source_fingerprint": "nix-<scoped-source-hash>",
  "source_dirty": false
}
```

The release is blocked when any of these are true:

- `/health` omits `source_fingerprint` or `server_version`;
- `/health` omits `server_contract_version`;
- `server_contract_version` is not the exact contract version expected by the
  app, CLI, Hermes bridge, and runtime image being shipped;
- `source_fingerprint` is not the fingerprint of the selected Nix Chat
  package;
- `source_dirty` is `true`;
- a server-side route or DTO changed but production still reports an older
  compatible-looking build;
- the app requires a companion service change such as `push-drain`, blob
  storage policy, or Hermes bridge behavior that has not been deployed.

This deploy gate is intentionally stricter than normal client/server
interoperability. The gate proves production is running the exact finitechat
server build selected for a release. The NixOS rollout record separately
preserves the overall monorepo Git revision. Runtime clients should treat
`server_contract_version` as a minimum server-visible transport/admission
contract: a newer server may be accepted when it still preserves the older
delivery behavior. Encrypted app-message protocol compatibility belongs to the
clients in the room, not to the server health check.

Older clients that only display `source_commit` will show `unknown-commit`
against a fingerprint-only Nix server. That is cosmetic; protocol compatibility
continues to come from `server_contract_version`.

## Handoff To finitecomputer-v2

When production needs a server update, loop Paul into `../finitecomputer-v2`
with:

- full monorepo commit SHA to deploy;
- whether the deployment needs only `finitechat-server` or also a companion
  worker such as `push-drain`;
- the finite-chat checks already run locally;
- any server data/backfill/rollback notes;
- the Nix-derived expected Chat fingerprint and post-deploy `/health` payload.

The current deployment lane is documented in
`../../infra/runbooks/deploy-finitechat-server.md` and uses the reviewed
lat1 NixOS closure artifact path.

Treat the exact deploy command as owned by v2. The required finite-chat
acceptance criterion is that production `/health` reports the expected Chat
source fingerprint and the app-facing smoke tests pass against
`https://chat.finite.computer`.

## Post-Deploy Smoke

After Paul deploys the server, run:

```sh
cargo run -q -p finitechat-cli -- http --server https://chat.finite.computer health
finitechat/scripts/server-contract-gate.py \
  --server https://chat.finite.computer \
  --expected-fingerprint "$(
    nix eval --raw .#packages.x86_64-linux.finitechat-server.sourceFingerprint
  )"
cargo test -p finitechat-server --test http_routes
cargo test -p finitechat-server --test http_persistence
```

For Friends Alpha, continue with `docs/friends-alpha-integration-runbook.md`.
For TestFlight, continue with `docs/testflight-runbook.md`.
