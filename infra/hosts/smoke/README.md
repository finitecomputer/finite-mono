# smoke — legacy fleet host

`ovh-vps-smoke` is `15.204.56.61`. It is not the production Brain service;
Brain runs on the app plane described in [infra](../../README.md).

This directory retains captured configuration. It is not evidence of current
host state, backup health, port exposure or credential validity. Use fresh
read-only inventory before any operation; preserve retained databases and
archives. Do not restart a historical Brain writer or reuse it as rollback.

Legacy fleet operations remain separate from the app-plane deploy. Unverified
host/credential follow-ups are tracked in [FIN-95](https://linear.app/finitecomputer/issue/FIN-95).
