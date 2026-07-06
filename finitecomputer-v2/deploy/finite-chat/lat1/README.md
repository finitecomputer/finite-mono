# Finite Chat lat1 Deploy Config

This directory holds the v2-owned operator config shape for deploying the
hosted Finite Chat room server to the current lat1/Core host.

Copy `workspace.env.example` to `secrets/workspace.env` and fill it locally.
`secrets/` is ignored and must never be committed.

```sh
mkdir -p deploy/finite-chat/lat1/secrets
cp deploy/finite-chat/lat1/workspace.env.example \
  deploy/finite-chat/lat1/secrets/workspace.env
```

Deploy a pinned Finite Chat commit from the `finitecomputer-v2` repo root:

```sh
scripts/deploy_finitechat_server_lat1.sh \
  deploy/finite-chat/lat1 \
  <finitechat-full-sha>
```

The script builds `finitechat-server` on the host from
`https://github.com/finitecomputer/finitechat.git`, installs a systemd service,
and routes `chat.finite.computer` through the host k3s/Traefik edge.
