# Hermes Runtime Test Matrix

Status: active v2 quality gate.

This document owns the product-shaped Hermes Agent Runtime ladder for
finitecomputer-v2. Finite Chat owns the encrypted protocol, native app, CLI, and
Hermes plugin. v2 owns proving that the real hosted-agent runtime shape works
before deploying it for users.

## Principle

Every rung must exercise the same product shape:

- deployed/default chat server is `https://chat.finite.computer` unless the run
  is explicitly validating an unmerged server branch;
- no phone localhost URLs;
- no PIN flow;
- the same Finite Chat Hermes plugin and CLI are packaged into the runtime;
- the same runtime image is used after the local source-level rung;
- destination differences are limited to provider config, durable volume
  binding, and public ingress;
- Hermes must be real, not an echo handler;
- the Finite Private model at every rung is `glm-5-2`, served behind the
  historical `https://kimi-k2-6.finite.containers.tinfoil.dev/v1` limiter URL
  (docs/service-dependencies.md, Finite Private Routing Debt);
- acceptance is a human-usable chat from the iOS app plus machine-readable
  evidence.

If a lower rung fails, do not climb. Fix the lowest failing contract first.

## Rung 1: Local Real-Hermes Adapter

Purpose: fast source-level proof that Finite Chat, the iOS app/simulator, and
Hermes can complete real turns.

Shape:

- `finitechat-server` may run locally for automated branch validation;
- iOS simulator can use an explicit local `FINITECHAT_SERVER_URL`;
- Hermes runs as a local process with the finitechat-owned plugin;
- model provider keys come from local operator env only.

This rung belongs mostly to `finitechat`. It proves the plugin/app/protocol
contract, not the hosted runtime image.

Acceptance:

- iOS simulator joins through the normal invite UI;
- user message reaches Hermes;
- Hermes produces at least two real model-backed replies;
- app shows pending/thinking/delivery states correctly;
- restart of the local Hermes service does not duplicate acknowledged messages.

## Rung 2: Runtime Image In Local Docker

Purpose: prove the real runtime image without a remote provider.

Operator command:

```bash
export FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY=<one operator-held deployed fpk_... key>
./scripts/local_create_agent_canary.sh
```

With `FC_LOCAL_FINITE_PRIVATE_UPSTREAM_KEY` set, the canary runs the in-tree
`finite-private-limiter` chained in front of the deployed limiter (via
`cargo run -p finite-saas-local -- finite-private-limiter-up`), and the local
rung exercises the real key path end to end:

- real for real: runner-driven provisioning through Core
  (`provision_finite_private_runtime_key`: auto-approved default-profile
  grant + runtime-scoped `fpk_...` key), agents launched without any key
  override, local admission/metering/burst+weekly limits enforced by local
  Core through the chained limiter, and key revocation on failed launch and
  on destroy — nothing on the key path is simulated;
- knowingly doubled: each inference call is metered twice — once by local
  Core (the runtime-scoped key) and once by prod Core (the single operator
  upstream key presented to the deployed limiter). The operator key's prod
  usage counters absorb all local canary traffic;
- the model is `glm-5-2` end to end. Agents inside Docker reach the chained
  limiter at `http://host.docker.internal:18002/v1` by default
  (`FC_LOCAL_CANARY_LIMITER_AGENT_BASE_URL` overrides it, e.g. with the
  docker bridge IP on Linux).

Fallback for operators without the local-limiter setup:
`FC_LOCAL_CANARY_FINITE_PRIVATE_API_KEY=<valid deployed fpk_... key>` makes the
runner inject that key directly into the agent, bypassing local provisioning
(keys minted by the throwaway local Core are not valid at the deployed
limiter, whose usage API is prod finite.computer).

Set `FC_LOCAL_AGENT_IMAGE=...` to test a newly published runtime image. Set
`FC_LOCAL_CANARY_KEEP_SERVICES=1` when you want to keep Core and the dashboard
running after the canary so a developer can inspect the local create-agent flow.
The runtime URL printed by this script is an API surface; open the printed
`/invite` endpoint, not the bare host root. The bare root is not the product
invite flow.
By default the canary builds `finitecomputer-v2-agent-runtime:local` from this
repo plus sibling `finitechat` and `finite-sites` checkouts. Set
`FC_LOCAL_CANARY_BUILD_RUNTIME_IMAGE=0` only when reusing an already-built local
or promoted image.
The local canary starts a throwaway Core; it requires one of the two Finite
Private credentials above by default.
`FC_LOCAL_CANARY_REQUIRE_FINITE_PRIVATE_KEY=0` is allowed only for a
launch-only check and must not be used before handing an invite to a human.
The canary must prove that the dashboard created a launchable Core request
before it invokes the runner. A dashboard redirect is not success by itself.

