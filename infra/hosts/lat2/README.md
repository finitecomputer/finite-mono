# finite-lat-2 — app plane

lat2 (`64.34.80.19`) runs Core, dashboard, Postgres, Chat, Hosted Web Device,
Brain, Identity, search, Caddy and backup/monitoring services. It runs no Agent
Runner. Sites runs on Fly.

The authoritative configuration is [the NixOS host](../../nixos/hosts/finite-lat-2/).
Use [Core deployment](../../runbooks/deploy-core.md), [secret inventory](../../nixos/README.md#secrets-bootstrap-checklist-values-never-in-this-repo),
and [recovery](../../runbooks/hosted-web-chat-recovery.md).

Other files in this directory capture retired services and are not deployment
authority. Unverified credential/runner cleanup is tracked in
[FIN-95](https://linear.app/finitecomputer/issue/FIN-95).
