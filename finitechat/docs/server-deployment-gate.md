# Chat server deployment gate

Deploy through the [Chat runbook](../../infra/runbooks/deploy-finitechat-server.md)
on the app-plane host. The release gate proves the exact selected server
artifact is running; it does not substitute for existing-state compatibility
or client-to-client encrypted protocol tests.

From the clean monorepo revision used to build the deployed NixOS closure:

```sh
FINITECHAT_SOURCE_FINGERPRINT="$(nix eval --raw .#packages.x86_64-linux.finitechat-server.sourceFingerprint)"
finitechat/scripts/server-contract-gate.py \
  --server https://chat.finite.computer \
  --expected-fingerprint "$FINITECHAT_SOURCE_FINGERPRINT"
```

The health response must include a healthy status, server version, expected
server contract version, exact scoped Nix source fingerprint, and a clean
source flag. Package inputs include the toolchain and Nixpkgs, so their changes
can rotate the fingerprint without a Chat source change.

Normal clients use the server contract version as a minimum compatible
transport/admission contract. The deployment gate deliberately requires the
selected artifact. Preserve older fielded clients and durable-state readers;
a successful health check alone does not prove them compatible.
