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

Deploy a pinned Finite Chat commit (the script lives in
`infra/hosts/lat1/scripts/` and resolves this workspace relative to the
finite-mono root):

```sh
../infra/hosts/lat1/scripts/deploy-finitechat-server.sh \
  finitecomputer-v2/deploy/finite-chat/lat1 \
  <finitechat-full-sha>
```

The script builds `finitechat-server` on the host from
`https://github.com/finitecomputer/finitechat.git`, installs a systemd service,
and creates k3s Service/IngressRoute objects. NOTE: lat1 has no Traefik and no
Nix — see the script header and `infra/hosts/lat1/README.md`; the live chat
server runs on clawland today.