Shape:

- build the v2 runtime image;
- package Hermes Agent 0.18, `finitechat`, `fsite`, and the Finite Chat Hermes
  plugin;
- run it under Docker with the same mounted durable state paths expected by the
  hosted provider;
- point it at `https://chat.finite.computer` unless testing a server branch;
- expose the same invite/status API expected by the dashboard;
- do not add local-only flags that cannot exist in Phala.

Acceptance:

- image starts cleanly from empty state;
- invite endpoint returns a stable invite without a PIN;
- iOS app joins through the normal flow;
- user gets multiple real Hermes replies;
- `fsite` can publish a small public test site;
- Finite Private request succeeds through the limiter path intended for v2
  (a runtime-scoped key provisioned by local Core, `glm-5-2` reply through
  the chained limiter);
- container restart with the durable volume preserves identity, chat state, and
  Hermes memory.
- the image healthcheck reports healthy against the same `/healthz` surface the
  runner polls.

## Rung 3: Runtime Image In Remote Docker

Purpose: prove x86 Linux, remote networking, and operator reproducibility before
using a confidential runtime provider.

Shape:

- use the same promoted runtime image from Rung 2;
- run it on a remote x86 host such as finite-lat-1 or finite-lat-2;
- keep the same durable mount layout and env names;
- use the same invite/status API as local Docker;
- the iOS app still talks to `https://chat.finite.computer`.

Acceptance:

- fresh remote Docker deployment works first from the documented command;
- invite/status API is reachable to the operator/dashboard path;
- iOS app chats with real Hermes for multiple turns;
- remote restart preserves identity, chat state, and Hermes memory;
- `fsite` publish smoke passes;
- logs and health endpoints make failures classifiable without shelling into
  random process internals.

## Rung 4: Phala CVM

Purpose: prove the production confidential-runtime path with durable mounts.

Shape:

- deploy the same runtime image with a Phala compose/CVM config;
- mount the provider durable volume at the same path used in Docker;
- provide the same env contract as Docker;
- keep public ingress and invite/status behavior equivalent to Docker;
- use Phala attestation/debug surfaces for canary evidence.

Acceptance:

- one-off CVM deploy succeeds from the v2 runbook;
- invite/status API returns the expected agent identity/profile metadata;
- iOS app joins and completes multiple real Hermes turns;
- agent can save a memory, restart, and demonstrate that memory survived;
- agent can publish a small site with `fsite`;
- Finite Private request succeeds (runtime-scoped key provisioned by Core —
  no operator key override — against the deployed limiter serving `glm-5-2`);
- Phala restart does not require re-pairing the user;
- deployment evidence includes image ref, finitechat commit, Hermes version,
  plugin commit, Phala app id, and health output.

## Rung 5: Dashboard-Controlled SaaS Launch

Purpose: prove the user-facing self-serve flow.

Shape:

- user signs in with WorkOS;
- Core creates Project, Finite Private grant, runtime record, and launch request;
- `finite-saas-runner` leases the request with `FC_RUNNER_BACKEND=phala` and
  deploys the same promoted OCI runtime image used by the Docker rungs;
- dashboard displays invite/status from the same runtime API used in Docker and
  Phala canaries.
- dashboard controls are limited to restart and known-good chat runtime
  recovery; user work and configuration happen over Finite Chat or inside the
  runtime.

Acceptance:

- a new user can create one agent without operator shell access;
- dashboard shows the invite and current runtime health;
- native iOS app joins and chats;
- Core records the runtime provider id and deployed image/ref;
- restart/repair action is visible and works against the same runtime record;
- recovery preserves chat identity, Hermes memory, workspace state, and user
  files while moving aside only generated Hermes chat config;
- billing can be added later without changing the runtime test shape.

## Parked: Tinfoil Without Durable Mounts

Tinfoil can remain a later confidential-runtime target, but it is not the
default until durable state is solved without reintroducing a bespoke backup
control plane. Any future Tinfoil rung must pass the same Docker-equivalent
runtime contract before becoming a user-facing option.
